//! Generation-aware resume checkpoints for native sources.
//!
//! A checkpoint says how far a source was consumed *and which generation of
//! the source that was*. Input may be skipped only after the generation is
//! proven again:
//!
//! - A transcript file is identified by its device and inode, and the
//!   checkpoint carries two digests: one of every byte before its position and
//!   one of the last 4 KiB before it, plus the inode's change time. A scan
//!   resumes behind the position only if the same inode still holds at least
//!   that many bytes and the whole prefix digests to the recorded value. A
//!   file whose length and change time both still match is proven unchanged
//!   by the cheaper trailing digest alone (every write moves the change time,
//!   which no ordinary tool sets back); a file whose change time moved is
//!   proven by the whole prefix even when its length did not. A different
//!   inode is a replacement, a shorter file a truncation, a differing digest an
//!   in-place rewrite: each starts a new generation and the file is read from
//!   the beginning again.
//!   Rows never duplicate, because records dedupe by UUID; the cost of a new
//!   generation is a re-read, never a gap.
//! - A reader host (Codex, Cursor) is scanned through the pinned producer,
//!   which can skip sessions it saw modified before an instant (`--since`).
//!   The generation is the instant a scan that covered every session started,
//!   bound to the producer that ran it and to the inventory of sessions (path
//!   and update clock) that scan covered. A later scan by the same producer
//!   first inventories the host with a headers-only run; only if every session
//!   older than the cutoff is in the recorded inventory, unchanged, does it ask
//!   for everything modified since the instant, less a margin for coarse
//!   timestamps. A session restored or moved in with an old clock, a changed
//!   older session, an inventory that could not be taken, or a different
//!   producer (a moved pin) all mean a full scan. A scan that left any gap
//!   does not advance the generation.
//!
//! The zero-position rows of `source_cursors` stay plain locators. Only a
//! checkpoint authorizes skipping input, and it commits together with the rows
//! it covers, so a failed or partial batch can never advance past unimported
//! input.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    /// One regular file: the inode it lived in, a digest of every byte before
    /// the position, a digest of the last `tail_len` bytes before it, and the
    /// count of complete lines consumed so far (for reporting only).
    File {
        dev: u64,
        ino: u64,
        ctime_ns: i64,
        prefix_sha256: String,
        tail_len: u64,
        tail_sha256: String,
        lines: u64,
    },
    /// One host scan through the pinned reader: the instant it started, the
    /// producer (pinned commit and plugin version) that ran it, and the
    /// sessions it covered, each as `path@mtime`.
    HostScan {
        started_at_ms: i64,
        producer_commit: String,
        producer_version: String,
        inventory: Vec<String>,
    },
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
    /// The inode's change time: moved by every write, truncation, rename,
    /// link or permission change, and not settable by ordinary tools.
    pub ctime_ns: i64,
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
                ctime_ns: metadata
                    .ctime()
                    .saturating_mul(1_000_000_000)
                    .saturating_add(metadata.ctime_nsec()),
                len: metadata.len(),
                known: true,
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                dev: 0,
                ino: 0,
                ctime_ns: 0,
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

/// Where a scan may start, with the verified trailing bytes that seed its
/// digest window, the running digest of the verified prefix, and the line
/// count they represent.
pub struct Resume {
    pub start: u64,
    pub lines: u64,
    pub basis: ResumeBasis,
    pub tail: Vec<u8>,
    pub prefix: Sha256,
}

