//! Generation-aware resume checkpoints for native sources.
//!
//! A checkpoint says how far a source was consumed *and which generation of
//! the source that was*. Input may be skipped only after the generation is
//! proven again:
//!
//! - A transcript file is identified by its device and inode, and the
//!   checkpoint carries a digest of the bytes that end at its position. A scan
//!   resumes behind the position only if the same inode still holds at least
//!   that many bytes and those trailing bytes digest to the same value. A
//!   different inode is a replacement, a shorter file a truncation, a differing
//!   digest an in-place rewrite: each starts a new generation and the file is
//!   read from the beginning again. Rows never duplicate, because records
//!   dedupe by UUID; the cost of a new generation is a re-read, never a gap.
//! - A reader host (Codex, Cursor) is scanned through the pinned producer,
//!   which can skip sessions it saw modified before an instant (`--since`).
//!   The generation is the instant a scan that covered every session started;
//!   a later scan asks for everything modified since then, less a margin for
//!   coarse timestamps. A scan that left any gap does not advance the instant.
//!
//! The zero-position rows of `source_cursors` stay plain locators. Only a
//! checkpoint authorizes skipping input, and it commits together with the rows
//! it covers, so a failed or partial batch can never advance past unimported
//! input.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};
use xt_store::{Host, SessionSource, Store, batch::NativeCheckpoint};

/// How many bytes ending at a file checkpoint are digested. A resume reads
/// this much back to prove the prefix it skips.
pub const TAIL_DIGEST_LEN: u64 = 4096;

/// Coarse source clocks and the producer's own update stamps may lag the scan
/// instant; a host scan asks this much further back than its generation.
pub const HOST_SCAN_MARGIN_MS: i64 = 2_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Generation {
    /// One regular file: the inode it lived in, a digest of the last
    /// `tail_len` bytes before the position, and the count of complete lines
    /// consumed so far (for reporting only).
    File {
        dev: u64,
        ino: u64,
        tail_len: u64,
        tail_sha256: String,
        lines: u64,
    },
    /// One host scan through the pinned reader: the instant it started.
    HostScan { started_at_ms: i64 },
}

impl Generation {
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}

/// The identity of an opened regular file, as far as the platform reveals it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    /// False where the platform cannot name the inode; such a file is never
    /// resumed, only read whole.
    pub known: bool,
}

impl FileIdentity {
    pub fn of(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                dev: metadata.dev(),
                ino: metadata.ino(),
                len: metadata.len(),
                known: true,
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                dev: 0,
                ino: 0,
                len: metadata.len(),
                known: false,
            }
        }
    }
}

/// Why a scan starts where it starts; reported, never acted on blindly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeBasis {
    /// No checkpoint: the whole file is read.
    Fresh,
    /// The checkpoint's generation holds and nothing follows the position.
    Unchanged,
    /// The checkpoint's generation holds and bytes follow the position.
    Appended,
    /// Another inode now lives at the path: a new generation, read whole.
    Replaced,
    /// The same inode is shorter than the position: read whole.
    Truncated,
    /// The same inode holds the position but the bytes before it changed: read whole.
    Rewritten,
    /// The platform cannot identify the inode: read whole.
    Unidentified,
}

/// Where a scan may start, with the verified bytes that seed its digest window
/// and the line count they represent.
#[derive(Debug)]
pub struct Resume {
    pub start: u64,
    pub lines: u64,
    pub basis: ResumeBasis,
    pub tail: Vec<u8>,
}

