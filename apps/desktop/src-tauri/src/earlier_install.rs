//! The one-time copy of an earlier install's data. Through release 0.1.3 the
//! app's ID was `ai.xtrace.desktop`, so its data folder and its WebKit storage
//! (theme, welcome done, Dashboard split) are named after that ID. Under the
//! current ID the app looks in new, empty folders; on the first start this
//! copies the earlier ones there before any database opens and before the
//! first window loads.
//!
//! The earlier folders are only read, never changed: they stay as the way
//! back. [`decide`] is the one rule for whether a copy runs.
use crate::state::{DATABASE_FILE, DataFolder, StartupOptions, StateError};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// The app ID XTrace Desktop used through release 0.1.3.
pub const EARLIER_IDENTIFIER: &str = "ai.xtrace.desktop";
/// Written into the new data folder by a copy: where it came from and when.
/// While it is there no copy runs again, even with the database gone.
pub const MARKER_FILE: &str = "copied-from-earlier-install.json";
/// A copy is built in a sibling folder with this suffix and renamed into
/// place only when complete, so a half-finished copy is never used.
const STAGING_SUFFIX: &str = ".copy-in-progress";
/// A WebKit folder being replaced is set aside under this suffix until the
/// copy is in its place.
const REPLACED_SUFFIX: &str = ".replaced";
/// Files SQLite keeps beside the database while it is in use.
const DATABASE_COMPANIONS: [&str; 3] = ["-wal", "-shm", "-journal"];

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    /// The earlier app could be writing its data while it is copied.
    #[error(
        "The earlier XTrace Desktop is still running. Quit it, then open XTrace Desktop \
         again so it can bring over your data."
    )]
    EarlierRunning,
    /// Earlier data may be there but could not be looked at. The app does
    /// not start over an empty database while it might exist.
    #[error(
        "XTrace found data from the earlier version but could not read it, so it did not \
         start with empty data. The earlier data is unchanged"
    )]
    EarlierUnreadable(#[source] io::Error),
    #[error(
        "XTrace could not copy its data from the earlier install, and left the earlier \
         folder unchanged"
    )]
    Copy(#[from] io::Error),
    /// The copy of the earlier database did not open. It is not the earlier
    /// file, which was not opened and is unchanged.
    #[error(
        "XTrace couldn't bring over the data from the earlier version. The earlier data is \
         unchanged and nothing was deleted."
    )]
    Database(#[source] xt_store::Error),
}

/// The earlier and current data folders. The earlier one sits beside the
/// current one, named after the earlier ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    pub earlier: PathBuf,
    pub current: PathBuf,
}

impl Folders {
    pub fn beside(current: PathBuf) -> Option<Self> {
        let earlier = current.parent()?.join(EARLIER_IDENTIFIER);
        Some(Self { earlier, current })
    }

