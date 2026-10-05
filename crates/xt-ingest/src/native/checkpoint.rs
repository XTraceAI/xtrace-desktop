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
//!   which no ordinary tool sets back), but only once the checkpoint was
//!   recorded a couple of seconds after that change time, so a coarse clock
//!   cannot hide a later write in the same tick; otherwise, and whenever the
//!   change time moved, the whole prefix is proven even at equal length. A different
//!   inode is a replacement, a shorter file a truncation, a differing digest an
//!   in-place rewrite: each starts a new generation and the file is read from
//!   the beginning again.
//!   Rows never duplicate, because records dedupe by UUID; the cost of a new
//!   generation is a re-read, never a gap.
//! - A reader host (Codex, Cursor) has no checkpoint: it is read whole through
//!   the pinned producer on every scan, and records dedupe by UUID.
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
use xt_store::{SessionSource, batch::NativeCheckpoint};

/// How many bytes ending at a file checkpoint are digested. A resume reads
/// this much back to prove the prefix it skips.
pub const TAIL_DIGEST_LEN: u64 = 4096;

/// A change time equal to the checkpoint's proves nothing by itself on a
/// filesystem whose clock is coarse: a rewrite in the same tick keeps it. It
/// counts only once the checkpoint was recorded this long after that change
/// time, so any later write must land in a later tick.
pub const CTIME_SETTLE_MS: i64 = 2_000;

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
    /// The file proved unchanged, but only by digesting its whole prefix
    /// because its change time moved (an identical rewrite, a permission
    /// change): the checkpoint should be refreshed with the current identity
    /// so the next proof is the cheap one again.
    pub refresh: bool,
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
    resume_point_checked(checkpoint, source, identity, &mut || Ok(()))
}

