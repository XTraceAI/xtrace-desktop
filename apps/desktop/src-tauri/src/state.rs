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
    #[error("database count exceeds the exact JSON integer range")]
    CountRange,
}

#[derive(Default)]
pub struct StartupOptions {
    pub data_dir: Option<PathBuf>,
    pub fixture: Option<String>,
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
        Ok(Self { data_dir, fixture })
    }
}

pub struct AppState {
    store: Mutex<Store>,
    info: AppInfo,
    // Declared last: SQLite closes before the temporary directory is removed.
    _fixture_directory: Option<tempfile::TempDir>,
}

impl AppState {
    /// The live default path is resolved lazily, so fixture mode never even asks
    /// the host for its application-data directory.
    pub fn build(
        options: StartupOptions,
        default_dir: impl FnOnce() -> Result<PathBuf, StateError>,
    ) -> Result<Self, StateError> {
        if let Some(id) = options.fixture {
            return Self::fixture(id, options.data_dir);
        }
        let data_dir = match options.data_dir {
            Some(path) => path,
            None => default_dir()?,
        };
        std::fs::create_dir_all(&data_dir)?;
        let store = Store::open(data_dir.join("xtrace.db"))?;
        Self::from_store(store, data_dir, None, None)
    }
    fn from_store(
        store: Store,
        data_dir: PathBuf,
        fixture: Option<String>,
        directory: Option<tempfile::TempDir>,
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
            store: Mutex::new(store),
            info,
            _fixture_directory: directory,
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
        )
    }
    #[cfg(not(all(debug_assertions, feature = "fixtures")))]
    fn fixture(_: String, _: Option<PathBuf>) -> Result<Self, StateError> {
        Err(StateError::FixtureDisabled)
    }
    pub fn app_info(&self) -> AppInfo {
        self.info.clone()
    }
    pub fn db_counts(&self) -> Result<DbCounts, StateError> {
        self.store
            .lock()
            .map_err(|_| StateError::Poisoned)?
            .counts()?
            .try_into()
            .map_err(|_| StateError::CountRange)
    }
}