    fn staging(&self) -> PathBuf {
        sibling(&self.current, STAGING_SUFFIX)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Copy(Folders),
    Skip(Skip),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Fixture startup never selects live data.
    Fixture,
    /// `XTRACE_DATA_DIR` names the data folder; the default one is not used.
    DataDirOverride,
    /// The host has no application-data folder to give.
    NoDataFolder,
    /// The current ID is the earlier one: there is nothing to copy from.
    SameFolder,
    /// The current folder already has a database: copied, or started fresh.
    HasDatabase,
    /// A copy already ran; the database was removed since. It is not copied
    /// again.
    AlreadyCopied,
    NoEarlierDatabase,
}

/// Whether the earlier install's data is copied: only into the app's default
/// data folder (as [`StartupOptions::data_folder`] decides), only once (no
/// marker of an earlier copy), only when that folder has no database and the
/// earlier folder has one. Only a missing earlier database means there is
/// nothing to copy; one that cannot be looked at, or is not a regular file,
/// is an error rather than a reason to start empty.
pub fn decide(
    options: &StartupOptions,
    default_dir: impl FnOnce() -> Result<PathBuf, StateError>,
) -> Result<Decision, CopyError> {
    let current = match options.data_folder(default_dir) {
        Ok(DataFolder::Fixture) => return Ok(Decision::Skip(Skip::Fixture)),
        Ok(DataFolder::Chosen(_)) => return Ok(Decision::Skip(Skip::DataDirOverride)),
        Ok(DataFolder::Default(path)) => path,
        // `start` resolves the folder again and reports the failure.
        Err(_) => return Ok(Decision::Skip(Skip::NoDataFolder)),
    };
    let Some(folders) = Folders::beside(current) else {
        return Ok(Decision::Skip(Skip::NoDataFolder));
    };
    if folders.current == folders.earlier {
        return Ok(Decision::Skip(Skip::SameFolder));
    }
    // Anything under these names, even one that cannot be read, counts: a
    // copy never replaces it.
    if !absent(&folders.current.join(DATABASE_FILE)) {
        return Ok(Decision::Skip(Skip::HasDatabase));
    }
    if !absent(&folders.current.join(MARKER_FILE)) {
        return Ok(Decision::Skip(Skip::AlreadyCopied));
    }
    match fs::symlink_metadata(folders.earlier.join(DATABASE_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(Decision::Skip(Skip::NoEarlierDatabase))
        }
        Err(error) => Err(CopyError::EarlierUnreadable(error)),
        Ok(meta) if meta.is_file() => Ok(Decision::Copy(folders)),
        // A link could lead the copy's writes back into the earlier folder.
        Ok(_) => Err(CopyError::EarlierUnreadable(io::Error::other(
            "the earlier database is not a regular file",
        ))),
    }
}

/// Copies the earlier data folder into the current one. `earlier_running`
/// says whether the earlier app is running; while it is, nothing is copied.
///
/// The copy is built in a sibling staging folder (a leftover from a failed
/// attempt is removed first). The copied database is opened there, which
/// checks it opens and folds its write-ahead log into the file, and a marker
/// is written. Only then is it moved into place, the database and then the
/// marker last, so the current folder never holds a database from a
/// half-finished copy, nor a marker without the database. A failure removes
/// the staging folder and leaves the current folder without a database or a
/// marker, so the next start tries again.
pub fn copy(folders: &Folders, earlier_running: impl FnOnce() -> bool) -> Result<(), CopyError> {
    if earlier_running() {
        return Err(CopyError::EarlierRunning);
    }
    let staging = folders.staging();
    let result = copy_through(folders, &staging);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn copy_through(folders: &Folders, staging: &Path) -> Result<(), CopyError> {
    remove_leftover(staging)?;
    let before = fingerprint(&folders.earlier)?;
    copy_tree(&folders.earlier, staging)?;
    // A database changed under the copy means a writer is still running: the
    // files copied need not agree with each other.
    if fingerprint(&folders.earlier)? != before {
        return Err(CopyError::EarlierRunning);
    }
    let store = xt_store::Store::open(staging.join(DATABASE_FILE)).map_err(CopyError::Database)?;
    // Closing the only connection checkpoints the write-ahead log into the
    // copied file.
    drop(store);
    write_marker(staging, &folders.earlier)?;
    place(staging, &folders.current)?;
    Ok(())
}

/// Copies the earlier app's WebKit storage, which holds the screen's saved
/// settings, to the folder WebKit uses under `identifier`. It runs only right
/// after the data copy, which means this ID has never had real use, so a
/// folder already there (from a test launch, say) is replaced. It must run
/// before the first webview of this app starts. The copy is built in a
/// staging folder and renamed into place; the folder it replaces is set aside
/// until then and put back if that fails. Returns whether it copied.
pub fn copy_screen_settings(home: &Path, identifier: &str) -> io::Result<bool> {
    let root = home.join("Library").join("WebKit");
    let earlier = root.join(EARLIER_IDENTIFIER);
    let current = root.join(identifier);
    if identifier == EARLIER_IDENTIFIER {
        return Ok(false);
    }
    let staging = sibling(&current, STAGING_SUFFIX);
    let replaced = sibling(&current, REPLACED_SUFFIX);
    remove_leftover(&staging)?;
    remove_leftover(&replaced)?;
    if !fs::symlink_metadata(&earlier).is_ok_and(|meta| meta.is_dir()) {
        return Ok(false);
    }
    let result = copy_tree(&earlier, &staging).and_then(|()| {
        if !absent(&current) {
            fs::rename(&current, &replaced)?;
        }
        fs::rename(&staging, &current)
    });
    if result.is_err() {
        if absent(&current) && !absent(&replaced) {
            let _ = fs::rename(&replaced, &current);
        }
        let _ = fs::remove_dir_all(&staging);
    }
    let _ = remove_leftover(&replaced);
    result.map(|()| true)
}

/// Whether an app with the earlier ID is running. A process launched from
/// the earlier bundle keeps that ID even after the bundle on disk was
/// replaced.
#[cfg(target_os = "macos")]
pub fn earlier_app_running() -> bool {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
        EARLIER_IDENTIFIER,
    ))
    .iter()
    .any(|app| !app.isTerminated())
}