/// Bytes hashed per read while proving a prefix.
const PREFIX_CHUNK: usize = 64 * 1024;

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
        prefix: Sha256::new(),
    };
    let Some(checkpoint) = checkpoint else {
        return Ok(fresh(ResumeBasis::Fresh));
    };
    let Some(Generation::File {
        dev,
        ino,
        ctime_ns,
        prefix_sha256,
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
    if identity.len == position && identity.ctime_ns == ctime_ns {
        // Nothing was written to the inode since the checkpoint and nothing
        // would be read: the trailing bytes alone decide, cheaply.
        source.seek(SeekFrom::Start(position - tail_len))?;
        let mut tail = vec![0u8; usize::try_from(tail_len).unwrap_or(0)];
        source.read_exact(&mut tail)?;
        if hex_sha256(&tail) != tail_sha256 {
            return Ok(fresh(ResumeBasis::Rewritten));
        }
        return Ok(Resume {
            start: position,
            lines,
            basis: ResumeBasis::Unchanged,
            tail,
            prefix: Sha256::new(),
        });
    }
    // Bytes follow the position, or the inode was written to since the
    // checkpoint: every byte before the position is proven before any of what
    // follows is read, and the running digest continues over it.
    source.seek(SeekFrom::Start(0))?;
    let mut prefix = Sha256::new();
    let mut window = TailWindow::default();
    let mut remaining = position;
    let mut chunk = vec![0u8; PREFIX_CHUNK];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(PREFIX_CHUNK as u64)).unwrap_or(PREFIX_CHUNK);
        source.read_exact(&mut chunk[..wanted])?;
        prefix.update(&chunk[..wanted]);
        window.push(&chunk[..wanted]);
        remaining -= wanted as u64;
    }
    if hex_digest(prefix.clone()) != prefix_sha256 || window.len() != tail_len {
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
        tail: window.bytes,
        prefix,
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
    hex_digest(Sha256::new_with_prefix(bytes))
}

