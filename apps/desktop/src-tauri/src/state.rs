//! One database owner per application. Fixture startup never selects live data.
use crate::dto::{AppInfo, DbCounts};
use std::{path::PathBuf, sync::Mutex};
use xt_store::Store;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("application storage operation failed")]
    Store(#[from] xt_store::Error),
    #[error("application data directory is unavailable")]
    Io(#[from] std::io::Error),
    #[error("fixture mode requires a debug build with the fixtures feature")]
    FixtureDisabled,
    #[error("fixture identifier or fixture data is unavailable")]
    FixtureInvalid,
    #[error("invalid startup option")]
    InvalidOption,
    #[error("application database is unavailable")]
    Poisoned,
    #[error("application database is closed")]
    Closed,
    #[error("database count exceeds the exact JSON integer range")]
    CountRange,
    /// The data directory would put the database inside the native history it
    /// indexes (or aliased to it); nothing is created there.
    #[error("data directory cannot hold the index: {0}")]
    IndexDestination(&'static str),
    /// The native home must exist: the watcher covers absent roots through
    /// their nearest existing ancestor, and the home is the last of those.
    #[error("native home is not an existing directory")]
    NativeHome,
}

#[derive(Default)]
pub struct StartupOptions {
    pub data_dir: Option<PathBuf>,
    pub fixture: Option<String>,
    /// The home the native index reads (`XTRACE_NATIVE_HOME`, an existing
    /// directory); the user's home otherwise.
    pub native_home: Option<PathBuf>,
    /// The interpreter for the Codex/Cursor readers (`XTRACE_PYTHON`);
    /// discovered otherwise.
    pub python: Option<std::ffi::OsString>,
}
impl StartupOptions {
    /// Arguments take precedence over the environment; duplicate flags fail.
    pub fn parse(
        data_dir: Option<PathBuf>,
        fixture: Option<std::ffi::OsString>,
        args: impl IntoIterator<Item = String>,
    ) -> Result<Self, StateError> {
        let fixture = fixture
            .map(|value| value.into_string().map_err(|_| StateError::InvalidOption))
            .transpose()?;
        let mut cli_fixture = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--fixture" {
                if cli_fixture.is_some() {
                    return Err(StateError::InvalidOption);
                }
                cli_fixture = Some(
                    args.next()
                        .filter(|v| !v.is_empty() && !v.starts_with('-'))
                        .ok_or(StateError::InvalidOption)?,
                );
            } else if let Some(value) = arg.strip_prefix("--fixture=") {
                if cli_fixture.is_some() || value.is_empty() {
                    return Err(StateError::InvalidOption);
                }
                cli_fixture = Some(value.to_owned());
            }
        }
        let fixture = cli_fixture.or(fixture);
        if fixture.as_ref().is_some_and(|v| v.is_empty())
            || data_dir.as_ref().is_some_and(|p| p.as_os_str().is_empty())
        {
            return Err(StateError::InvalidOption);
        }
        Ok(Self {
            data_dir,
            fixture,
            native_home: None,
            python: None,
        })
    }

    /// Read the native index overrides from the environment; an empty value
    /// is invalid, like the other options.
    pub fn with_native_environment(
        mut self,
        native_home: Option<std::ffi::OsString>,
        python: Option<std::ffi::OsString>,
    ) -> Result<Self, StateError> {
        if native_home.as_ref().is_some_and(|v| v.is_empty())
            || python.as_ref().is_some_and(|v| v.is_empty())
        {
            return Err(StateError::InvalidOption);
        }
        self.native_home = native_home.map(PathBuf::from);
        self.python = python;
        Ok(self)
    }
}

pub struct AppState {
    database: Mutex<Option<Database>>,
    info: AppInfo,
    /// The live database file; none in fixture mode, whose database is disposable.
    db_path: Option<PathBuf>,
    /// The home the native index reads; none in fixture mode.
    native_home: Option<PathBuf>,
}

struct Database {
    store: Store,
    // Declared last: SQLite closes before the temporary directory is removed.
    _fixture_directory: Option<tempfile::TempDir>,
}