/// Decide the resume point for `source`, whose identity was just observed.
/// Reads at most `TAIL_DIGEST_LEN` bytes; leaves the file position unspecified.
pub fn resume_point(
    checkpoint: Option<&NativeCheckpoint>,
    source: &mut fs::File,
    identity: &FileIdentity,
) -> io::Result<Resume> {
    let fresh = |basis| Resume {
        start: 0,
        lines: 0,
        basis,
        tail: Vec::new(),
    };
    let Some(checkpoint) = checkpoint else {
        return Ok(fresh(ResumeBasis::Fresh));
    };
    let Some(Generation::File {
        dev,
        ino,
        tail_len,
        tail_sha256,
        lines,
    }) = Generation::parse(&checkpoint.generation)
    else {
        // A generation this build does not understand proves nothing.
        return Ok(fresh(ResumeBasis::Rewritten));
    };
    if !identity.known {
        return Ok(fresh(ResumeBasis::Unidentified));
    }
    if dev != identity.dev || ino != identity.ino {
        return Ok(fresh(ResumeBasis::Replaced));
    }
    let position = u64::try_from(checkpoint.position).unwrap_or(0);
    if identity.len < position || tail_len > position || tail_len > TAIL_DIGEST_LEN {
        return Ok(fresh(ResumeBasis::Truncated));
    }
    source.seek(SeekFrom::Start(position - tail_len))?;
    let mut tail = vec![0u8; usize::try_from(tail_len).unwrap_or(0)];
    source.read_exact(&mut tail)?;
    if hex_sha256(&tail) != tail_sha256 {
        return Ok(fresh(ResumeBasis::Rewritten));
    }
    Ok(Resume {
        start: position,
        lines,
        basis: if identity.len == position {
            ResumeBasis::Unchanged
        } else {
            ResumeBasis::Appended
        },
        tail,
    })
}

/// The last `TAIL_DIGEST_LEN` bytes of the complete lines consumed so far.
#[derive(Clone, Debug, Default)]
pub struct TailWindow {
    bytes: Vec<u8>,
}

impl TailWindow {
    pub fn seeded(tail: Vec<u8>) -> Self {
        let mut window = Self { bytes: tail };
        window.trim();
        window
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
        self.trim();
    }

    fn trim(&mut self) {
        let limit = usize::try_from(TAIL_DIGEST_LEN).unwrap_or(usize::MAX);
        if self.bytes.len() > limit {
            let excess = self.bytes.len() - limit;
            self.bytes.drain(..excess);
        }
    }

