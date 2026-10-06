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
/// module or gained one, that holds a bytecode cache, or that carries another
/// producer's files is refused without Git being installed. The pin must list the scripts tree itself,
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
/// in `/`), and regular files carry mode 100644 or 100755 by their executable
/// bit. A Python bytecode cache, a symlink or any other entry kind is refused.
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
            // Python loads a cache whose recorded source size and time match
            // the module beside it, so bytecode the pin never covered could
            // run from one; a bundle that holds a cache is refused outright.
            if name == "__pycache__" {
                return Err(format!(
                    "{} holds a bytecode cache, which the pin never covers",
                    entry.path().display()
                ));
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
/// `--python` or `PYTHON` for the CLI, which reads that variable itself)
/// overrides the search. The library reads no environment variable for this:
/// a `PYTHON` a desktop process happens to inherit is not a choice.
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

/// Select the interpreter: an explicit executable is probed as named;
/// otherwise `python3` on `PATH`, then in each known install directory, the
/// first that qualifies. Every reason a candidate did not qualify is reported
/// when none did. Probes are bounded and, with a token, cancellable.
pub fn discover_python(
    explicit: Option<&OsStr>,
    cancel: Option<&CancelToken>,
) -> Result<OsString, ReaderError> {
    if explicit.is_some() {
        return resolve_python(explicit, cancel);
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

/// Probe the named interpreter, or `python3` on `PATH`. It must be 3.10+ and
/// must not strip assertions. The probe is killed after `PROBE_TIMEOUT`, and
/// at once by a cancel of the token.
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
    let mut probe = spawn_grouped(
        Command::new(&python)
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
            .stderr(std::process::Stdio::null()),
    )
    .map_err(|_| {
        ReaderError::MissingRuntime(format!(
            "{} is not executable",
            Path::new(&python).display()
        ))
    })?;
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
        return Err(ReaderError::MissingRuntime(format!(
            "{} must be Python 3.10 or newer with assertions enabled",
            Path::new(&python).display()
        )));
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
                terminate(child);
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
            terminate(child);
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

/// Kill a child and, on Unix, every process in the group it leads: readers
/// and probes are started as group leaders, so an interpreter launcher that
/// spawned the real interpreter without replacing itself cannot leave it
/// behind holding the stream (a descendant that left the group on its own is
/// beyond this). A child already reaped is left alone: its group ID may be
/// another's by now.
fn terminate(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: a plain signal to the unreaped child's own process group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

/// Start `command` as the leader of its own process group on Unix, so a
/// cancel or deadline can terminate everything it spawned.
fn spawn_grouped(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
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
                    terminate(child);
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

/// The pinned reader for one host over `home`, before any mode argument: the
/// environment every reader run sees, whatever it is asked to read.
fn reader_command(python: &OsStr, producer: &PinnedProducer, host: Host, home: &Path) -> Command {
    let mut command = Command::new(python);
    command
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
        .stderr(std::process::Stdio::piped());
    command
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
    spawn_export(reader_command(python, producer, host, home), cancel)
}

/// [`spawn_reader`] for Codex with the full export's opt-in origin evidence
/// (`--origin-evidence`): the same sessions and records, a marker on each
/// header the reader examined and a claim on each record it proves Codex
/// injected. Only Codex is ever asked for it. The same run carries the
/// image-wrapper length evidence and the automated-input evidence
/// (`--automated-input-evidence`, see [`super::tool_sent`]).
pub fn spawn_codex_reader_with_origin_evidence(
    python: &OsStr,
    producer: &PinnedProducer,
    home: &Path,
    cancel: Option<&CancelToken>,
) -> Result<(std::io::BufReader<std::process::ChildStdout>, ReaderHandle), ReaderError> {
    let mut command = reader_command(python, producer, Host::Codex, home);
    command.arg("--origin-evidence");
    command.arg("--human-input-adjustments");
    command.arg("--automated-input-evidence");
    spawn_export(command, cancel)
}

/// [`spawn_reader`] for Cursor with the full export's opt-in automated-input
/// evidence (`--automated-input-evidence`): the same sessions and records, and
/// a claim on each record the reader proves Cursor, or the reader itself,
/// wrote (see [`super::tool_sent`]).
pub fn spawn_cursor_reader_with_tool_sent_evidence(
    python: &OsStr,
    producer: &PinnedProducer,
    home: &Path,
    cancel: Option<&CancelToken>,
) -> Result<(std::io::BufReader<std::process::ChildStdout>, ReaderHandle), ReaderError> {
    let mut command = reader_command(python, producer, Host::Cursor, home);
    command.arg("--automated-input-evidence");
    spawn_export(command, cancel)
}

fn spawn_export(
    mut command: Command,
    cancel: Option<&CancelToken>,
) -> Result<(std::io::BufReader<std::process::ChildStdout>, ReaderHandle), ReaderError> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(ReaderError::Cancelled);
    }
    let mut child = spawn_grouped(&mut command)
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

/// The most this process reads from an exact-detail reader's stdout: the
/// producer's own ceiling on one encoded session, header line included. A
/// reader that writes more has not kept the contract, and is refused rather
/// than read further.
pub const DETAIL_MAX_STDOUT: usize = 32 * 1024 * 1024;
/// The most this process reads from an exact-detail reader's stderr. The
/// producer writes a few static diagnostic lines at most.
pub const DETAIL_MAX_STDERR: usize = 64 * 1024;
/// The deadline handed to the producer, which refuses from inside once it
/// passes. The default is the producer's own.
pub const DETAIL_DEADLINE: Duration = Duration::from_secs(30);
/// How long past the producer's deadline this process waits before it kills
/// the reader's process group itself.
pub const DETAIL_GRACE: Duration = Duration::from_secs(5);
/// How long a reader's output may stay open after its group was killed and
/// reaped. Only a process that left the group can hold it open that long.
const PIPE_SETTLE: Duration = Duration::from_secs(1);
/// How much of a pipe is read per call.
const PIPE_CHUNK: usize = 64 * 1024;

/// What one exact-detail run may cost this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailBounds {
    /// Passed to the producer as `--deadline-seconds`.
    pub deadline: Duration,
    /// Added to `deadline` for the hard deadline this process enforces.
    pub grace: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
}

impl Default for DetailBounds {
    fn default() -> Self {
        Self {
            deadline: DETAIL_DEADLINE,
            grace: DETAIL_GRACE,
            max_stdout: DETAIL_MAX_STDOUT,
            max_stderr: DETAIL_MAX_STDERR,
        }
    }
}

/// One of the producer's declared exact-detail ceilings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailLimit {
    SourceBytes,
    Files,
    NativeRows,
    Records,
    LineBytes,
    OutputBytes,
    HeaderBytes,
    DiscoveryEntries,
    /// Identification, not the selected session: one identity probe of a
    /// discovered file read as much as it may.
    HeaderProbeBytes,
    /// Identification: the identity probes of every discovered file together
    /// read as much as they may.
    ProbeBytes,
    /// Identification: more discovered files than may be probed for identity.
    Probes,
}

/// Why the producer refused an exact-detail read, from its static diagnostic
/// code. Nothing else of what it wrote is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailRefusal {
    Limit(DetailLimit),
    /// The deadline the producer was handed passed while it worked.
    Deadline,
    /// The session's selected representation is a SQLite store, which this
    /// mode is not defined over. Its transcript is never read in its place.
    StoreUnsupported,
    /// A source changed while it was being read.
    SourceChanged,
    /// No source names the session, or more than one does.
    Unavailable,
    /// The session's source could not be read.
    Unreadable,
    /// Part of the history could not be enumerated, so the session cannot be
    /// known to be the only one of its name.
    DiscoveryIncomplete,
}

impl DetailRefusal {
    fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "detail_limit_source_bytes" => Self::Limit(DetailLimit::SourceBytes),
            "detail_limit_files" => Self::Limit(DetailLimit::Files),
            "detail_limit_native_rows" => Self::Limit(DetailLimit::NativeRows),
            "detail_limit_records" => Self::Limit(DetailLimit::Records),
            "detail_limit_line_bytes" => Self::Limit(DetailLimit::LineBytes),
            "detail_limit_output_bytes" => Self::Limit(DetailLimit::OutputBytes),
            "detail_limit_header_bytes" => Self::Limit(DetailLimit::HeaderBytes),
            "detail_limit_discovery_entries" => Self::Limit(DetailLimit::DiscoveryEntries),
            "detail_limit_header_probe_bytes" => Self::Limit(DetailLimit::HeaderProbeBytes),
            "detail_limit_probe_bytes" => Self::Limit(DetailLimit::ProbeBytes),
            "detail_limit_probes" => Self::Limit(DetailLimit::Probes),
            "detail_deadline" => Self::Deadline,
            "detail_prerequisite_unsupported" => Self::StoreUnsupported,
            "source_changed" => Self::SourceChanged,
            "session_unavailable" => Self::Unavailable,
            "session_unreadable" => Self::Unreadable,
            "discovery_incomplete" => Self::DiscoveryIncomplete,
            _ => return None,
        })
    }
}

