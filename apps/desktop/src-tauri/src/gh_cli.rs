//! Finding the GitHub CLI executable a manual pull-request refresh runs.
//!
//! The reviewed [`xt_probes::gh`] client refuses to search for its own
//! executable: it takes an already resolved absolute path. This is the only
//! place that resolves one, and it is deliberately the smallest resolver that
//! can answer the question:
//!
//! * an explicit absolute path from the backend-only [`GH_ENV`] override,
//!   which exists so tests and a deployment can name the executable;
//! * otherwise the first `gh` on this process's inherited `PATH`;
//! * otherwise `gh` in one of the documented standard install directories.
//!
//! Nothing else is consulted. It never runs a login shell to expand a `PATH`,
//! never starts a process (not even `gh --version`), never reads a
//! configuration file, token, keychain entry or any other credential, and
//! never offers to install or authenticate anything. The frontend cannot
//! supply a path: the override is read from the process environment at
//! startup, and the IPC surface carries stored pull-request IDs only.
//!
//! A candidate is accepted only as an absolute path to a regular file with an
//! execute bit. Resolution failures name no path, so the reason is safe to
//! show: a path the user configured is still a local filesystem path.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// The backend-only override, read from the process environment at startup.
pub const GH_ENV: &str = "XTRACE_GH";
/// The executable looked for in a search directory.
pub const EXECUTABLE: &str = "gh";

/// The documented standard install directories, searched in this order after
/// `PATH`. A desktop app launched from Finder inherits a short `PATH` that
/// usually omits them.
#[cfg(target_os = "macos")]
pub const KNOWN_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
];
#[cfg(not(target_os = "macos"))]
pub const KNOWN_DIRS: &[&str] = &[
    "/usr/local/bin",
    "/usr/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

/// Why no executable could be named. Every message is actionable and holds no
/// path, command line, environment value or credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GhUnavailable {
    #[error("the XTRACE_GH override must be an absolute path to the GitHub CLI executable")]
    OverrideNotAbsolute,
    #[error("the XTRACE_GH override does not name an executable file")]
    OverrideNotExecutable,
    #[error(
        "the GitHub CLI (gh) was not found on PATH or in a standard install directory; \
         install it, or set XTRACE_GH to its absolute path"
    )]
    NotFound,
    #[error("refreshing a pull request with the GitHub CLI needs a Unix host")]
    UnsupportedPlatform,
}

/// Resolve the executable from an explicit override and a `PATH` value,
/// against the documented standard directories.
pub fn resolve(explicit: Option<&OsStr>, search: Option<&OsStr>) -> Result<PathBuf, GhUnavailable> {
    let known: Vec<&Path> = KNOWN_DIRS.iter().map(Path::new).collect();
    resolve_within(explicit, search, &known)
}

/// [`resolve`] against explicit standard directories. Every input is supplied
/// rather than read here, so a test resolves against its own directories
/// without touching the process environment or this host's installation.
pub fn resolve_within(
    explicit: Option<&OsStr>,
    search: Option<&OsStr>,
    known: &[&Path],
) -> Result<PathBuf, GhUnavailable> {
    if !cfg!(unix) {
        return Err(GhUnavailable::UnsupportedPlatform);
    }
    if let Some(explicit) = explicit {
        let path = Path::new(explicit);
        if !path.is_absolute() {
            return Err(GhUnavailable::OverrideNotAbsolute);
        }
        return executable(path).ok_or(GhUnavailable::OverrideNotExecutable);
    }
    if let Some(search) = search {
        // A relative `PATH` entry would resolve against this process's working
        // directory, which is not a place an executable may come from.
        let found = std::env::split_paths(search)
            .filter(|entry| entry.is_absolute())
            .find_map(|entry| executable(&entry.join(EXECUTABLE)));
        if let Some(found) = found {
            return Ok(found);
        }
    }
    known
        .iter()
        .find_map(|directory| executable(&directory.join(EXECUTABLE)))
        .ok_or(GhUnavailable::NotFound)
}

/// The override and `PATH` as this process received them.
pub fn resolve_from_environment(explicit: Option<&OsStr>) -> Result<PathBuf, GhUnavailable> {
    resolve(explicit, std::env::var_os("PATH").as_deref())
}

/// An absolute path to a regular file with an execute bit, or nothing.
/// Metadata follows symbolic links, so the usual package-manager symlink is
/// accepted as the absolute path it is spelled with.
#[cfg(unix)]
fn executable(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if !path.is_absolute() {
        return None;
    }
    let metadata = path.metadata().ok()?;
    (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then(|| path.to_path_buf())
}

#[cfg(not(unix))]
fn executable(_: &Path) -> Option<PathBuf> {
    None
}
