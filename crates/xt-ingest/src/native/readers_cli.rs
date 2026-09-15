//! Running the pinned shared readers. The producer is exactly the sources
//! named by `.plugin-pin`: either a developer checkout, whose HEAD and every
//! listed reader source object are verified through Git, or the copy bundled
//! with the app, whose files are verified by computing the same Git object
//! identities without Git. The readers see a disposable-looking environment
//! rooted at the requested home, no inherited interpreter overrides, and their
//! stdout is the shared stream; their stderr carries static diagnostic codes.
//! A running reader can be cancelled: its process is killed and reaped, and
//! what it had streamed stays committed.

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use xt_store::Host;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub repository: String,
    pub commit: String,
    pub plugin_root: String,
    pub plugin_version: String,
    pub reader_sources: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct PinnedProducer {
    pub commit: String,
    pub plugin_version: String,
    pub script: PathBuf,
    // Keep a checkout's immutable-source export alive through reader
    // execution; a bundle runs in place.
    _snapshot: Option<Arc<tempfile::TempDir>>,
}

/// Where the pinned producer's sources are.
#[derive(Clone, Debug)]
pub enum ProducerSource {
    /// A developer checkout at the pinned commit, verified through Git; the
    /// readers run from a temporary export of the committed objects.
    Checkout {
        pin: PathBuf,
        plugin_root: Option<PathBuf>,
    },
    /// A copy of the pinned sources laid out as in the producer repository
    /// (the app's bundled readers), verified by object identity without Git;
    /// the readers run in place.
    Bundle { pin: Pin, root: PathBuf },
}

