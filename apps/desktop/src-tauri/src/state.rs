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
    #[error("metric query failed")]
    Metrics(#[from] xt_metrics::Error),
    #[error("metric response could not be represented safely")]
    MetricEncoding,
    #[error("metric range must be 7, 14, or 30 days")]
    InvalidMetricWindow,
    #[error("system time zone is unavailable")]
    MetricTimezone,
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

enum MetricContext {
    System,
    #[cfg(all(debug_assertions, feature = "fixtures"))]
    Fixture {
        now_ms: i64,
        catalog: xt_metrics::PriceCatalog,
        /// The shared fixture probe over F16's synthetic matrix, read once.
        probe: Box<xt_probes::EnvironmentProbe>,
    },
}

/// One command's clock, zone, catalog and read sources, fixed under the lock.
struct MetricInputs<'a> {
    now: i64,
    zone: jiff::tz::TimeZone,
    clock: crate::dto::MetricClock,
    catalog: Option<&'a xt_metrics::PriceCatalog>,
    store: &'a Store,
    fixture_probe: Option<&'a xt_probes::EnvironmentProbe>,
}

struct Database {
    store: Store,
    metrics_path: PathBuf,
    metric_context: MetricContext,
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
            MetricContext::System,
        )
    }
    fn from_store(
        store: Store,
        data_dir: PathBuf,
        fixture: Option<String>,
        directory: Option<tempfile::TempDir>,
        db_path: Option<PathBuf>,
        native_home: Option<PathBuf>,
        metric_context: MetricContext,
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
                metrics_path: data_dir.join("xtrace.db"),
                metric_context,
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
        let metric_context = MetricContext::Fixture {
            now_ms: fixture.now().timestamp_millis(),
            catalog: crate::dashboard::fixture_catalog(fixture.snapshots().get("prices"))?,
            probe: Box::new(crate::environment::fixture_probe(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures"),
            )?),
        };
        Self::from_store(
            store,
            directory.path().to_owned(),
            Some(id),
            Some(directory),
            None,
            None,
            metric_context,
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

    pub fn sessions_list(
        &self,
        search: &str,
        host: Option<&str>,
        after: Option<&str>,
    ) -> Result<crate::dto::SessionPage, StateError> {
        let guard = self.database.lock().map_err(|_| StateError::Poisoned)?;
        crate::dto::session_page(
            &guard.as_ref().ok_or(StateError::Closed)?.store,
            search,
            host,
            after,
        )
        .map_err(Into::into)
    }

    /// Reads run under the state lock, so shutdown cannot remove a fixture's
    /// temporary database while a metric connection is using it.
    fn with_metrics<T>(
        &self,
        window_days: u32,
        read: impl FnOnce(&xt_metrics::MetricsDb, MetricInputs<'_>) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        crate::dashboard::validate_window(window_days)?;
        let guard = self.database.lock().map_err(|_| StateError::Poisoned)?;
        let database = guard.as_ref().ok_or(StateError::Closed)?;
        let inputs = match &database.metric_context {
            MetricContext::System => MetricInputs {
                now: jiff::Timestamp::now().as_millisecond(),
                zone: jiff::tz::TimeZone::try_system().map_err(|_| StateError::MetricTimezone)?,
                clock: crate::dto::MetricClock::System,
                catalog: None,
                store: &database.store,
                fixture_probe: None,
            },
            #[cfg(all(debug_assertions, feature = "fixtures"))]
            MetricContext::Fixture {
                now_ms,
                catalog,
                probe,
            } => MetricInputs {
                now: *now_ms,
                zone: jiff::tz::TimeZone::UTC,
                clock: crate::dto::MetricClock::Fixture,
                catalog: Some(catalog),
                store: &database.store,
                fixture_probe: Some(probe),
            },
        };
        let metrics = xt_metrics::MetricsDb::open(&database.metrics_path)?;
        read(&metrics, inputs)
    }

    pub fn metrics_dashboard(
        &self,
        window_days: u32,
    ) -> Result<crate::dto::DashboardMetrics, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            let bundled;
            let catalog = match inputs.catalog {
                Some(catalog) => catalog,
                None => {
                    bundled = xt_metrics::PriceCatalog::bundled()?;
                    &bundled
                }
            };
            crate::dashboard::assemble(
                metrics,
                window_days,
                inputs.now,
                inputs.zone,
                inputs.clock,
                catalog,
            )
        })
    }

    pub fn tokens_by_host(&self, window_days: u32) -> Result<crate::dto::TokensByHost, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            crate::dashboard::tokens_by_host(
                metrics,
                window_days,
                inputs.now,
                inputs.zone,
                inputs.clock,
            )
        })
    }

    /// M-17 for the selected range and the fixed 14-date strip, with an
    /// unknown inventory, beside the configured-component probe. Native roots
    /// are the native home option and local repository paths from stored
    /// session metadata; fixture mode uses the shared fixture probe.
    pub fn metrics_environment(
        &self,
        window_days: u32,
    ) -> Result<crate::dto::EnvironmentMetrics, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            let native;
            let probe = match inputs.fixture_probe {
                Some(probe) => probe,
                None => {
                    let stored: Vec<Option<String>> = inputs
                        .store
                        .sessions_page("", None, None)?
                        .into_iter()
                        .map(|session| session.repo)
                        .collect();
                    native = xt_probes::probe(&crate::environment::native_roots(
                        self.native_home.as_deref(),
                        &stored,
                    ));
                    &native
                }
            };
            crate::environment::assemble(
                metrics,
                window_days,
                inputs.now,
                inputs.zone,
                inputs.clock,
                probe,
            )
        })
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
