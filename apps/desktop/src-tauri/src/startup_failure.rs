//! What the app says when it cannot start. Launched from Finder, a failed
//! start would otherwise end with nothing on screen. One function decides the
//! text from the startup error; the platform code only shows it and exits.
use crate::state::StateError;
use std::{
    error::Error,
    path::{Path, PathBuf},
};

/// The alert a failed start shows. `database` is the data file the failure
/// concerns, when it concerns one, so the alert can show it in Finder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailure {
    pub title: String,
    pub body: String,
    pub database: Option<PathBuf>,
}

const DATA_TITLE: &str = "XTrace can't open its data";
const START_TITLE: &str = "XTrace Desktop couldn't start";

/// Decides the alert for a startup error. It looks through the whole chain of
/// sources for the database the store refused: one whose migration history
/// differs from this build's, or one a newer build wrote. Anything else shows
/// every message in the chain.
pub fn message(error: &(dyn Error + 'static)) -> StartupFailure {
    // A copy of the earlier version's database that did not open says so in
    // its own words: the earlier file was not opened, so nothing about it
    // needs moving, and nothing was deleted.
    if let Some(copy @ crate::earlier_install::CopyError::Database(source)) =
        chain(error).find_map(|error| error.downcast_ref::<crate::earlier_install::CopyError>())
    {
        return StartupFailure {
            title: START_TITLE.into(),
            body: format!("{copy}\n\nDetails: {}", details(source)),
            database: None,
        };
    }
    let opened = chain(error).find_map(|error| match error.downcast_ref::<StateError>() {
        Some(StateError::OpenDatabase { path, source }) => Some((path, source)),
        _ => None,
    });
    match opened {
        Some((path, xt_store::Error::MigrationHistory(detail))) => {
            let name = file_name(path);
            StartupFailure {
                title: DATA_TITLE.into(),
                body: format!(
                    "This data file was changed by a different version of XTrace, so this \
                     version left it untouched.\n\n\
                     To keep using this version, quit XTrace and move every file whose name \
                     starts with {name} out of its folder. XTrace then builds a new one from \
                     the Claude Code, Codex and Cursor history still on this Mac. Settings \
                     saved in this file, including what content to keep, go back to their \
                     defaults, and conversations whose history files are gone are not \
                     rebuilt.\n\n\
                     Data file: {}\n\n\
                     Details: {detail}",
                    path.display()
                ),
                database: Some(path.clone()),
            }
        }
        Some((path, xt_store::Error::IncompatibleSchema)) => StartupFailure {
            title: DATA_TITLE.into(),
            body: format!(
                "This data file was written by a newer version of XTrace. Install the newer \
                 version to open it. This version left the file untouched.\n\n\
                 Data file: {}",
                path.display()
            ),
            database: Some(path.clone()),
        },
        Some((path, _)) => StartupFailure {
            title: START_TITLE.into(),
            body: format!("{}\n\nData file: {}", details(error), path.display()),
            database: Some(path.clone()),
        },
        None => StartupFailure {
            title: START_TITLE.into(),
            body: details(error),
            database: None,
        },
    }
}

/// Shows `failure`, reveals the data file in Finder when asked, and ends the
/// process with a non-zero status. The text is also written to standard
/// error, where a launch from a terminal shows it.
pub fn exit(failure: &StartupFailure) -> ! {
    eprintln!("{}\n\n{}", failure.title, failure.body);
    #[cfg(target_os = "macos")]
    if alert(failure) == Choice::ShowInFinder
        && let Some(path) = &failure.database
    {
        // Finder selects the file in its folder; the app quits either way.
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .status();
    }
    std::process::exit(1)
}

#[cfg(target_os = "macos")]
#[derive(PartialEq, Eq)]
enum Choice {
    Quit,
    ShowInFinder,
}

/// The native alert: "Quit" is the first button, so Return picks it, and
/// "Show in Finder" is offered when the failure names a data file. AppKit
/// runs only on the main thread, where setup and `build` run; anywhere else
/// the message reaches only standard error.
#[cfg(target_os = "macos")]
fn alert(failure: &StartupFailure) -> Choice {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication,
        NSApplicationActivationPolicy,
    };
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return Choice::Quit;
    };
    // Launched from Finder there may be no window yet: the app comes to the
    // front so the alert is not left behind other apps.
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str(&failure.title));
    alert.setInformativeText(&NSString::from_str(&failure.body));
    alert.addButtonWithTitle(&NSString::from_str("Quit"));
    if failure.database.is_some() {
        alert.addButtonWithTitle(&NSString::from_str("Show in Finder"));
    }
    if alert.runModal() == NSAlertFirstButtonReturn {
        Choice::Quit
    } else {
        Choice::ShowInFinder
    }
}

