//! Safe, bounded, read-only access to the ledger files under one root.
//!
//! Every file is located with `lstat` (links and non-regular files refused),
//! opened without following a final link, and `fstat`-confirmed to be the
//! object located before any byte is read. The ledger's size is captured at
//! open and nothing past it is read; afterwards the open file and the path
//! are re-checked so truncation or replacement discards the snapshot.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{Interruption, SUPPORTED_SCHEMA_VERSION, SourceChange, Stop, Unavailable};

const LEDGER_DIR: &str = "ledger";
const SCHEMA_MARKER: &str = "schema_version";
const LEDGER_FILE: &str = "fires.jsonl";

/// The marker is one short integer line; anything longer is malformed.
const MAX_MARKER_BYTES: u64 = 16;

/// Bytes per read call, so cancellation is polled while capturing.
const CHUNK_BYTES: usize = 256 << 10;

/// Which path under the root a problem concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePart {
    Root,
    LedgerDir,
    SchemaMarker,
    Ledger,
}

/// What is wrong with a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathProblem {
    Missing,
    Symlink,
    NotDirectory,
    NotRegularFile,
    /// A regular file with more than one hard link; it may be shared with a
    /// location outside the root, so it is not read.
    MultipleLinks,
    /// The canonical path is not inside the canonical root.
    OutsideRoot,
    /// The object opened is not the object located (it changed in between).
    Changed,
    Unreadable(io::ErrorKind),
}

fn unavailable(part: SourcePart, problem: PathProblem) -> Stop {
    Stop::Unavailable(Unavailable::Path { part, problem })
}

fn io_problem(error: &io::Error) -> PathProblem {
    match error.kind() {
        io::ErrorKind::NotFound => PathProblem::Missing,
        kind => PathProblem::Unreadable(kind),
    }
}

/// Deadline and cancellation for one read.
pub(super) struct Budget<'a> {
    deadline: Instant,
    cancel: &'a dyn Fn() -> bool,
}

impl<'a> Budget<'a> {
    pub(super) fn new(deadline: Instant, cancel: &'a dyn Fn() -> bool) -> Self {
        Self { deadline, cancel }
    }

    pub(super) fn check(&self) -> Result<(), Stop> {
        if (self.cancel)() {
            return Err(Stop::Interrupted(Interruption::Cancelled));
        }
        if Instant::now() >= self.deadline {
            return Err(Stop::Interrupted(Interruption::DeadlineExceeded));
        }
        Ok(())
    }
}

/// Device and inode of an opened file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileIdentity {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
fn identity(metadata: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

#[cfg(unix)]
fn link_count(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink()
}

/// Only the Unix build is supported for the desktop app; elsewhere every
/// file compares equal and single-linked, which the path re-check still
/// guards partially.
#[cfg(not(unix))]
fn identity(_metadata: &Metadata) -> FileIdentity {
    FileIdentity { dev: 0, ino: 0 }
}

#[cfg(not(unix))]
fn link_count(_metadata: &Metadata) -> u64 {
    1
}

/// The canonical root. The root itself may be reached through links (a
/// linked `~/.config` is common); everything below it may not.
pub(super) fn resolve_root(root: &Path) -> Result<PathBuf, Stop> {
    if !root.is_absolute() {
        return Err(Stop::Unavailable(Unavailable::RootNotAbsolute));
    }
    let canonical =
        fs::canonicalize(root).map_err(|e| unavailable(SourcePart::Root, io_problem(&e)))?;
    let metadata =
        fs::metadata(&canonical).map_err(|e| unavailable(SourcePart::Root, io_problem(&e)))?;
    if !metadata.is_dir() {
        return Err(unavailable(SourcePart::Root, PathProblem::NotDirectory));
    }
    Ok(canonical)
}

/// `<root>/ledger`, which must be a real directory inside the root.
pub(super) fn ledger_dir(root: &Path) -> Result<PathBuf, Stop> {
    let part = SourcePart::LedgerDir;
    let path = root.join(LEDGER_DIR);
    let metadata = fs::symlink_metadata(&path).map_err(|e| unavailable(part, io_problem(&e)))?;
    if metadata.file_type().is_symlink() {
        return Err(unavailable(part, PathProblem::Symlink));
    }
    if !metadata.is_dir() {
        return Err(unavailable(part, PathProblem::NotDirectory));
    }
    let canonical = fs::canonicalize(&path).map_err(|e| unavailable(part, io_problem(&e)))?;
    if canonical.parent() != Some(root) {
        return Err(unavailable(part, PathProblem::OutsideRoot));
    }
    Ok(canonical)
}

/// A regular, single-linked, non-link file directly inside `dir`, opened and
/// confirmed to be the object located.
fn open_regular(dir: &Path, name: &str, part: SourcePart) -> Result<Opened, Stop> {
    let path = dir.join(name);
    let located = fs::symlink_metadata(&path).map_err(|e| unavailable(part, io_problem(&e)))?;
    check_regular(&located, part)?;
    let file = open_no_follow(&path).map_err(|problem| unavailable(part, problem))?;
    let opened = file
        .metadata()
        .map_err(|e| unavailable(part, io_problem(&e)))?;
    check_regular(&opened, part)?;
    if identity(&located) != identity(&opened) {
        return Err(unavailable(part, PathProblem::Changed));
    }
    let canonical = fs::canonicalize(&path).map_err(|e| unavailable(part, io_problem(&e)))?;
    if canonical.parent() != Some(dir) {
        return Err(unavailable(part, PathProblem::OutsideRoot));
    }
    Ok(Opened {
        path,
        identity: identity(&opened),
        len: opened.len(),
        file,
    })
}

fn check_regular(metadata: &Metadata, part: SourcePart) -> Result<(), Stop> {
    if metadata.file_type().is_symlink() {
        return Err(unavailable(part, PathProblem::Symlink));
    }
    if !metadata.is_file() {
        return Err(unavailable(part, PathProblem::NotRegularFile));
    }
    if link_count(metadata) > 1 {
        return Err(unavailable(part, PathProblem::MultipleLinks));
    }
    Ok(())
}

/// Open the final component with `O_NOFOLLOW | O_NONBLOCK`: a link swapped
/// in after `lstat` fails instead of being followed, and a FIFO or device
/// swapped in cannot block the open. Nonblocking does not affect reading a
/// regular file.
#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<File, PathProblem> {
    use std::os::unix::fs::OpenOptionsExt;
    File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) => PathProblem::Symlink,
            Some(libc::ENXIO) => PathProblem::NotRegularFile,
            _ => io_problem(&error),
        })
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> Result<File, PathProblem> {
    File::open(path).map_err(|error| io_problem(&error))
}