/// [`resume_point`], asking `check` before and after every read the proof
/// makes — the trailing bytes, or each chunk of the prefix — and ending with
/// the first error it returns. A caller that must stop cooperatively (a cancel,
/// a deadline) is heard between reads, never inside one.
pub fn resume_point_checked<E: From<io::Error>>(
    checkpoint: Option<&NativeCheckpoint>,
    source: &mut fs::File,
    identity: &FileIdentity,
    check: &mut dyn FnMut() -> Result<(), E>,
) -> Result<Resume, E> {
    let fresh = |basis| Resume {
        start: 0,
        lines: 0,
        basis,
        tail: Vec::new(),
        prefix: Sha256::new(),
        refresh: false,
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
    let settled = checkpoint
        .updated_at
        .saturating_sub(ctime_ns.saturating_div(1_000_000))
        >= CTIME_SETTLE_MS;
    if identity.len == position && identity.ctime_ns == ctime_ns && settled {
        // Nothing was written to the inode since the checkpoint (its change
        // time had settled well before the checkpoint was recorded, so even a
        // coarse clock would show a later write) and nothing would be read:
        // the trailing bytes alone decide, cheaply.
        check()?;
        source.seek(SeekFrom::Start(position - tail_len))?;
        let mut tail = vec![0u8; usize::try_from(tail_len).unwrap_or(0)];
        source.read_exact(&mut tail)?;
        check()?;
        if hex_sha256(&tail) != tail_sha256 {
            return Ok(fresh(ResumeBasis::Rewritten));
        }
        return Ok(Resume {
            start: position,
            lines,
            basis: ResumeBasis::Unchanged,
            tail,
            prefix: Sha256::new(),
            refresh: false,
        });
    }
    // Bytes follow the position, or the inode was (or may have been) written
    // to since the checkpoint: every byte before the position is proven before
    // any of what follows is read, and the running digest continues over it.
    source.seek(SeekFrom::Start(0))?;
    let mut prefix = Sha256::new();
    let mut window = TailWindow::default();
    let mut remaining = position;
    let mut chunk = vec![0u8; PREFIX_CHUNK];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(PREFIX_CHUNK as u64)).unwrap_or(PREFIX_CHUNK);
        check()?;
        source.read_exact(&mut chunk[..wanted])?;
        check()?;
        prefix.update(&chunk[..wanted]);
        window.push(&chunk[..wanted]);
        remaining -= wanted as u64;
    }
    if hex_digest(prefix.clone()) != prefix_sha256 || window.len() != tail_len {
        return Ok(fresh(ResumeBasis::Rewritten));
    }
    let unchanged = identity.len == position;
    Ok(Resume {
        start: position,
        lines,
        basis: if unchanged {
            ResumeBasis::Unchanged
        } else {
            ResumeBasis::Appended
        },
        tail: window.bytes,
        prefix,
        refresh: unchanged,
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
        // Other shapes, the retired host-scan generation among them, are not trusted.
        assert_eq!(
            Generation::parse(
                r#"{"kind":"host_scan","started_at_ms":5,"producer_commit":"c","producer_version":"v","inventory":[]}"#
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
        assert!(
            same.refresh,
            "proven by the whole prefix: refresh the checkpoint"
        );
        let refreshed = file_checkpoint(
            &file_key(&path),
            &rewritten_same,
            content.len() as u64,
            &same.prefix,
            &TailWindow::seeded(same.tail.clone()),
            8,
            2,
        );
        // Recorded in the same tick as the change time (a coarse clock could
        // hide a rewrite): still proven by the whole prefix.
        let mut source = fs::File::open(&path).unwrap();
        let unsettled = resume_point(Some(&refreshed), &mut source, &rewritten_same).unwrap();
        assert_eq!(unsettled.basis, ResumeBasis::Unchanged);
        assert!(unsettled.refresh, "an unsettled change time is no proof");
        // Recorded well after the change time: the cheap proof applies.
        let mut settled = refreshed.clone();
        settled.updated_at = rewritten_same.ctime_ns / 1_000_000 + CTIME_SETTLE_MS;
        let mut source = fs::File::open(&path).unwrap();
        let cheap = resume_point(Some(&settled), &mut source, &rewritten_same).unwrap();
        assert_eq!(cheap.basis, ResumeBasis::Unchanged);
        assert!(
            !cheap.refresh,
            "a settled checkpoint proves the file cheaply"
        );
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

    /// The checked proof asks before and after every read — each prefix chunk,
    /// or the trailing bytes — decides exactly as the plain one when never
    /// stopped, and reads nothing more once its check fails.
    #[cfg(unix)]
    #[test]
    fn a_checked_proof_is_heard_around_every_read_and_stops_when_told() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("s.jsonl");
        // Three whole prefix chunks and part of a fourth.
        let content = vec![b'x'; 3 * PREFIX_CHUNK + 100];
        fs::write(&path, &content).unwrap();
        let source = fs::File::open(&path).unwrap();
        let id = identity(&source);
        let checkpoint = file_checkpoint(
            &file_key(&path),
            &id,
            content.len() as u64,
            &Sha256::new_with_prefix(&content),
            &TailWindow::seeded(content.clone()),
            1,
            id.ctime_ns / 1_000_000 + CTIME_SETTLE_MS,
        );
        let mut grown = content.clone();
        grown.extend_from_slice(b"appended\n");
        fs::write(&path, &grown).unwrap();
        let mut source = fs::File::open(&path).unwrap();
        let grown = identity(&source);

        let mut asked = 0;
        let checked = resume_point_checked(Some(&checkpoint), &mut source, &grown, &mut || {
            asked += 1;
            Ok::<(), io::Error>(())
        })
        .unwrap();
        assert_eq!(asked, 2 * 4, "before and after each of four prefix reads");
        let plain = resume_point(Some(&checkpoint), &mut source, &grown).unwrap();
        assert_eq!(
            (checked.start, checked.basis, checked.tail, checked.refresh),
            (plain.start, plain.basis, plain.tail, plain.refresh)
        );
        assert_eq!(checked.basis, ResumeBasis::Appended);

        // Told to stop after the second chunk: two chunks read, no more.
        let mut asked = 0;
        let stopped = resume_point_checked(Some(&checkpoint), &mut source, &grown, &mut || {
            asked += 1;
            if asked == 4 {
                Err(io::Error::other("stop"))
            } else {
                Ok(())
            }
        });
        assert_eq!(
            stopped.err().map(|error| error.to_string()).as_deref(),
            Some("stop")
        );
        assert_eq!(source.stream_position().unwrap(), 2 * PREFIX_CHUNK as u64);

        // Unchanged and settled: the trailing bytes alone, heard around their
        // one read; told to stop first, nothing is read at all.
        let mut settled = file_checkpoint(
            &file_key(&path),
            &grown,
            grown.len,
            &Sha256::new(),
            &TailWindow::seeded(fs::read(&path).unwrap()),
            1,
            0,
        );
        settled.updated_at = grown.ctime_ns / 1_000_000 + CTIME_SETTLE_MS;
        let mut asked = 0;
        let cheap = resume_point_checked(Some(&settled), &mut source, &grown, &mut || {
            asked += 1;
            Ok::<(), io::Error>(())
        })
        .unwrap();
        assert_eq!(
            (cheap.basis, cheap.refresh, asked),
            (ResumeBasis::Unchanged, false, 2)
        );
        let mut source = fs::File::open(&path).unwrap();
        let refused = resume_point_checked(Some(&settled), &mut source, &grown, &mut || {
            Err(io::Error::other("stop"))
        });
        assert!(refused.is_err());
        assert_eq!(source.stream_position().unwrap(), 0);
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
}