#[cfg(not(target_os = "macos"))]
pub fn earlier_app_running() -> bool {
    false
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// True only when nothing at all is at `path`, not even a broken link.
fn absent(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

fn remove_leftover(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// The size and modification time of the database and its write-ahead and
/// rollback journals.
fn fingerprint(folder: &Path) -> io::Result<Vec<Option<(u64, SystemTime)>>> {
    ["", "-wal", "-journal"]
        .into_iter()
        .map(
            |suffix| match fs::symlink_metadata(folder.join(format!("{DATABASE_FILE}{suffix}"))) {
                Ok(meta) => Ok(Some((meta.len(), meta.modified()?))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            },
        )
        .collect()
}

/// Copies folders and regular files, with their permissions. Links and
/// special files are not copied: the app writes neither, and a link could
/// lead a later write back into the earlier folder.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    // Last, so a folder without write permission is still filled first.
    fs::set_permissions(to, fs::metadata(from)?.permissions())
}

fn write_marker(staging: &Path, earlier: &Path) -> io::Result<()> {
    let marker = serde_json::json!({
        "copiedFrom": earlier.to_string_lossy(),
        "copiedAt": jiff::Timestamp::now().to_string(),
    });
    fs::write(
        staging.join(MARKER_FILE),
        serde_json::to_vec_pretty(&marker).map_err(io::Error::other)?,
    )
}

/// Moves the finished copy into the current folder. When that folder does
/// not exist the whole copy is renamed at once. Otherwise (it exists, with no
/// database and no marker) each entry it does not already have is moved in,
/// then the database, whose arrival marks the copy complete, and the marker
/// last, so a marker never stands without the database. Database companions
/// left there without a database are removed first, so none is ever paired
/// with the copied database.
fn place(staging: &Path, current: &Path) -> io::Result<()> {
    if absent(current) {
        return fs::rename(staging, current);
    }
    for companion in DATABASE_COMPANIONS {
        match fs::remove_file(current.join(format!("{DATABASE_FILE}{companion}"))) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    for entry in fs::read_dir(staging)? {
        let name = entry?.file_name();
        if name == DATABASE_FILE || name == MARKER_FILE {
            continue;
        }
        let target = current.join(&name);
        if absent(&target) {
            fs::rename(staging.join(&name), target)?;
        }
    }
    fs::rename(staging.join(DATABASE_FILE), current.join(DATABASE_FILE))?;
    // The copy is complete; the database alone already stops a second copy.
    let _ = fs::rename(staging.join(MARKER_FILE), current.join(MARKER_FILE));
    // The copy is in place; what is left are entries the current folder
    // already had. Failing to remove them does not undo the copy.
    let _ = fs::remove_dir_all(staging);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};
    use xt_store::{Store, typing_speed::TypingSpeed};

    const CURRENT: &str = "ai.xtrace.app";
    const HISTORY: &str = "account-usage-history.jsonl";
    const LAST_READING: &str = "claude-usage-last.json";
    const PROBE: &str = "xtrace-claude-usage-probe";

    /// Every entry under a folder: its contents (for a file) and permissions.
    type Snapshot = BTreeMap<PathBuf, (Option<Vec<u8>>, u32)>;

    fn snapshot(root: &Path) -> Snapshot {
        let mut entries = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                let contents = if meta.is_dir() {
                    pending.push(path.clone());
                    None
                } else {
                    Some(fs::read(&path).unwrap())
                };
                entries.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (contents, meta.permissions().mode()),
                );
            }
        }
        entries
    }

    fn folders(root: &Path) -> Folders {
        let support = root.join("Application Support");
        fs::create_dir_all(&support).unwrap();
        Folders::beside(support.join(CURRENT)).unwrap()
    }

    fn options() -> StartupOptions {
        StartupOptions::default()
    }

    /// An earlier folder as the app leaves it: a database whose last write is
    /// still only in its write-ahead log (as after the app was ended), the
    /// usage history, the last reading and the probe folder.
    fn earlier(folders: &Folders, wpm: u32) {
        fs::create_dir_all(folders.earlier.join(PROBE).join(".claude")).unwrap();
        fs::set_permissions(
            folders.earlier.join(PROBE),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::write(
            folders.earlier.join(PROBE).join(".claude/settings.json"),
            b"{}",
        )
        .unwrap();
        fs::write(folders.earlier.join(HISTORY), b"{\"a\":1}\n{\"a\":2}\n").unwrap();
        fs::write(folders.earlier.join(LAST_READING), b"{\"reading\":1}").unwrap();
        let mut store = Store::open(folders.earlier.join(DATABASE_FILE)).unwrap();
        store
            .set_typing_speed(TypingSpeed::new(wpm).unwrap())
            .unwrap();
        // Never closed: the write stays in the log, not the database file.
        std::mem::forget(store);
        assert!(
            fs::metadata(folders.earlier.join("xtrace.db-wal"))
                .unwrap()
                .len()
                > 0,
            "the earlier database has a write-ahead log to carry"
        );
    }

    fn typing_speed(db: &Path) -> u32 {
        Store::open(db).unwrap().typing_speed().unwrap().wpm()
    }

    /// The rule over the default folder `current`, which must not fail.
    fn decision(options: &StartupOptions, current: &Path) -> Decision {
        decide(options, || Ok(current.to_path_buf())).unwrap()
    }

    #[test]
    fn the_rule_copies_only_once_into_the_default_folder_with_no_database() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        let current = folders.current.as_path();
        assert_eq!(
            decision(&options(), current),
            Decision::Skip(Skip::NoEarlierDatabase)
        );
        fs::create_dir_all(&folders.earlier).unwrap();
        fs::write(folders.earlier.join(DATABASE_FILE), b"").unwrap();
        assert_eq!(
            decision(&options(), current),
            Decision::Copy(folders.clone())
        );
        // The overrides never resolve the default folder: the same rule
        // `AppState::build` uses.
        let unresolved =
            || -> Result<PathBuf, StateError> { panic!("the default folder is not resolved") };
        let data_dir = StartupOptions {
            data_dir: Some(root.path().join("elsewhere")),
            ..options()
        };
        assert_eq!(
            decide(&data_dir, unresolved).unwrap(),
            Decision::Skip(Skip::DataDirOverride)
        );
        let fixture = StartupOptions {
            fixture: Some("F1".into()),
            ..options()
        };
        assert_eq!(
            decide(&fixture, unresolved).unwrap(),
            Decision::Skip(Skip::Fixture)
        );
        assert_eq!(
            decide(&options(), || Err(StateError::InvalidOption)).unwrap(),
            Decision::Skip(Skip::NoDataFolder)
        );
        assert_eq!(
            decision(&options(), &folders.earlier),
            Decision::Skip(Skip::SameFolder)
        );
        // Anything under the database's name in the current folder stops it.
        fs::create_dir_all(current).unwrap();
        std::os::unix::fs::symlink("missing", current.join(DATABASE_FILE)).unwrap();
        assert_eq!(
            decision(&options(), current),
            Decision::Skip(Skip::HasDatabase)
        );
        fs::remove_file(current.join(DATABASE_FILE)).unwrap();
        fs::write(current.join(DATABASE_FILE), b"").unwrap();
        assert_eq!(
            decision(&options(), current),
            Decision::Skip(Skip::HasDatabase)
        );
        // One time only: the marker of a copy stops another, even with the
        // database gone.
        fs::remove_file(current.join(DATABASE_FILE)).unwrap();
        fs::write(current.join(MARKER_FILE), b"{}").unwrap();
        assert_eq!(
            decision(&options(), current),
            Decision::Skip(Skip::AlreadyCopied)
        );
    }

    #[test]
    fn earlier_data_that_cannot_be_read_stops_the_start_instead_of_starting_empty() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 70);
        let current = || Ok(folders.current.clone());

        // A folder that cannot be looked into.
        fs::set_permissions(&folders.earlier, fs::Permissions::from_mode(0o000)).unwrap();
        let error = decide(&options(), current).unwrap_err();
        fs::set_permissions(&folders.earlier, fs::Permissions::from_mode(0o755)).unwrap();
        let CopyError::EarlierUnreadable(source) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
        let failure = crate::startup_failure::message(&error);
        assert!(
            failure.body.starts_with(
                "XTrace found data from the earlier version but could not read it, so it did \
                 not start with empty data. The earlier data is unchanged: "
            ),
            "{}",
            failure.body
        );
        assert!(absent(&folders.current));

        // A database reached through a link is not copied, nor ignored.
        fs::rename(
            folders.earlier.join(DATABASE_FILE),
            root.path().join("elsewhere.db"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            root.path().join("elsewhere.db"),
            folders.earlier.join(DATABASE_FILE),
        )
        .unwrap();
        assert!(matches!(
            decide(&options(), current),
            Err(CopyError::EarlierUnreadable(_))
        ));
        assert!(absent(&folders.current));
    }

    #[test]
    fn a_copy_carries_every_file_and_the_logged_write_and_leaves_the_earlier_folder_unchanged() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 123);
        let before = snapshot(&folders.earlier);

        let Decision::Copy(decided) = decision(&options(), &folders.current) else {
            panic!("only the earlier folder exists: the data is copied");
        };
        copy(&decided, || false).unwrap();

        assert_eq!(
            snapshot(&folders.earlier),
            before,
            "earlier folder unchanged"
        );
        assert!(absent(&folders.staging()), "no staging folder is left");
        let current = &folders.current;
        assert_eq!(
            fs::read(current.join(HISTORY)).unwrap(),
            b"{\"a\":1}\n{\"a\":2}\n"
        );
        assert_eq!(
            fs::read(current.join(LAST_READING)).unwrap(),
            b"{\"reading\":1}"
        );
        assert_eq!(
            fs::read(current.join(PROBE).join(".claude/settings.json")).unwrap(),
            b"{}"
        );
        assert_eq!(
            fs::metadata(current.join(PROBE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        // The log was folded into the copied file.
        assert!(absent(&current.join("xtrace.db-wal")));
        let marker: serde_json::Value =
            serde_json::from_slice(&fs::read(current.join(MARKER_FILE)).unwrap()).unwrap();
        assert_eq!(
            marker["copiedFrom"],
            folders.earlier.to_string_lossy().as_ref()
        );
        marker["copiedAt"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .unwrap();
        assert_eq!(typing_speed(&current.join(DATABASE_FILE)), 123);

        // The app then starts over the copied folder.
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let state = crate::state::AppState::build(
            StartupOptions {
                data_dir: Some(current.clone()),
                native_home: Some(home),
                ..options()
            },
            || panic!("default path must not be resolved"),
            || panic!("default home must not be resolved"),
        )
        .unwrap();
        state.db_counts().unwrap();
        state.shutdown();

        // The next start finds the database and copies nothing again.
        assert_eq!(
            decision(&options(), &folders.current),
            Decision::Skip(Skip::HasDatabase)
        );
    }

    #[test]
    fn a_running_earlier_app_stops_the_copy_with_a_plain_message() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 90);
        let before = snapshot(&folders.earlier);

        let error = copy(&folders, || true).unwrap_err();

        assert!(matches!(error, CopyError::EarlierRunning));
        assert!(absent(&folders.current) && absent(&folders.staging()));
        assert_eq!(snapshot(&folders.earlier), before);
        let failure = crate::startup_failure::message(&error);
        assert_eq!(failure.title, "XTrace Desktop couldn't start");
        assert_eq!(
            failure.body,
            "The earlier XTrace Desktop is still running. Quit it, then open XTrace Desktop \
             again so it can bring over your data."
        );
        assert_eq!(failure.database, None);
    }

    #[test]
    fn a_leftover_staging_folder_is_removed_and_the_copy_retried() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 77);
        fs::create_dir_all(folders.staging().join("half")).unwrap();
        fs::write(folders.staging().join("junk"), b"x").unwrap();
        fs::write(folders.staging().join(DATABASE_FILE), b"half a database").unwrap();

        copy(&folders, || false).unwrap();

        assert!(absent(&folders.staging()));
        assert!(absent(&folders.current.join("junk")));
        assert!(absent(&folders.current.join("half")));
        assert_eq!(typing_speed(&folders.current.join(DATABASE_FILE)), 77);
    }

    #[test]
    fn an_existing_folder_without_a_database_keeps_its_files_and_gains_the_copy() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 61);
        fs::create_dir_all(&folders.current).unwrap();
        fs::write(folders.current.join(HISTORY), b"already here").unwrap();
        fs::write(folders.current.join("xtrace.db-wal"), b"stale log").unwrap();

        copy(&folders, || false).unwrap();

        assert_eq!(
            fs::read(folders.current.join(HISTORY)).unwrap(),
            b"already here"
        );
        assert_eq!(
            fs::read(folders.current.join(LAST_READING)).unwrap(),
            b"{\"reading\":1}"
        );
        assert!(folders.current.join(MARKER_FILE).is_file());
        assert!(absent(&folders.staging()));
        // The stale log was removed, not paired with the copied database.
        assert!(absent(&folders.current.join("xtrace.db-wal")));
        assert_eq!(typing_speed(&folders.current.join(DATABASE_FILE)), 61);
    }

    #[test]
    fn a_failed_move_into_an_existing_folder_is_retried_on_the_next_start() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 48);
        fs::create_dir_all(&folders.current).unwrap();
        fs::write(folders.current.join(HISTORY), b"already here").unwrap();
        let before = snapshot(&folders.earlier);
        // Nothing can be moved into the folder.
        fs::set_permissions(&folders.current, fs::Permissions::from_mode(0o500)).unwrap();

        let error = copy(&folders, || false).unwrap_err();

        fs::set_permissions(&folders.current, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(error, CopyError::Copy(_)), "{error:?}");
        assert!(absent(&folders.current.join(DATABASE_FILE)));
        assert!(absent(&folders.current.join(MARKER_FILE)));
        assert!(absent(&folders.staging()));
        assert_eq!(snapshot(&folders.earlier), before);
        assert_eq!(
            decision(&options(), &folders.current),
            Decision::Copy(folders.clone()),
            "the next start copies again"
        );

        copy(&folders, || false).unwrap();

        assert_eq!(
            fs::read(folders.current.join(HISTORY)).unwrap(),
            b"already here"
        );
        assert!(folders.current.join(MARKER_FILE).is_file());
        assert_eq!(typing_speed(&folders.current.join(DATABASE_FILE)), 48);
    }

    #[test]
    fn a_failed_copy_leaves_no_database_and_the_earlier_folder_unchanged() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        earlier(&folders, 55);
        let unreadable = folders.earlier.join(LAST_READING);
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        let before = snapshot_without(&folders.earlier, &unreadable);

        let error = copy(&folders, || false).unwrap_err();

        assert!(matches!(error, CopyError::Copy(_)), "{error:?}");
        assert!(absent(&folders.current) && absent(&folders.staging()));
        assert_eq!(snapshot_without(&folders.earlier, &unreadable), before);
        let failure = crate::startup_failure::message(&error);
        assert!(
            failure.body.starts_with(
                "XTrace could not copy its data from the earlier install, and left the \
                 earlier folder unchanged: "
            ),
            "{}",
            failure.body
        );
        // Readable again, the next start copies.
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
        copy(&folders, || false).unwrap();
        assert_eq!(typing_speed(&folders.current.join(DATABASE_FILE)), 55);
    }

    #[test]
    fn a_copied_database_that_does_not_open_is_not_put_in_place() {
        let root = tempfile::TempDir::new().unwrap();
        let folders = folders(root.path());
        fs::create_dir_all(&folders.earlier).unwrap();
        let db = folders.earlier.join(DATABASE_FILE);
        fs::write(&db, vec![7u8; 8192]).unwrap();
        fs::write(folders.earlier.join(HISTORY), b"{}\n").unwrap();
        let before = snapshot(&folders.earlier);

        let error = copy(&folders, || false).unwrap_err();

        assert!(matches!(error, CopyError::Database(_)), "{error:?}");
        assert!(absent(&folders.current) && absent(&folders.staging()));
        assert_eq!(snapshot(&folders.earlier), before);
        // Its own words: the earlier data is unchanged, nothing was deleted,
        // and nothing tells the person to move the earlier files away.
        let failure = crate::startup_failure::message(&error);
        assert_eq!(failure.title, "XTrace Desktop couldn't start");
        assert_eq!(failure.database, None);
        assert!(
            failure.body.starts_with(
                "XTrace couldn't bring over the data from the earlier version. The earlier \
                 data is unchanged and nothing was deleted.\n\nDetails: SQLite operation \
                 failed: "
            ),
            "{}",
            failure.body
        );
        assert!(!failure.body.contains("move"), "{}", failure.body);
    }

    #[test]
    fn screen_settings_replace_whatever_the_new_webkit_folder_held() {
        let home = tempfile::TempDir::new().unwrap();
        let webkit = home.path().join("Library/WebKit");
        let current = webkit.join(CURRENT);
        let staging = sibling(&current, STAGING_SUFFIX);
        // A leftover from a failed attempt is removed even with nothing to
        // copy.
        fs::create_dir_all(staging.join("half")).unwrap();
        assert!(
            !copy_screen_settings(home.path(), CURRENT).unwrap(),
            "none to copy"
        );
        assert!(absent(&staging));
        let earlier = webkit.join(EARLIER_IDENTIFIER).join("WebsiteData/Default");
        fs::create_dir_all(&earlier).unwrap();
        fs::write(earlier.join("salt"), b"salt").unwrap();
        fs::write(earlier.join("localstorage.sqlite3"), b"theme=dark").unwrap();
        let before = snapshot(&webkit.join(EARLIER_IDENTIFIER));
        assert!(
            !copy_screen_settings(home.path(), EARLIER_IDENTIFIER).unwrap(),
            "the earlier ID copies nothing"
        );

        // Into no folder yet.
        assert!(copy_screen_settings(home.path(), CURRENT).unwrap());
        assert_eq!(snapshot(&current), before);

        // Over a folder a test launch of the new ID left, with a leftover
        // staging folder: the earlier settings replace it entirely.
        fs::write(current.join("WebsiteData/Default/salt"), b"test launch").unwrap();
        fs::write(current.join("from-a-test-launch"), b"x").unwrap();
        fs::create_dir_all(staging.join("half")).unwrap();
        assert!(copy_screen_settings(home.path(), CURRENT).unwrap());
        assert_eq!(snapshot(&current), before);
        assert_eq!(snapshot(&webkit.join(EARLIER_IDENTIFIER)), before);
        assert!(absent(&staging));
        assert!(absent(&sibling(&current, REPLACED_SUFFIX)));
    }

    #[test]
    fn a_failed_screen_settings_copy_keeps_the_folder_it_would_replace() {
        let home = tempfile::TempDir::new().unwrap();
        let webkit = home.path().join("Library/WebKit");
        let current = webkit.join(CURRENT);
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("kept"), b"kept").unwrap();
        let earlier = webkit.join(EARLIER_IDENTIFIER);
        fs::create_dir_all(&earlier).unwrap();
        fs::write(earlier.join("unreadable"), b"x").unwrap();
        fs::set_permissions(
            earlier.join("unreadable"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();

        assert!(copy_screen_settings(home.path(), CURRENT).is_err());

        fs::set_permissions(
            earlier.join("unreadable"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(fs::read(current.join("kept")).unwrap(), b"kept");
        assert!(absent(&sibling(&current, STAGING_SUFFIX)));
        assert!(absent(&sibling(&current, REPLACED_SUFFIX)));
    }

    /// The copy relies on Tauri's order: plugins are set up while the app is
    /// built, before the windows in the configuration exist, and `setup`
    /// runs after they do. A test plugin records it on the mock runtime; the
    /// real copy is never run here, since it would look at the real home.
    #[test]
    fn plugins_are_set_up_before_the_configured_windows_exist() {
        use std::sync::{Arc, Mutex, mpsc};
        use tauri::{
            Manager, RunEvent,
            test::{mock_builder, mock_context, noop_assets},
        };

        let (sent, outcome) = mpsc::channel();
        // On its own thread: a run loop that never ends is reported, not
        // waited on.
        std::thread::spawn(move || {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mut context = mock_context(noop_assets());
            context
                .config_mut()
                .app
                .windows
                .push(tauri::utils::config::WindowConfig {
                    label: crate::tray::MAIN_LABEL.into(),
                    ..Default::default()
                });
            let plugin = tauri::plugin::Builder::<tauri::test::MockRuntime>::new("order-probe")
                .setup({
                    let seen = Arc::clone(&seen);
                    move |app, _| {
                        seen.lock()
                            .unwrap()
                            .push(("plugin", app.webview_windows().len()));
                        Ok(())
                    }
                })
                .build();
            let app = mock_builder()
                .plugin(plugin)
                .setup({
                    let seen = Arc::clone(&seen);
                    move |app| {
                        seen.lock()
                            .unwrap()
                            .push(("setup", app.webview_windows().len()));
                        Ok(())
                    }
                })
                .build(context)
                .unwrap();
            app.run(|handle, event| {
                if let RunEvent::Ready = event {
                    for window in handle.webview_windows().into_values() {
                        let _ = window.destroy();
                    }
                }
            });
            let _ = sent.send(seen.lock().unwrap().clone());
        });
        let seen = outcome
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the run loop ends");
        assert_eq!(seen, [("plugin", 0), ("setup", 1)]);
    }

    /// [`snapshot`] of a folder holding one file the test made unreadable,
    /// recorded by its permissions only.
    fn snapshot_without(root: &Path, unreadable: &Path) -> (Snapshot, u32) {
        let mode = fs::metadata(unreadable).unwrap().permissions().mode();
        fs::set_permissions(unreadable, fs::Permissions::from_mode(0o600)).unwrap();
        let entries = snapshot(root);
        fs::set_permissions(unreadable, fs::Permissions::from_mode(mode)).unwrap();
        (entries, mode)
    }
}