impl AppState {
    /// The live default paths are resolved lazily, so fixture mode never even
    /// asks the host for its application-data or home directory. The database
    /// destination is validated against the native home before anything is
    /// created or opened there: a data directory inside the native history, or
    /// aliased to it, is refused with nothing written.
    pub fn build(
        options: StartupOptions,
        default_dir: impl FnOnce() -> Result<PathBuf, StateError>,
        default_home: impl FnOnce() -> Result<PathBuf, StateError>,
    ) -> Result<Self, StateError> {
        if let Some(id) = options.fixture {
            return Self::fixture(id, options.data_dir);
        }
        let data_dir = match options.data_dir {
            Some(path) => path,
            None => default_dir()?,
        };
        let native_home = match options.native_home {
            Some(path) => path,
            None => default_home()?,
        };
        if !native_home.is_dir() {
            return Err(StateError::NativeHome);
        }
        let db_path = data_dir.join("xtrace.db");
        xt_ingest::native::validate_index_destination(&db_path, &native_home)
            .map_err(StateError::IndexDestination)?;
        std::fs::create_dir_all(&data_dir)?;
        let store = Store::open(&db_path)?;
        Self::from_store(
            store,
            data_dir,
            None,
            None,
            Some(db_path),
            Some(native_home),
        )
    }
    fn from_store(
        store: Store,
        data_dir: PathBuf,
        fixture: Option<String>,
        directory: Option<tempfile::TempDir>,
        db_path: Option<PathBuf>,
        native_home: Option<PathBuf>,
    ) -> Result<Self, StateError> {
        let info = AppInfo {
            name: "XTrace Desktop".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            data_dir: data_dir.to_str().ok_or(StateError::InvalidOption)?.into(),
            fixture,
            schema_version: store.schema_version()?,
            listening: false,
        };
        Ok(Self {
            database: Mutex::new(Some(Database {
                store,
                _fixture_directory: directory,
            })),
            info,
            db_path,
            native_home,
        })
    }
    #[cfg(all(debug_assertions, feature = "fixtures"))]
    fn fixture(id: String, parent: Option<PathBuf>) -> Result<Self, StateError> {
        let parsed = xt_fixtures::FixtureId::parse(&id).map_err(|_| StateError::FixtureInvalid)?;
        let fixture = xt_fixtures::Fixture::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../fixtures")
                .join(parsed.to_string()),
        )
        .map_err(|_| StateError::FixtureInvalid)?;
        // No existing database is reused, including under an explicit override.
        let directory = match parent {
            Some(parent) => {
                std::fs::create_dir_all(&parent)?;
                tempfile::TempDir::new_in(parent)?
            }
            None => tempfile::TempDir::new()?,
        };
        let path = directory.path().join("xtrace.db");
        fixture
            .write_db(&path, true)
            .map_err(|_| StateError::FixtureInvalid)?;
        let store = Store::open(path)?;
        Self::from_store(
            store,
            directory.path().to_owned(),
            Some(id),
            Some(directory),
            None,
            None,
        )
    }
    #[cfg(not(all(debug_assertions, feature = "fixtures")))]
    fn fixture(_: String, _: Option<PathBuf>) -> Result<Self, StateError> {
        Err(StateError::FixtureDisabled)
    }
    pub fn app_info(&self) -> AppInfo {
        self.info.clone()
    }
    /// The live database the native index writes to; none in fixture mode.
    pub fn database_path(&self) -> Option<&std::path::Path> {
        self.db_path.as_deref()
    }
    /// The home the native index reads, validated against the database
    /// destination; none in fixture mode.
    pub fn native_home(&self) -> Option<&std::path::Path> {
        self.native_home.as_deref()
    }
    pub fn db_counts(&self) -> Result<DbCounts, StateError> {
        self.database
            .lock()
            .map_err(|_| StateError::Poisoned)?
            .as_ref()
            .ok_or(StateError::Closed)?
            .store
            .counts()?
            .try_into()
            .map_err(|_| StateError::CountRange)
    }

    /// Tauri exits the process without dropping managed state. Close resources
    /// explicitly on its Exit event, serialized with any in-flight database read.
    pub fn shutdown(&self) {
        self.database
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}