impl ProducerSource {
    /// Verify the sources against the pin and locate the reader script.
    pub fn producer(&self) -> Result<PinnedProducer, ReaderError> {
        match self {
            Self::Checkout { pin, plugin_root } => {
                let root = plugin_root.as_deref().ok_or_else(|| {
                    ReaderError::PinMismatch("no pinned plugin root was supplied".into())
                })?;
                verify_pin(&read_pin(pin)?, root)
            }
            Self::Bundle { pin, root } => verify_bundle(pin, root),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReaderDiagnostic {
    pub code: String,
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderError {
    MissingRuntime(String),
    PinMismatch(String),
    Failed(String),
    /// The caller cancelled the reader: it was not started, or it was killed
    /// and reaped. What it had streamed before is not in doubt.
    Cancelled,
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRuntime(reason) => write!(f, "python runtime unavailable: {reason}"),
            Self::PinMismatch(reason) => write!(f, "pinned producer unavailable: {reason}"),
            Self::Failed(reason) => write!(f, "reader failed: {reason}"),
            Self::Cancelled => write!(f, "reader cancelled"),
        }
    }
}
impl std::error::Error for ReaderError {}

fn hex40(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn read_pin(path: &Path) -> Result<Pin, ReaderError> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| ReaderError::PinMismatch(".plugin-pin is unreadable".into()))?;
    parse_pin(&text)
}

/// Parse the pin file's text (the app compiles the pin in).
pub fn parse_pin(text: &str) -> Result<Pin, ReaderError> {
    let pin: Pin = serde_json::from_str(text)
        .map_err(|_| ReaderError::PinMismatch(".plugin-pin is not the expected JSON".into()))?;
    if !hex40(&pin.commit)
        || pin.reader_sources.is_empty()
        || pin.reader_sources.values().any(|id| !hex40(id))
    {
        return Err(ReaderError::PinMismatch(
            ".plugin-pin must name a full commit and reader source objects".into(),
        ));
    }
    Ok(pin)
}

fn git(repository: &Path, args: &[&str]) -> Result<String, ReaderError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|_| ReaderError::PinMismatch("git is unavailable to verify the pin".into()))?;
    if !output.status.success() {
        return Err(ReaderError::PinMismatch(format!(
            "git {} failed in the plugin checkout",
            args.first().copied().unwrap_or("")
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Resolve the pinned reader script under `plugin_root`, which must be the pin's
/// plugin root inside a clean checkout at the pinned commit with identical
/// reader source objects.
pub fn verify_pin(pin: &Pin, plugin_root: &Path) -> Result<PinnedProducer, ReaderError> {
    let root = plugin_root
        .canonicalize()
        .map_err(|_| ReaderError::PinMismatch("plugin root does not exist".into()))?;
    let toplevel = PathBuf::from(git(&root, &["rev-parse", "--show-toplevel"])?);
    let toplevel = toplevel.canonicalize().unwrap_or(toplevel);
    let relative = root
        .strip_prefix(&toplevel)
        .map_err(|_| ReaderError::PinMismatch("plugin root is outside its checkout".into()))?;
    if relative.to_string_lossy().replace('\\', "/") != pin.plugin_root {
        return Err(ReaderError::PinMismatch(
            "plugin root is not the pinned plugin_root".into(),
        ));
    }
    if git(&root, &["rev-parse", "HEAD"])? != pin.commit {
        return Err(ReaderError::PinMismatch(
            "plugin checkout is not at the pinned commit".into(),
        ));
    }
    for (path, expected) in &pin.reader_sources {
        let actual =
            git(&root, &["rev-parse", "--verify", &format!("HEAD:{path}")]).map_err(|_| {
                ReaderError::PinMismatch(format!("pinned reader source is absent: {path}"))
            })?;
        if &actual != expected {
            return Err(ReaderError::PinMismatch(format!(
                "pinned reader source differs: {path}"
            )));
        }
    }
    if !git(&root, &["status", "--porcelain"])?.is_empty() {
        return Err(ReaderError::PinMismatch(
            "plugin checkout has local modifications".into(),
        ));
    }
    // Execute only committed objects, never ignored working-tree modules/caches.
    let snapshot = tempfile::TempDir::new()
        .map_err(|_| ReaderError::PinMismatch("cannot create reader snapshot".into()))?;
    let archive = Command::new("git")
        .arg("-C")
        .arg(&toplevel)
        .args(["archive", &pin.commit, &pin.plugin_root])
        .output()
        .map_err(|_| ReaderError::PinMismatch("cannot export pinned reader".into()))?;
    if !archive.status.success() {
        return Err(ReaderError::PinMismatch(
            "cannot export pinned reader".into(),
        ));
    }
    let mut unpack = Command::new("tar")
        .args(["-xf", "-", "-C"])
        .arg(snapshot.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ReaderError::PinMismatch("cannot unpack pinned reader".into()))?;
    use std::io::Write;
    let written = unpack.stdin.take().unwrap().write_all(&archive.stdout);
    let status = unpack.wait();
    if written.is_err() || !status.is_ok_and(|status| status.success()) {
        return Err(ReaderError::PinMismatch(
            "cannot unpack pinned reader".into(),
        ));
    }
    let script = snapshot
        .path()
        .join(&pin.plugin_root)
        .join("scripts/readers_cli.py");
    if !script.is_file() {
        return Err(ReaderError::PinMismatch("readers_cli.py is absent".into()));
    }
    Ok(PinnedProducer {
        commit: pin.commit.clone(),
        plugin_version: pin.plugin_version.clone(),
        script,
        _snapshot: Some(Arc::new(snapshot)),
    })
}

/// Resolve the pinned reader script inside a bundled copy of the producer's
/// sources at `root`, laid out as in the producer repository. Every reader
/// source the pin lists must be present with exactly the pinned Git object
/// identity, computed here the way Git computes it (a blob's bytes, a tree's
/// sorted entries with their modes), so a bundle that was edited, that lost a
/// module or gained one, or that carries another producer's files is refused
/// without Git being installed. The pin must list the scripts tree itself,
/// since the readers import sibling modules from it.
pub fn verify_bundle(pin: &Pin, root: &Path) -> Result<PinnedProducer, ReaderError> {
    let scripts = format!("{}/scripts", pin.plugin_root);
    if !pin.reader_sources.contains_key(&scripts) {
        return Err(ReaderError::PinMismatch(
            "the pin does not name the bundled scripts tree".into(),
        ));
    }
    for (path, expected) in &pin.reader_sources {
        let actual = git_object_id(&root.join(path)).map_err(|reason| {
            ReaderError::PinMismatch(format!("bundled reader source {path}: {reason}"))
        })?;
        if &actual != expected {
            return Err(ReaderError::PinMismatch(format!(
                "bundled reader source differs: {path}"
            )));
        }
    }
    let script = root.join(&scripts).join("readers_cli.py");
    if !script.is_file() {
        return Err(ReaderError::PinMismatch("readers_cli.py is absent".into()));
    }
    Ok(PinnedProducer {
        commit: pin.commit.clone(),
        plugin_version: pin.plugin_version.clone(),
        script,
        _snapshot: None,
    })
}

/// The Git object identity of a regular file (blob) or directory (tree) on
/// disk, as `git hash-object` and `git write-tree` would compute it: the
/// lowercase hex SHA-1 of the object header and its content. A directory's
/// entries are sorted as Git sorts them (a subdirectory as if its name ended
/// in `/`), regular files carry mode 100644 or 100755 by their executable
/// bit, and Python bytecode caches are skipped, since Git never holds them.
/// A symlink or any other entry kind is refused.
pub fn git_object_id(path: &Path) -> Result<String, String> {
    fn hex(hash: &[u8]) -> String {
        hash.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    fn object(kind: &str, content: &[u8]) -> Vec<u8> {
        let mut hasher = Sha1::new();
        hasher.update(format!("{kind} {}\0", content.len()).as_bytes());
        hasher.update(content);
        hasher.finalize().to_vec()
    }
    fn raw(path: &Path) -> Result<Vec<u8>, String> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("{} ({})", error.kind(), path.display()))?;
        if metadata.is_file() {
            let content = std::fs::read(path).map_err(|error| error.kind().to_string())?;
            return Ok(object("blob", &content));
        }
        if !metadata.is_dir() {
            return Err(format!(
                "{} is neither a file nor a directory",
                path.display()
            ));
        }
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|error| error.kind().to_string())? {
            let entry = entry.map_err(|error| error.kind().to_string())?;
            let name = entry.file_name();
            if name == "__pycache__" {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| error.kind().to_string())?;
            let name = name
                .to_str()
                .ok_or_else(|| "entry name is not UTF-8".to_owned())?
                .to_owned();
            let (mode, sort_key) = if kind.is_dir() {
                ("40000", format!("{name}/"))
            } else if kind.is_file() {
                (
                    if executable(&entry.path()) {
                        "100755"
                    } else {
                        "100644"
                    },
                    name.clone(),
                )
            } else {
                return Err(format!(
                    "{} is neither a file nor a directory",
                    entry.path().display()
                ));
            };
            entries.push((sort_key, mode, name, raw(&entry.path())?));
        }
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut content = Vec::new();
        for (_, mode, name, id) in entries {
            content.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            content.extend_from_slice(&id);
        }
        Ok(object("tree", &content))
    }
    raw(path).map(|id| hex(&id))
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_: &Path) -> bool {
    false
}

/// Install directories a GUI process's `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`
/// under launchd) does not reach, tried after `PATH` when no interpreter was
/// named: Homebrew on Apple silicon and Intel, MacPorts, and the user's own
/// `bin` (pipx, uv). Resolving the interactive login shell's environment is
/// separate work; naming the interpreter (`XTRACE_PYTHON` in the app,
/// `--python`/`PYTHON` for the CLI) overrides the search.
#[cfg(target_os = "macos")]
const KNOWN_PYTHON_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "~/.local/bin",
];
#[cfg(not(target_os = "macos"))]
const KNOWN_PYTHON_DIRS: &[&str] = &["~/.local/bin"];

/// How long an interpreter may take to answer the version probe. A runtime
/// that cannot print its version within this is not one the readers can
/// use; the probe is killed and the candidate reported.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Select the interpreter: an explicit executable or `PYTHON` is probed as
/// named; otherwise `python3` on `PATH`, then in each known install directory,
/// the first that qualifies. Every reason a candidate did not qualify is
/// reported when none did. Probes are bounded and, with a token, cancellable.
pub fn discover_python(
    explicit: Option<&OsStr>,
    cancel: Option<&CancelToken>,
) -> Result<OsString, ReaderError> {
    let named = explicit
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PYTHON").filter(|value| !value.is_empty()));
    if let Some(named) = named {
        return resolve_python(Some(&named), cancel);
    }
    let mut reasons = Vec::new();
    match resolve_python(None, cancel) {
        Ok(python) => return Ok(python),
        Err(ReaderError::Cancelled) => return Err(ReaderError::Cancelled),
        Err(error) => reasons.push(format!("python3 on PATH: {error}")),
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for directory in KNOWN_PYTHON_DIRS {
        let directory = match (directory.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest),
            (Some(_), None) => continue,
            (None, _) => PathBuf::from(directory),
        };
        let candidate = directory.join("python3");
        if !candidate.is_file() {
            continue;
        }
        match resolve_python(Some(candidate.as_os_str()), cancel) {
            Ok(python) => return Ok(python),
            Err(ReaderError::Cancelled) => return Err(ReaderError::Cancelled),
            Err(error) => reasons.push(format!("{}: {error}", candidate.display())),
        }
    }
    Err(ReaderError::MissingRuntime(reasons.join("; ")))
}

/// Select the interpreter: an explicit executable, else `PYTHON`, else `python3`.
/// It must be 3.10+ and must not strip assertions. The probe is killed after
/// `PROBE_TIMEOUT`, and at once by a cancel of the token.
pub fn resolve_python(
    explicit: Option<&OsStr>,
    cancel: Option<&CancelToken>,
) -> Result<OsString, ReaderError> {
    resolve_python_within(explicit, cancel, PROBE_TIMEOUT)
}

/// `resolve_python` with an explicit probe bound (tests shorten it).
pub fn resolve_python_within(
    explicit: Option<&OsStr>,
    cancel: Option<&CancelToken>,
    timeout: Duration,
) -> Result<OsString, ReaderError> {
    let python = explicit
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PYTHON").filter(|value| !value.is_empty()))
        .unwrap_or_else(|| OsString::from("python3"));
    // The reader runs with the imported home as its working directory, so an
    // explicit relative path (as opposed to a bare command name found on PATH)
    // is anchored to the caller's directory now, before it changes.
    let path = Path::new(&python);
    let python = if path.components().count() > 1 && path.is_relative() {
        std::env::current_dir()
            .map_err(|_| ReaderError::MissingRuntime("current directory is unavailable".into()))?
            .join(path)
            .into_os_string()
    } else {
        python
    };
    #[cfg(unix)]
    let python = absolute_executable(
        &python,
        &std::env::current_dir()
            .map_err(|_| ReaderError::MissingRuntime("current directory unavailable".into()))?,
        &std::env::var_os("PATH").unwrap_or_default(),
    )?;
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(ReaderError::Cancelled);
    }
    let mut probe = Command::new(&python)
        .env_remove("PYTHONOPTIMIZE")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONSTARTUP")
        .args([
            "-c",
            "import sys; print(sys.version_info >= (3, 10) and sys.flags.optimize == 0)",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ReaderError::MissingRuntime("python3 is not executable".into()))?;
    let mut stdout = probe
        .stdout
        .take()
        .ok_or_else(|| ReaderError::MissingRuntime("probe output is unavailable".into()))?;
    // The answer is one short line, read after exit: a probe that floods its
    // output would fill the pipe and stall, which the deadline then ends.
    let slot: ChildSlot = Arc::new(Mutex::new(Some(probe)));
    if let Some(cancel) = cancel {
        cancel.register(&slot);
    }
    let awaited = await_child(&slot, Some(Instant::now() + timeout));
    if let Some(cancel) = cancel {
        cancel.unregister(&slot);
    }
    let (status, timed_out) = awaited?;
    if cancel.is_some_and(CancelToken::is_cancelled) && !status.success() {
        return Err(ReaderError::Cancelled);
    }
    if timed_out {
        return Err(ReaderError::MissingRuntime(format!(
            "interpreter probe did not finish within {timeout:?}"
        )));
    }
    let mut answer = String::new();
    let _ = std::io::Read::read_to_string(&mut stdout, &mut answer);
    if !status.success() || answer.trim() != "True" {
        return Err(ReaderError::MissingRuntime(
            "python3 must be 3.10 or newer with assertions enabled".into(),
        ));
    }
    Ok(python)
}

/// Cancels reader execution from another thread: a reader (or interpreter
/// probe) not yet started is refused, and every one running is killed (its
/// process reaped by the thread awaiting it) so a producer that never
/// returns cannot hold the caller. Cancellation is permanent for the token.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<CancelState>);

type ChildSlot = Arc<Mutex<Option<Child>>>;

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    /// The children running now, shared with their handles so a cancel that
    /// lands while they run kills exactly those processes; each is removed
    /// by the thread that reaped it.
    active: Mutex<Vec<ChildSlot>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Refuse children from now on and kill those running. The flag and the
    /// active list change under one lock, so a child registering
    /// concurrently is killed as well.
    pub fn cancel(&self) {
        let active = self
            .0
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.0.cancelled.store(true, Ordering::SeqCst);
        for slot in active.iter() {
            let mut child = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(child) = child.as_mut() {
                // A process that already exited is left for its handle to reap.
                let _ = child.kill();
            }
        }
    }

    /// Register a spawned child, killing it at once if a cancel landed
    /// between the caller's check and the spawn.
    fn register(&self, slot: &ChildSlot) {
        let mut active = self
            .0
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active.push(Arc::clone(slot));
        if self.0.cancelled.load(Ordering::SeqCst)
            && let Some(child) = slot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_mut()
        {
            let _ = child.kill();
        }
    }

    fn unregister(&self, slot: &ChildSlot) {
        self.0
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|known| !Arc::ptr_eq(known, slot));
    }
}