/// Why an exact-detail run produced nothing. Every variant is a closed value:
/// no producer output, path or message crosses it, and whatever the reader
/// wrote before failing has already been discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailFailure {
    /// The caller cancelled: the reader was not started, or its group was
    /// killed and reaped.
    Cancelled,
    /// This process's hard deadline passed: the reader's group was killed and
    /// reaped.
    Deadline,
    /// The reader wrote more than this process reads from it.
    OutputBound,
    /// The producer refused, and said why with a known code.
    Refused(DetailRefusal),
    /// The reader did not keep the exact-detail protocol.
    Protocol(&'static str),
    /// The reader could not be started.
    Start,
}

/// Which pipe a drained result belongs to.
#[derive(Clone, Copy)]
enum Pipe {
    Stdout,
    Stderr,
}

/// A pipe read to its end, or abandoned at its bound.
struct Drained {
    bytes: Vec<u8>,
    /// The pipe held more than its bound, or memory for it was unavailable;
    /// `bytes` is empty.
    overflowed: bool,
    failed: bool,
}

/// Read `pipe` to its end, never holding more than `cap` bytes. A chunk that
/// would take the buffer past `cap` is refused before any memory is reserved
/// for it, and growth is reserved exactly, so the buffer's capacity never
/// exceeds `cap` either. On overflow `overflow` is raised so the reader can be
/// killed at once, and what was read is dropped.
fn drain_bounded(mut pipe: impl std::io::Read, cap: usize, overflow: &AtomicBool) -> Drained {
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = vec![0_u8; PIPE_CHUNK];
    let abandoned = |failed: bool| Drained {
        bytes: Vec::new(),
        overflowed: !failed,
        failed,
    };
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => {
                return Drained {
                    bytes,
                    overflowed: false,
                    failed: false,
                };
            }
            Ok(read) => {
                if read > cap - bytes.len() {
                    overflow.store(true, Ordering::SeqCst);
                    return abandoned(false);
                }
                if bytes.capacity() - bytes.len() < read {
                    let want = bytes
                        .capacity()
                        .saturating_mul(2)
                        .max(bytes.len() + read)
                        .min(cap);
                    if bytes.try_reserve_exact(want - bytes.len()).is_err() {
                        overflow.store(true, Ordering::SeqCst);
                        return abandoned(false);
                    }
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return abandoned(true),
        }
    }
}