struct Opened {
    path: PathBuf,
    identity: FileIdentity,
    len: u64,
    file: File,
}

/// Read and validate `<ledger>/schema_version`: exactly the decimal
/// supported version, optionally followed by one newline.
pub(super) fn read_schema_marker(dir: &Path) -> Result<u32, Stop> {
    let part = SourcePart::SchemaMarker;
    let opened = open_regular(dir, SCHEMA_MARKER, part)?;
    if opened.len > MAX_MARKER_BYTES {
        return Err(Stop::Unavailable(Unavailable::SchemaMalformed));
    }
    let mut bytes = Vec::new();
    (&opened.file)
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| unavailable(part, io_problem(&e)))?;
    let digits = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let well_formed = !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
        && (digits.len() == 1 || digits[0] != b'0');
    let found = std::str::from_utf8(digits)
        .ok()
        .filter(|_| well_formed)
        .and_then(|text| text.parse::<u64>().ok())
        .ok_or(Stop::Unavailable(Unavailable::SchemaMalformed))?;
    if found != u64::from(SUPPORTED_SCHEMA_VERSION) {
        return Err(Stop::Unavailable(Unavailable::SchemaUnsupported { found }));
    }
    Ok(SUPPORTED_SCHEMA_VERSION)
}

/// The opened ledger file.
pub(super) struct Ledger(Opened);

pub(super) fn open_ledger(dir: &Path) -> Result<Ledger, Stop> {
    open_regular(dir, LEDGER_FILE, SourcePart::Ledger).map(Ledger)
}

/// Bytes captured from the end of the ledger.
pub(super) struct Captured {
    /// Ledger length at open; the captured bytes end here.
    pub(super) len: u64,
    /// Offset of the first captured byte.
    pub(super) start: u64,
    /// Whether `start` falls inside a line (the preceding byte is not `\n`).
    pub(super) starts_mid_line: bool,
    pub(super) bytes: Vec<u8>,
}

impl Ledger {
    pub(super) fn identity(&self) -> FileIdentity {
        self.0.identity
    }

    /// Read `[len - tail_bytes, len)` where `len` is the size at open.
    pub(super) fn capture(&self, tail_bytes: u64, budget: &Budget<'_>) -> Result<Captured, Stop> {
        let len = self.0.len;
        let start = len.saturating_sub(tail_bytes);
        let starts_mid_line = if start == 0 {
            false
        } else {
            let mut before = [0_u8; 1];
            self.read_exact_at(&mut before, start - 1, budget)?;
            before[0] != b'\n'
        };
        let size = usize::try_from(len - start).unwrap_or(usize::MAX);
        let mut bytes = vec![0_u8; size];
        self.read_exact_at(&mut bytes, start, budget)?;
        Ok(Captured {
            len,
            start,
            starts_mid_line,
            bytes,
        })
    }

    /// Fill `buf` from `offset`, polling the budget between chunks. Running
    /// out of bytes before the captured end means the file shrank.
    fn read_exact_at(&self, buf: &mut [u8], offset: u64, budget: &Budget<'_>) -> Result<(), Stop> {
        let mut done = 0;
        while done < buf.len() {
            budget.check()?;
            let end = buf.len().min(done + CHUNK_BYTES);
            let at = offset + u64::try_from(done).unwrap_or(u64::MAX);
            match read_at(&self.0.file, &mut buf[done..end], at) {
                Ok(0) => return Err(Stop::Changed(SourceChange::Shrunk)),
                Ok(n) => done += n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(unavailable(SourcePart::Ledger, io_problem(&error)));
                }
            }
        }
        Ok(())
    }

    /// After capture: the open file must not be shorter than the captured
    /// end, and the path must still name the same object. Returns whether the
    /// file grew (an append newer than the snapshot).
    pub(super) fn verify(&self, captured_len: u64) -> Result<bool, Stop> {
        let now = self
            .0
            .file
            .metadata()
            .map_err(|e| unavailable(SourcePart::Ledger, io_problem(&e)))?;
        if now.len() < captured_len {
            return Err(Stop::Changed(SourceChange::Shrunk));
        }
        match fs::symlink_metadata(&self.0.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(Stop::Changed(SourceChange::Removed))
            }
            Err(error) => Err(unavailable(SourcePart::Ledger, io_problem(&error))),
            Ok(at_path)
                if at_path.file_type().is_symlink() || identity(&at_path) != self.0.identity =>
            {
                Err(Stop::Changed(SourceChange::Replaced))
            }
            Ok(_) => Ok(now.len() > captured_len),
        }
    }
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}