/// Await a registered child: the slot is unlocked between polls, so a cancel
/// can kill the process meanwhile, and a child still running at `deadline`
/// is killed too. Returns the exit status and whether the deadline killed it.
fn await_child(
    slot: &ChildSlot,
    deadline: Option<Instant>,
) -> Result<(std::process::ExitStatus, bool), ReaderError> {
    let mut timed_out = false;
    loop {
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let child = guard
            .as_mut()
            .ok_or_else(|| ReaderError::Failed("child process was already reaped".into()))?;
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status, timed_out)),
            Ok(None) => {
                if !timed_out && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    timed_out = true;
                    let _ = child.kill();
                }
            }
            Err(_) => {
                return Err(ReaderError::Failed(
                    "child process could not be awaited".into(),
                ));
            }
        }
        drop(guard);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A running pinned reader's exit half: the caller consumes the returned
/// stdout line by line, then waits here; stderr (static diagnostics only) is
/// drained concurrently.
pub struct ReaderHandle {
    child: ChildSlot,
    cancel: Option<CancelToken>,
    stderr: std::thread::JoinHandle<Vec<u8>>,
}

#[derive(Debug)]
pub struct ReaderOutcome {
    pub diagnostics: Vec<ReaderDiagnostic>,
    /// The producer exits 0 for complete coverage and 2 for incomplete coverage
    /// with partial successes still emitted.
    pub complete: bool,
}