fn hex_digest(hasher: Sha256) -> String {
    let digest = hasher.finalize();
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

/// A file checkpoint through `position`, in the generation of `identity`;
/// `prefix` has digested every byte before `position`.
pub fn file_checkpoint(
    key: &str,
    identity: &FileIdentity,
    position: u64,
    prefix: &Sha256,
    window: &TailWindow,
    lines: u64,
    observed_at: i64,
) -> NativeCheckpoint {
    let generation = Generation::File {
        dev: identity.dev,
        ino: identity.ino,
        ctime_ns: identity.ctime_ns,
        prefix_sha256: hex_digest(prefix.clone()),
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

/// The last gapless scan of a host by one producer: when it started and what
/// it covered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostScan {
    pub started_at_ms: i64,
    pub inventory: BTreeSet<String>,
}

impl HostScan {
    /// The producer's cutoff for this generation, in the seconds its session
    /// clocks use: everything modified at or after it is read again.
    pub fn cutoff_secs(&self) -> f64 {
        self.started_at_ms.saturating_sub(HOST_SCAN_MARGIN_MS) as f64 / 1000.0
    }

    /// Whether asking only for sessions modified since the cutoff can skip
    /// nothing unknown: every session of the current source set that the
    /// cutoff would skip must be in the recorded inventory with the same
    /// clock. A restored or moved-in session with an old clock is not.
    pub fn cutoff_covers(&self, current: &BTreeMap<String, f64>) -> bool {
        let cutoff = self.cutoff_secs();
        current
            .iter()
            .filter(|(_, mtime)| **mtime < cutoff)
            .all(|(path, mtime)| self.inventory.contains(&inventory_key(path, *mtime)))
    }
}

/// One inventory entry: a session's path with its update clock.
pub fn inventory_key(path: &str, mtime: f64) -> String {
    format!("{path}@{mtime}")
}

/// The last gapless scan of `host` by this very producer, if any. A generation
/// left by another producer, or by an older shape without an inventory,
/// proves nothing for this one.
pub fn host_scan_generation(
    store: &Store,
    host: Host,
    home: &Path,
    producer_commit: &str,
) -> Option<HostScan> {
    let checkpoint = store
        .native_checkpoint(SessionSource::ReadersCli, &host_scan_key(host, home))
        .ok()??;
    match Generation::parse(&checkpoint.generation)? {
        Generation::HostScan {
            started_at_ms,
            producer_commit: recorded,
            inventory,
            ..
        } if recorded == producer_commit => Some(HostScan {
            started_at_ms,
            inventory: inventory.into_iter().collect(),
        }),
        _ => None,
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

pub fn host_scan_checkpoint(
    host: Host,
    home: &Path,
    started_at_ms: i64,
    producer_commit: &str,
    producer_version: &str,
    inventory: &BTreeMap<String, f64>,
) -> NativeCheckpoint {
    NativeCheckpoint {
        source: SessionSource::ReadersCli,
        cursor_key: host_scan_key(host, home),
        generation: serde_json::to_string(&Generation::HostScan {
            started_at_ms,
            producer_commit: producer_commit.to_owned(),
            producer_version: producer_version.to_owned(),
            inventory: inventory
                .iter()
                .map(|(path, mtime)| inventory_key(path, *mtime))
                .collect(),
        })
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
            ctime_ns: 5,
            prefix_sha256: "cd".into(),
            tail_len: 3,
            tail_sha256: "ab".into(),
            lines: 4,
        };
        let json = serde_json::to_string(&file).unwrap();
        assert_eq!(Generation::parse(&json), Some(file));
        assert_eq!(
            Generation::parse(
                r#"{"kind":"host_scan","started_at_ms":5,"producer_commit":"c","producer_version":"0.55.0","inventory":["p@1.5"]}"#
            ),
            Some(Generation::HostScan {
                started_at_ms: 5,
                producer_commit: "c".into(),
                producer_version: "0.55.0".into(),
                inventory: vec!["p@1.5".into()],
            })
        );
        // Older shapes without the producer or the inventory are not trusted.
        assert_eq!(
            Generation::parse(r#"{"kind":"host_scan","started_at_ms":5}"#),
            None
        );
        assert_eq!(
            Generation::parse(
                r#"{"kind":"host_scan","started_at_ms":5,"producer_commit":"c","producer_version":"v"}"#
            ),
            None
        );
        assert_eq!(Generation::parse(r#"{"kind":"future","x":1}"#), None);
        assert_eq!(Generation::parse("[]"), None);
    }

    #[cfg(unix)]
    #[test]
    fn resume_proves_inode_length_prefix_and_trailing_bytes_before_skipping() {
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
        let prefix = Sha256::new_with_prefix(b"first line\n");
        let key = file_key(&path);
        let checkpoint = file_checkpoint(&key, &id, 11, &prefix, &window, 1, 7);
        let resumed = resume_point(Some(&checkpoint), &mut source, &id).unwrap();
        assert_eq!(
            (resumed.start, resumed.lines, resumed.basis),
            (11, 1, ResumeBasis::Appended)
        );
        assert_eq!(resumed.tail, b"first line\n");
        assert_eq!(hex_digest(resumed.prefix), hex_sha256(b"first line\n"));
        // Nothing after the position: unchanged, by the trailing bytes alone.
        let whole_prefix = Sha256::new_with_prefix(b"first line\nsecond line\n");
        let whole = file_checkpoint(
            &key,
            &id,
            23,
            &whole_prefix,
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
        foreign.generation =
            r#"{"kind":"host_scan","started_at_ms":1,"producer_commit":"c","producer_version":"v","inventory":[]}"#
                .into();
        assert_eq!(
            resume_point(Some(&foreign), &mut source, &replaced)
                .unwrap()
                .basis,
            ResumeBasis::Rewritten
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_rewrite_before_the_trailing_window_is_caught_once_the_file_grows() {
        // A prefix far longer than the trailing window, checkpointed whole.
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("s.jsonl");
        let mut content = Vec::new();
        for index in 0..8 {
            content.extend_from_slice(format!("{index:0>2000}\n").as_bytes());
        }
        fs::write(&path, &content).unwrap();
        let source = fs::File::open(&path).unwrap();
        let id = identity(&source);
        let window = TailWindow::seeded(content.clone());
        let prefix = Sha256::new_with_prefix(&content);
        let checkpoint = file_checkpoint(
            &file_key(&path),
            &id,
            content.len() as u64,
            &prefix,
            &window,
            8,
            1,
        );
        // Change the first line only, keeping length and trailing bytes, and
        // append: the prefix digest disagrees, so the file is read whole.
        let mut edited = content.clone();
        edited[0] = b'9';
        edited.extend_from_slice(b"appended\n");
        fs::write(&path, &edited).unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let grown = identity(&source);
        assert_eq!(grown.ino, id.ino);
        assert_eq!(
            resume_point(Some(&checkpoint), &mut source, &grown)
                .unwrap()
                .basis,
            ResumeBasis::Rewritten
        );
        // The same bytes rewritten in place, length unchanged: the change time
        // moved, so the whole prefix is proven again and the file reads as
        // unchanged only because every byte still matches.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, &content).unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let rewritten_same = identity(&source);
        assert_ne!(rewritten_same.ctime_ns, id.ctime_ns);
        let same = resume_point(Some(&checkpoint), &mut source, &rewritten_same).unwrap();
        assert_eq!(same.basis, ResumeBasis::Unchanged);
        // The first byte rewritten in place, length and trailing bytes kept:
        // caught by the change time and the whole-prefix digest.
        let mut edited_same = content.clone();
        edited_same[0] = b'9';
        fs::write(&path, &edited_same).unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let edited = identity(&source);
        assert_eq!(
            resume_point(Some(&checkpoint), &mut source, &edited)
                .unwrap()
                .basis,
            ResumeBasis::Rewritten
        );
        // An untouched prefix with the same append resumes behind it.
        let mut appended = content.clone();
        appended.extend_from_slice(b"appended\n");
        fs::write(&path, &appended).unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let grown = identity(&source);
        let resumed = resume_point(Some(&checkpoint), &mut source, &grown).unwrap();
        assert_eq!(resumed.basis, ResumeBasis::Appended);
        assert_eq!(resumed.start, content.len() as u64);
        assert_eq!(resumed.tail.len() as u64, TAIL_DIGEST_LEN);
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
        let covered: BTreeMap<String, f64> = [("$HOME/.codex/sessions/a.jsonl".to_owned(), 1.5)]
            .into_iter()
            .collect();
        let checkpoint = host_scan_checkpoint(
            Host::Codex,
            Path::new("/home/x"),
            5,
            "abc",
            "0.55.0",
            &covered,
        );
        assert_eq!(checkpoint.cursor_key, "scan:codex:/home/x");
        assert_eq!(checkpoint.source, SessionSource::ReadersCli);
        assert_eq!(
            Generation::parse(&checkpoint.generation),
            Some(Generation::HostScan {
                started_at_ms: 5,
                producer_commit: "abc".into(),
                producer_version: "0.55.0".into(),
                inventory: vec!["$HOME/.codex/sessions/a.jsonl@1.5".into()],
            })
        );
        // The generation counts only for the producer that ran the scan.
        let mut store = Store::open_in_memory().unwrap();
        store.record_native_checkpoint(&checkpoint).unwrap();
        let generation =
            host_scan_generation(&store, Host::Codex, Path::new("/home/x"), "abc").unwrap();
        assert_eq!(generation.started_at_ms, 5);
        assert_eq!(
            host_scan_generation(&store, Host::Codex, Path::new("/home/x"), "moved"),
            None,
            "a moved pin starts with a full scan"
        );
        assert_eq!(
            host_scan_generation(&store, Host::Cursor, Path::new("/home/x"), "abc"),
            None
        );
    }

    #[test]
    fn a_cutoff_is_used_only_when_every_older_session_is_in_the_inventory() {
        let generation = HostScan {
            started_at_ms: 1_788_782_400_000,
            inventory: [inventory_key("old", 1_700_000_000.0)]
                .into_iter()
                .collect(),
        };
        let cutoff = generation.cutoff_secs();
        assert_eq!(cutoff, 1_788_782_398.0);
        let mut current: BTreeMap<String, f64> = BTreeMap::new();
        // Nothing older than the cutoff: safe.
        current.insert("new".into(), cutoff + 10.0);
        assert!(generation.cutoff_covers(&current));
        // An older session the scan covered, unchanged: safe.
        current.insert("old".into(), 1_700_000_000.0);
        assert!(generation.cutoff_covers(&current));
        // The same older session with another clock: not covered.
        current.insert("old".into(), 1_700_000_001.0);
        assert!(!generation.cutoff_covers(&current));
        current.insert("old".into(), 1_700_000_000.0);
        // An older session the scan never saw (restored, moved in): not covered.
        current.insert("restored".into(), 1_600_000_000.0);
        assert!(!generation.cutoff_covers(&current));
        // A session at the cutoff itself is read again anyway.
        current.remove("restored");
        current.insert("edge".into(), cutoff);
        assert!(generation.cutoff_covers(&current));
    }
}