/// Whether the child has exited, without reaping it: an exited leader that is
/// not yet reaped still holds its process ID, so its group can be signalled
/// safely. Only [`run_exact_detail`] reaps its child, so an error here cannot
/// mean another reaper took it; it is treated as exited, and the reap below
/// reports it.
#[cfg(unix)]
fn exited_unreaped(child: &Child) -> bool {
    // `id_t` is the unsigned process ID `Child::id` already is.
    let pid: libc::id_t = child.id();
    // SAFETY: a zeroed siginfo is a valid out-parameter for waitid, which
    // only inspects the caller's own child and, with WNOWAIT, leaves it
    // waitable.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if status != 0 {
        return true;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let exited = unsafe { info.si_pid() } != 0;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let exited = info.si_pid != 0;
    exited
}

#[cfg(not(unix))]
fn exited_unreaped(child: &Child) -> bool {
    // Without process groups there is nothing to signal before reaping; the
    // reap below observes the exit.
    let _ = child;
    false
}

/// Kill the child's whole process group, then reap the leader. The caller is
/// the child's only reaper — it is registered with no cancel token, whose
/// `terminate` would reap an exited leader first — so the leader is always
/// unreaped here, running or a zombie, and its ID, which is the group's,
/// cannot have been reused when the group is signalled.
fn kill_group_and_reap(child: &mut Child) -> Result<std::process::ExitStatus, DetailFailure> {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: a signal to the group of this process's own unreaped child.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    child
        .wait()
        .map_err(|_| DetailFailure::Protocol("reader process could not be reaped"))
}