/// Start the pinned reader for one host over `home`, reading every session it
/// discovers. Only the shared stream on stdout and static diagnostic codes on
/// stderr are consumed; a crash surfaces as a bounded failure text, never as
/// imported data. A cancelled token refuses the start; a cancel while the
/// reader runs kills it, which ends the stream, and `finish` reports it.
pub fn spawn_reader(
    python: &OsStr,
    producer: &PinnedProducer,
    host: Host,
    home: &Path,
    cancel: Option<&CancelToken>,
) -> Result<(std::io::BufReader<std::process::ChildStdout>, ReaderHandle), ReaderError> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(ReaderError::Cancelled);
    }
    let mut child = Command::new(python)
        .arg(&producer.script)
        .args(["--host", host.as_str()])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUTF8", "1")
        .env("NO_PROXY", "*")
        .current_dir(home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| ReaderError::Failed("reader process could not start".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ReaderError::Failed("reader stdout is unavailable".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ReaderError::Failed("reader stderr is unavailable".into()))?;
    let drain = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut bytes);
        bytes
    });
    let child: ChildSlot = Arc::new(Mutex::new(Some(child)));
    if let Some(cancel) = cancel {
        cancel.register(&child);
    }
    Ok((
        std::io::BufReader::new(stdout),
        ReaderHandle {
            child,
            cancel: cancel.cloned(),
            stderr: drain,
        },
    ))
}