fn chain<'a>(error: &'a (dyn Error + 'static)) -> impl Iterator<Item = &'a (dyn Error + 'static)> {
    std::iter::successors(Some(error), |&error| error.source())
}

/// Every message in the chain, outermost first. A source whose text an outer
/// message already includes (an error that prints its source) is not repeated.
fn details(error: &(dyn Error + 'static)) -> String {
    let mut text = String::new();
    for message in chain(error).map(ToString::to_string) {
        if text.is_empty() {
            text = message;
        } else if !text.contains(&message) {
            text = format!("{text}: {message}");
        }
    }
    text
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, StartupOptions};

    /// Starts the app's state over `root/data` the way setup does for a
    /// custom data directory, and returns the error boxed as setup's `?`
    /// boxes it.
    fn start(root: &Path) -> Box<dyn Error> {
        std::fs::create_dir_all(root.join("home")).unwrap();
        let error = AppState::build(
            StartupOptions {
                data_dir: Some(root.join("data")),
                native_home: Some(root.join("home")),
                ..Default::default()
            },
            || panic!("default path must not be resolved"),
            || panic!("default home must not be resolved"),
        )
        .err()
        .expect("startup must fail");
        error.into()
    }

    /// A database this build creates, then changed with `sql` so that it is
    /// refused when opened again.
    fn database(root: &Path, sql: &str) -> PathBuf {
        let path = root.join("data").join("xtrace.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        drop(xt_store::Store::open(&path).unwrap());
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(sql)
            .unwrap();
        path
    }

    #[test]
    fn a_database_from_a_different_build_names_the_file_and_what_differs() {
        let root = tempfile::TempDir::new().unwrap();
        let path = database(
            root.path(),
            &format!(
                "UPDATE migration_fingerprints SET sha256 = '{}' WHERE version = 1",
                "0".repeat(64)
            ),
        );
        let before = std::fs::read(&path).unwrap();
        let error = start(root.path());
        let Some(StateError::OpenDatabase {
            source: xt_store::Error::MigrationHistory(detail),
            ..
        }) = error.downcast_ref::<StateError>()
        else {
            panic!("the store must refuse the history: {error:?}");
        };
        assert!(
            detail.starts_with(
                "version 1 was applied from a different 0001_canonical than this build's \
                 (fingerprint 000000000000 recorded, "
            ),
            "{detail}"
        );
        let failure = message(error.as_ref());
        assert_eq!(failure.title, "XTrace can't open its data");
        assert_eq!(failure.database.as_deref(), Some(path.as_path()));
        assert_eq!(
            failure.body,
            format!(
                "This data file was changed by a different version of XTrace, so this version \
                 left it untouched.\n\n\
                 To keep using this version, quit XTrace and move every file whose name starts \
                 with xtrace.db out of its folder. XTrace then builds a new one from the Claude \
                 Code, Codex and Cursor history still on this Mac. Settings saved in this \
                 file, including what content to keep, go back to their defaults, and \
                 conversations whose history files are gone are not rebuilt.\n\n\
                 Data file: {}\n\n\
                 Details: {detail}",
                path.display(),
            )
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the file is unchanged"
        );
    }

    #[test]
    fn a_database_from_a_newer_build_asks_for_the_newer_version() {
        let root = tempfile::TempDir::new().unwrap();
        let path = database(
            root.path(),
            "INSERT INTO schema_version(version, applied_at) VALUES (9999, 'later')",
        );
        let failure = message(start(root.path()).as_ref());
        assert_eq!(
            failure,
            StartupFailure {
                title: "XTrace can't open its data".into(),
                body: format!(
                    "This data file was written by a newer version of XTrace. Install the \
                     newer version to open it. This version left the file untouched.\n\n\
                     Data file: {}",
                    path.display()
                ),
                database: Some(path),
            }
        );
    }

    #[test]
    fn any_other_failure_shows_every_message_in_the_chain() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("data").join("xtrace.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![b'x'; 4096]).unwrap();
        let failure = message(start(root.path()).as_ref());
        assert_eq!(failure.title, "XTrace Desktop couldn't start");
        assert_eq!(failure.database.as_deref(), Some(path.as_path()));
        // SQLite's own message is not repeated after the store's, which
        // prints it; the code below it is a different message and is shown.
        assert_eq!(
            failure.body,
            format!(
                "application database could not be opened: SQLite operation failed: file is not \
                 a database: Error code 26: file is not a database\n\n\
                 Data file: {}",
                path.display()
            )
        );

        let unopened: Box<dyn Error> = StateError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "no access",
        ))
        .into();
        assert_eq!(
            message(unopened.as_ref()),
            StartupFailure {
                title: "XTrace Desktop couldn't start".into(),
                body: "application data directory is unavailable: no access".into(),
                database: None,
            }
        );
    }

    /// Why setup decides the alert itself: once Tauri wraps a setup error, its
    /// source is no longer reachable, so the refused database could not be
    /// recognised from it.
    #[test]
    fn a_setup_error_inside_tauri_hides_its_source() {
        let root = tempfile::TempDir::new().unwrap();
        database(
            root.path(),
            "INSERT INTO schema_version(version, applied_at) VALUES (9999, 'later')",
        );
        let wrapped = tauri::Error::Setup(start(root.path()).into());
        let failure = message(&wrapped);
        assert_eq!(failure.title, "XTrace Desktop couldn't start");
        assert_eq!(failure.database, None);
    }
}