/// Run the pinned reader in exact-detail mode for one identified session and
/// return its stdout, whole, only if the run was a complete validated success:
/// the process exited `0`, wrote nothing to stderr, and closed its output
/// within the bounds. Everything else returns a closed failure and discards
/// all the reader wrote.
///
/// The session is passed as `--session=<id>`, so an identity is never read as
/// an option, and the caller must have refused anything that could name a
/// path. Both pipes are drained concurrently into buffers bounded before any
/// allocation; a pipe that exceeds its bound kills the reader at once. The
/// reader runs as the leader of its own process group, and the group is
/// killed before the leader is reaped on every path — success included — so
/// nothing it started outlives the call. The hard deadline is the producer's
/// deadline plus `grace`. The token is polled, never handed the child: this
/// function is the child's only reaper, so a cancel — whenever it lands, the
/// leader exited or not — kills the group before the leader is reaped.
pub fn run_exact_detail(
    python: &OsStr,
    producer: &PinnedProducer,
    host: Host,
    home: &Path,
    native_session_id: &str,
    bounds: DetailBounds,
    cancel: Option<&CancelToken>,
) -> Result<Vec<u8>, DetailFailure> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(DetailFailure::Cancelled);
    }
    let mut command = reader_command(python, producer, host, home);
    command
        .arg("--exact-detail")
        .arg(format!("--session={native_session_id}"))
        .arg("--deadline-seconds")
        .arg(format!("{:.3}", bounds.deadline.as_secs_f64()));
    let hard = Instant::now() + bounds.deadline + bounds.grace;
    let mut child = spawn_grouped(&mut command).map_err(|_| DetailFailure::Start)?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = kill_group_and_reap(&mut child);
        return Err(DetailFailure::Start);
    };
    let overflow = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = std::sync::mpsc::channel::<(Pipe, Drained)>();
    for (pipe, reader, cap) in [
        (
            Pipe::Stdout,
            Box::new(stdout) as Box<dyn std::io::Read + Send>,
            bounds.max_stdout,
        ),
        (Pipe::Stderr, Box::new(stderr), bounds.max_stderr),
    ] {
        let sender = sender.clone();
        let overflow = Arc::clone(&overflow);
        // A drain that outlives the call (a pipe held open by a process that
        // left the group) ends when that process does; its result is dropped.
        std::thread::spawn(move || {
            let _ = sender.send((pipe, drain_bounded(reader, cap, &overflow)));
        });
    }
    drop(sender);
    // Stdout, then stderr, once each pipe has been read to its end.
    let mut drained: [Option<Drained>; 2] = [None, None];
    let collect = |drained: &mut [Option<Drained>; 2], (pipe, result): (Pipe, Drained)| {
        drained[pipe as usize] = Some(result);
    };
    // Stopped early, and why; `None` means the reader ran to its own end.
    let mut stopped: Option<DetailFailure> = None;
    let status = loop {
        while let Ok(received) = receiver.try_recv() {
            collect(&mut drained, received);
        }
        if cancel.is_some_and(CancelToken::is_cancelled) {
            stopped = Some(DetailFailure::Cancelled);
        } else if overflow.load(Ordering::SeqCst) {
            stopped = Some(DetailFailure::OutputBound);
        } else if Instant::now() >= hard {
            stopped = Some(DetailFailure::Deadline);
        }
        // The output is complete only once both pipes have closed; a leader
        // that exited while something it started still holds them open is
        // waited for up to the deadline, not taken at its word.
        let finished = drained.iter().all(Option::is_some) && exited_unreaped(&child);
        if stopped.is_some() || finished {
            break kill_group_and_reap(&mut child);
        }
        if let Ok(received) = receiver.recv_timeout(Duration::from_millis(10)) {
            collect(&mut drained, received);
        }
    };
    // The group is gone; its pipes close with it unless a process left it.
    let settle = Instant::now() + PIPE_SETTLE;
    while drained.iter().any(Option::is_none) {
        let remaining = settle.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(received) => collect(&mut drained, received),
            Err(_) => break,
        }
    }
    // A cancel that landed at any point discards the run, whatever it did.
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(DetailFailure::Cancelled);
    }
    if let Some(stopped) = stopped {
        return Err(stopped);
    }
    let status = status?;
    let [Some(out), Some(err)] = drained else {
        return Err(DetailFailure::Protocol("reader output did not close"));
    };
    if out.overflowed || err.overflowed {
        return Err(DetailFailure::OutputBound);
    }
    if out.failed || err.failed {
        return Err(DetailFailure::Protocol("reader output could not be read"));
    }
    match status.code() {
        Some(0) if err.bytes.iter().all(u8::is_ascii_whitespace) => Ok(out.bytes),
        Some(0) => Err(DetailFailure::Protocol(
            "reader wrote diagnostics beside a complete session",
        )),
        Some(2) => Err(refusal(&err.bytes, host)),
        _ => Err(DetailFailure::Protocol("reader exited abnormally")),
    }
}

/// The producer's reason for a refused exact read: every stderr line must be
/// one of its diagnostics for this host with a known code, and the first one
/// names the refusal. Anything else is a broken protocol, and none of its text
/// is kept.
fn refusal(stderr: &[u8], host: Host) -> DetailFailure {
    let broken = DetailFailure::Protocol("reader refused without a known diagnostic");
    let Ok(text) = std::str::from_utf8(stderr) else {
        return broken;
    };
    let mut first = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return broken;
        };
        let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
        if field("type") != Some("diagnostic") || field("host") != Some(host.as_str()) {
            return broken;
        }
        let Some(refusal) = field("code").and_then(DetailRefusal::from_code) else {
            return broken;
        };
        first.get_or_insert(refusal);
    }
    first.map_or(broken, DetailFailure::Refused)
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