impl ReaderHandle {
    /// Wait for the reader after its stdout has been consumed to its end and
    /// classify its diagnostics and exit status. The process is always
    /// reaped here, killed or not; a cancelled reader that did not exit
    /// normally reports `Cancelled`, one that had exited normally before the
    /// cancel landed reports its outcome, since its stream was complete.
    pub fn finish(self) -> Result<ReaderOutcome, ReaderError> {
        // The child stays registered, and the slot unlocked between polls,
        // until it has exited: a reader that closed its stream but runs on
        // can still be killed by a cancel while it is awaited here.
        let (status, _) = await_child(&self.child, None)?;
        if let Some(cancel) = &self.cancel {
            cancel.unregister(&self.child);
        }
        let cancelled = self.cancel.as_ref().is_some_and(CancelToken::is_cancelled);
        let stderr = self.stderr.join().unwrap_or_default();
        let stderr = String::from_utf8_lossy(&stderr);
        let mut diagnostics = Vec::new();
        let mut unexpected = None;
        for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value)
                    if value.get("type").and_then(serde_json::Value::as_str)
                        == Some("diagnostic") =>
                {
                    diagnostics.push(ReaderDiagnostic {
                        code: value
                            .get("code")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        path: value
                            .get("path")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                    });
                }
                _ => unexpected = Some(line.chars().take(160).collect::<String>()),
            }
        }
        let complete = match status.code() {
            Some(0) => true,
            Some(2) => false,
            _ if cancelled => return Err(ReaderError::Cancelled),
            code => {
                return Err(ReaderError::Failed(format!(
                    "reader exited with status {}: {}",
                    code.map_or("signal".to_owned(), |c| c.to_string()),
                    unexpected.unwrap_or_default()
                )));
            }
        };
        if let Some(text) = unexpected {
            return Err(ReaderError::Failed(format!(
                "reader wrote non-diagnostic output: {text}"
            )));
        }
        Ok(ReaderOutcome {
            diagnostics,
            complete,
        })
    }
}

