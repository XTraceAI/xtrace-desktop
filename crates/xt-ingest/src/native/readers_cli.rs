//! Running the pinned shared readers. The producer is exactly the checkout named
//! by `.plugin-pin`: HEAD and every listed reader source object are verified
//! before the script runs. The readers see a disposable-looking environment
//! rooted at the requested home, no inherited interpreter overrides, and their
//! stdout is the shared stream; their stderr carries static diagnostic codes.

use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
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
    // Keep the immutable-source export alive through reader execution.
    _snapshot: std::sync::Arc<tempfile::TempDir>,
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
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRuntime(reason) => write!(f, "python runtime unavailable: {reason}"),
            Self::PinMismatch(reason) => write!(f, "pinned producer unavailable: {reason}"),
            Self::Failed(reason) => write!(f, "reader failed: {reason}"),
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
    let pin: Pin = serde_json::from_str(&text)
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
        _snapshot: std::sync::Arc::new(snapshot),
    })
}

/// Select the interpreter: an explicit executable, else `PYTHON`, else `python3`.
/// It must be 3.10+ and must not strip assertions.
pub fn resolve_python(explicit: Option<&OsStr>) -> Result<OsString, ReaderError> {
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
    let probe = Command::new(&python)
        .env_remove("PYTHONOPTIMIZE")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONSTARTUP")
        .args([
            "-c",
            "import sys; print(sys.version_info >= (3, 10) and sys.flags.optimize == 0)",
        ])
        .output()
        .map_err(|_| ReaderError::MissingRuntime("python3 is not executable".into()))?;
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != "True" {
        return Err(ReaderError::MissingRuntime(
            "python3 must be 3.10 or newer with assertions enabled".into(),
        ));
    }
    Ok(python)
}

/// A running pinned reader's exit half: the caller consumes the returned
/// stdout line by line, then waits here; stderr (static diagnostics only) is
/// drained concurrently.
pub struct ReaderHandle {
    child: std::process::Child,
    stderr: std::thread::JoinHandle<Vec<u8>>,
}

#[derive(Debug)]
pub struct ReaderOutcome {
    pub diagnostics: Vec<ReaderDiagnostic>,
    /// The producer exits 0 for complete coverage and 2 for incomplete coverage
    /// with partial successes still emitted.
    pub complete: bool,
}

/// Start the pinned reader for one host over `home`. Only the shared stream on
/// stdout and static diagnostic codes on stderr are consumed; a crash surfaces
/// as a bounded failure text, never as imported data.
pub fn spawn_reader(
    python: &OsStr,
    producer: &PinnedProducer,
    host: Host,
    home: &Path,
) -> Result<(std::io::BufReader<std::process::ChildStdout>, ReaderHandle), ReaderError> {
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
    Ok((
        std::io::BufReader::new(stdout),
        ReaderHandle {
            child,
            stderr: drain,
        },
    ))
}

impl ReaderHandle {
    /// Wait for the reader after its stdout has been consumed to its end and
    /// classify its diagnostics and exit status.
    pub fn finish(mut self) -> Result<ReaderOutcome, ReaderError> {
        let status = self
            .child
            .wait()
            .map_err(|_| ReaderError::Failed("reader process could not be awaited".into()))?;
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