    pub fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn digest(&self) -> String {
        hex_sha256(&self.bytes)
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// The checkpoint key of a transcript file: the same key as its locator.
pub fn file_key(path: &Path) -> String {
    format!("claude:{}", path.display())
}

/// A file checkpoint through `position`, in the generation of `identity`.
pub fn file_checkpoint(
    key: &str,
    identity: &FileIdentity,
    position: u64,
    window: &TailWindow,
    lines: u64,
    observed_at: i64,
) -> NativeCheckpoint {
    let generation = Generation::File {
        dev: identity.dev,
        ino: identity.ino,
        tail_len: window.len(),
        tail_sha256: window.digest(),
        lines,
    };
    NativeCheckpoint {
        source: SessionSource::Transcript,
        cursor_key: key.to_owned(),
        generation: serde_json::to_string(&generation).expect("generation serializes"),
        position: i64::try_from(position).unwrap_or(i64::MAX),
        updated_at: observed_at,
    }
}

/// The checkpoint key of one host's reader scan under `home`.
pub fn host_scan_key(host: Host, home: &Path) -> String {
    format!("scan:{}:{}", host.as_str(), home.display())
}

/// The instant the last gapless scan of `host` started, if any.
pub fn host_scan_generation(store: &Store, host: Host, home: &Path) -> Option<i64> {
    let checkpoint = store
        .native_checkpoint(SessionSource::ReadersCli, &host_scan_key(host, home))
        .ok()??;
    match Generation::parse(&checkpoint.generation)? {
        Generation::HostScan { started_at_ms } => Some(started_at_ms),
        Generation::File { .. } => None,
    }
}

/// The `--since` instant for the next scan: the generation less the margin,
/// as RFC 3339 with an explicit UTC offset, which the producer requires.
pub fn host_scan_since(started_at_ms: i64) -> Option<String> {
    let instant = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        started_at_ms.saturating_sub(HOST_SCAN_MARGIN_MS),
    )?;
    Some(instant.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

pub fn host_scan_checkpoint(host: Host, home: &Path, started_at_ms: i64) -> NativeCheckpoint {
    NativeCheckpoint {
        source: SessionSource::ReadersCli,
        cursor_key: host_scan_key(host, home),
        generation: serde_json::to_string(&Generation::HostScan { started_at_ms })
            .expect("generation serializes"),
        position: 0,
        updated_at: started_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn identity(file: &fs::File) -> FileIdentity {
        FileIdentity::of(&file.metadata().unwrap())
    }

    #[test]
    fn generations_round_trip_and_unknown_shapes_prove_nothing() {
        let file = Generation::File {
            dev: 1,
            ino: 2,
            tail_len: 3,
            tail_sha256: "ab".into(),
            lines: 4,
        };
        let json = serde_json::to_string(&file).unwrap();
        assert_eq!(Generation::parse(&json), Some(file));
        assert_eq!(
            Generation::parse(r#"{"kind":"host_scan","started_at_ms":5}"#),
            Some(Generation::HostScan { started_at_ms: 5 })
        );
        assert_eq!(Generation::parse(r#"{"kind":"future","x":1}"#), None);
        assert_eq!(Generation::parse("[]"), None);
    }

    #[cfg(unix)]
    #[test]
    fn resume_proves_inode_length_and_trailing_bytes_before_skipping() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("s.jsonl");
        fs::write(&path, b"first line\nsecond line\n").unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let id = identity(&source);
        let fresh = resume_point(None, &mut source, &id).unwrap();
        assert_eq!((fresh.start, fresh.basis), (0, ResumeBasis::Fresh));
        // Checkpoint through the first line.
        let mut window = TailWindow::default();
        window.push(b"first line\n");
        let key = file_key(&path);
        let checkpoint = file_checkpoint(&key, &id, 11, &window, 1, 7);
        let resumed = resume_point(Some(&checkpoint), &mut source, &id).unwrap();
        assert_eq!(
            (resumed.start, resumed.lines, resumed.basis),
            (11, 1, ResumeBasis::Appended)
        );
        assert_eq!(resumed.tail, b"first line\n");
        // Nothing after the position: unchanged.
        let whole = file_checkpoint(
            &key,
            &id,
            23,
            &TailWindow::seeded(b"first line\nsecond line\n".to_vec()),
            2,
            7,
        );
        assert_eq!(
            resume_point(Some(&whole), &mut source, &id).unwrap().basis,
            ResumeBasis::Unchanged
        );
        // Shorter than the position: truncated.
        fs::write(&path, b"first\n").unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let shorter = identity(&source);
        assert_eq!(
            resume_point(Some(&checkpoint), &mut source, &shorter)
                .unwrap()
                .basis,
            ResumeBasis::Truncated
        );
        // Same length, different bytes before the position: rewritten.
        fs::write(&path, b"FIRST LINE\nsecond line\n").unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let rewritten = identity(&source);
        assert_eq!(
            resume_point(Some(&checkpoint), &mut source, &rewritten)
                .unwrap()
                .basis,
            ResumeBasis::Rewritten
        );
        // Another inode at the path: replaced.
        let staging = temp.path().join("staging");
        fs::write(&staging, b"first line\nsecond line\n").unwrap();
        fs::rename(&staging, &path).unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let replaced = identity(&source);
        assert_ne!(replaced.ino, id.ino);
        assert_eq!(
            resume_point(Some(&checkpoint), &mut source, &replaced)
                .unwrap()
                .basis,
            ResumeBasis::Replaced
        );
        // A generation this build does not understand proves nothing.
        let mut foreign = checkpoint.clone();
        foreign.generation = r#"{"kind":"host_scan","started_at_ms":1}"#.into();
        assert_eq!(
            resume_point(Some(&foreign), &mut source, &replaced)
                .unwrap()
                .basis,
            ResumeBasis::Rewritten
        );
    }

    #[test]
    fn the_tail_window_keeps_only_the_last_bytes() {
        let mut window = TailWindow::default();
        assert!(window.is_empty());
        let mut writer = Vec::new();
        for index in 0..3 {
            let line = format!("{index:0>2000}\n");
            writer.write_all(line.as_bytes()).unwrap();
            window.push(line.as_bytes());
        }
        assert_eq!(window.len(), TAIL_DIGEST_LEN);
        let expected = &writer[writer.len() - TAIL_DIGEST_LEN as usize..];
        assert_eq!(window.digest(), hex_sha256(expected));
        assert_eq!(hex_sha256(b"").len(), 64);
    }

    #[test]
    fn host_scans_ask_for_everything_since_the_generation_less_a_margin() {
        assert_eq!(
            host_scan_since(1_788_782_400_000).as_deref(),
            Some("2026-09-07T11:59:58.000Z")
        );
        let checkpoint = host_scan_checkpoint(Host::Codex, Path::new("/home/x"), 5);
        assert_eq!(checkpoint.cursor_key, "scan:codex:/home/x");
        assert_eq!(checkpoint.source, SessionSource::ReadersCli);
        assert_eq!(
            Generation::parse(&checkpoint.generation),
            Some(Generation::HostScan { started_at_ms: 5 })
        );
    }
}