#[cfg(unix)]
fn absolute_executable(name: &OsStr, cwd: &Path, search: &OsStr) -> Result<OsString, ReaderError> {
    use std::os::unix::fs::PermissionsExt;
    if Path::new(name).components().count() > 1 || Path::new(name).is_absolute() {
        return Ok(cwd.join(name).into_os_string());
    }
    std::env::split_paths(search)
        .map(|entry| cwd.join(entry).join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .map(PathBuf::into_os_string)
        .ok_or_else(|| ReaderError::MissingRuntime("python executable is not on PATH".into()))
}

#[cfg(all(test, unix))]
mod executable_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn relative_path_entries_are_resolved_before_changing_directory() {
        let temp = tempfile::TempDir::new().unwrap();
        let caller = temp.path().join("caller");
        let imported = temp.path().join("imported");
        for directory in [&caller, &imported] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("python"), "fixture").unwrap();
            std::fs::set_permissions(
                directory.join("python"),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        }
        let selected = absolute_executable(OsStr::new("python"), &caller, OsStr::new(".")).unwrap();
        assert_eq!(PathBuf::from(&selected), caller.join("./python"));
        assert!(Path::new(&selected).is_absolute());
        assert_ne!(
            selected,
            absolute_executable(OsStr::new("python"), &imported, OsStr::new(".")).unwrap()
        );
    }
}
