//! One database owner per application. Fixture startup never selects live data.
use crate::dto::{AppInfo, DbCounts, PrList, PrRow, PrSkipReason};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};
use xt_store::{
    Store,
    pr_link::{LinkedPullRequest, RefreshOrigin, RefreshOutcome, RefreshWrite},
};

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("application storage operation failed")]
    Store(#[from] xt_store::Error),
    /// Startup could not open the database file, or the store refused it as
    /// written by another build and left it unchanged. The path is kept for
    /// the startup alert, never shown by the Display.
    #[error("application database could not be opened")]
    OpenDatabase {
        path: PathBuf,
        #[source]
        source: xt_store::Error,
    },
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
    /// Two metrics read in one snapshot to be shown side by side did not
    /// describe the same things, so neither is shown beside the other.
    #[error("metric reports read together did not agree")]
    MetricDisagreement,
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
    /// A session's source could not be represented on the wire. The detail
    /// names a kind, never a path or a fragment of what the session said.
    #[error("session source response could not be represented safely: {0}")]
    SourceEncoding(&'static str),
    /// Too many transcript reads are running at once, or this one is already.
    #[error("transcript read was refused")]
    ReadRefused,
    /// Cancelled or out of time before storage was counted, so no honest
    /// count of what was requested exists.
    #[error("the read stopped before it started")]
    ReadStopped,
    /// Refused before storage is touched.
    #[error("a title read names at most 50 distinct sessions")]
    InvalidTitleRequest,
    #[error("a compaction read names at most 50 distinct sessions")]
    InvalidCompactionRequest,
    /// Refused before storage is touched: no lane could have sent it.
    #[error("a span detail request names one session and one span of the lane window")]
    InvalidSpanRequest,
    /// Refused before storage is touched.
    #[error("typing speed must be a whole number from 1 to 300 words per minute")]
    InvalidTypingSpeed,
    /// Refused before storage is touched.
    #[error("break length must be a whole number from 5 to 240 minutes")]
    InvalidHumanBreak,
    #[error("the typing test could not be opened in the browser")]
    TypingTestUnavailable,
    #[error("public releases could not be opened in the browser")]
    PublicReleasesUnavailable,
    /// A linked-session request named something no report could have shown:
    /// the detail names the part, never the value.
    #[error("invalid pull-request session request: {0}")]
    InvalidPrSessions(&'static str),
    /// A stored pull-request value cannot cross IPC exactly.
    #[error("{0}")]
    PrEncoding(&'static str),
    #[error("{0}")]
    LiveStatus(&'static str),
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
    /// The GitHub CLI executable a manual pull-request refresh runs
    /// (`XTRACE_GH`, an absolute path); resolved otherwise. Backend only:
    /// the frontend cannot name an executable.
    pub github_cli: Option<std::ffi::OsString>,
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
            ..Default::default()
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

    /// Read the backend-only GitHub CLI override from the environment; an
    /// empty value is invalid, like the other options. The path itself is
    /// validated where it is resolved, not here.
    pub fn with_github_cli(
        mut self,
        github_cli: Option<std::ffi::OsString>,
    ) -> Result<Self, StateError> {
        if github_cli.as_ref().is_some_and(|value| value.is_empty()) {
            return Err(StateError::InvalidOption);
        }
        self.github_cli = github_cli;
        Ok(self)
    }
}

pub struct AppState {
    database: Mutex<Option<Database>>,
    /// Live status never queues behind Dashboard work or another identity read.
    /// Open lazily on the exact live DB; fixture state never opens this reader.
    live_identity: Mutex<Option<xt_store::identity_reader::SessionIdentityReader>>,
    /// Set once by shutdown before it waits for the database lock; never
    /// cleared. A caller queued on the lock behind the current holder then
    /// sees the database closed instead of taking the lock ahead of shutdown.
    closing: AtomicBool,
    /// Metric reads in flight. Each takes a shared guard while it still holds
    /// the database lock and keeps it, without that lock, for its whole read
    /// on its own read-only connection; shutdown takes it exclusively, after
    /// the database lock, so it waits for those reads and no new one starts.
    metric_reads: RwLock<()>,
    info: AppInfo,
    /// The live database file; none in fixture mode, whose database is disposable.
    db_path: Option<PathBuf>,
    /// The home the native index reads; none in fixture mode.
    native_home: Option<PathBuf>,
    compactions: Mutex<xt_ingest::native::session_compactions::Reader>,
    live_codex_status: crate::live_codex_status::LiveCodexStatus,
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

/// One command's clock, zone, catalog and probe, fixed under the lock. It
/// holds no store: a metric read runs after the database lock is released,
/// on its own read-only connection.
struct MetricInputs<'a> {
    now: i64,
    zone: jiff::tz::TimeZone,
    clock: crate::dto::MetricClock,
    catalog: Option<&'a xt_metrics::PriceCatalog>,
    fixture_probe: Option<&'a xt_probes::EnvironmentProbe>,
}

struct Database {
    store: Store,
    metrics_path: PathBuf,
    /// Shared with the metric reads in flight, which outlive the lock.
    metric_context: Arc<MetricContext>,
    // Declared last: SQLite closes before the temporary directory is removed.
    _fixture_directory: Option<tempfile::TempDir>,
}

/// The answer a caller may be given, after one last look at the cancel.
///
/// The loader checks the cancel and returns nothing when it is set. But the
/// payload is translated **after** that check — a whole session's records,
/// their blocks and their arguments — and a reader who left during that work
/// is as gone as one who left during the read. A cancelled read returns
/// nothing at all, so the translated content is dropped here rather than
/// handed to a screen nobody is looking at.
///
/// It is a function of its two arguments so the window can be tested without
/// racing it: a settled outcome and a cancelled token is exactly the state
/// this exists for, whatever order the two arrived in.
fn settled(
    outcome: &xt_ingest::native::session_source::SessionSourceOutcome,
    cancel: &xt_ingest::native::readers_cli::CancelToken,
) -> Result<crate::dto::SessionSourceStatus, &'static str> {
    let status = crate::dto::SessionSourceStatus::try_from(outcome)?;
    if cancel.is_cancelled() {
        return Ok(crate::dto::SessionSourceStatus::Unavailable {
            reason: crate::dto::SessionSourceReason::Cancelled,
        });
    }
    Ok(status)
}

/// What opening one session's source came to: nothing read, with the status
/// that says why, or the loader's outcome, in memory only.
enum SourceOpen {
    NotRead(crate::dto::SessionSourceStatus),
    Read(xt_ingest::native::session_source::SessionSourceOutcome),
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
        let store = Store::open(&db_path).map_err(|source| StateError::OpenDatabase {
            path: db_path.clone(),
            source,
        })?;
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
            // Read here, in `build`, before the caller starts the native index.
            had_indexed_history_at_startup: store.counts()?.sessions > 0,
        };
        Ok(Self {
            live_identity: Mutex::new(None),
            database: Mutex::new(Some(Database {
                store,
                metrics_path: data_dir.join("xtrace.db"),
                metric_context: Arc::new(metric_context),
                _fixture_directory: directory,
            })),
            closing: AtomicBool::new(false),
            metric_reads: RwLock::new(()),
            info,
            db_path,
            live_codex_status: crate::live_codex_status::LiveCodexStatus::new(
                native_home.as_deref(),
            ),
            native_home,
            compactions: Mutex::new(Default::default()),
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

    pub fn live_sessions_read(
        &self,
        ids: &[String],
        view: Option<&str>,
    ) -> Result<crate::dto::LiveSessionSnapshot, StateError> {
        crate::live_codex_status::validate_read(ids, view).map_err(StateError::LiveStatus)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        let Some(token) = view else {
            let snapshot = self
                .live_codex_status
                .read(ids, None, Default::default())
                .map_err(StateError::LiveStatus)?;
            if self.closing.load(Ordering::SeqCst) {
                return Err(StateError::Closed);
            }
            return Ok(snapshot);
        };
        self.live_codex_status
            .require_active(token)
            .map_err(StateError::LiveStatus)?;
        let eligible = self.live_identity_eligible(ids)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        // The identity lock and SQLite snapshot end before observer/file I/O.
        let snapshot = self
            .live_codex_status
            .read(ids, view, eligible)
            .map_err(StateError::LiveStatus)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        Ok(snapshot)
    }

    fn live_identity_eligible(
        &self,
        ids: &[String],
    ) -> Result<std::collections::BTreeSet<String>, StateError> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        let (Some(path), Some(_)) = (&self.db_path, &self.native_home) else {
            return Ok(Default::default());
        };
        let Ok(mut reader) = self.live_identity.try_lock() else {
            // Still call the observer with no eligible IDs to invalidate this
            // view's previous Running claims, rather than returning cached data.
            return Ok(Default::default());
        };
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        if reader.is_none() {
            *reader = xt_store::identity_reader::SessionIdentityReader::open(path).ok();
        }
        let identities: Vec<_> = ids
            .iter()
            .filter_map(|id| {
                if let Some(native) = crate::live_codex_status::canonical_native(id) {
                    Some((id.as_str(), xt_store::Host::Codex, native))
                } else if cfg!(target_os = "macos")
                    && crate::live_codex_status::canonical_claude(id).is_some()
                {
                    Some((id.as_str(), xt_store::Host::Claude, id.as_str()))
                } else {
                    None
                }
            })
            .collect();
        match reader.as_ref().map(|reader| reader.eligible(&identities)) {
            Some(Ok(eligible)) => Ok(eligible),
            _ => {
                // Reopen on the next poll after an open or snapshot failure.
                // Partial eligibility must never survive an interrupted batch.
                reader.take();
                Ok(Default::default())
            }
        }
    }

    pub fn live_sessions_release(&self, view: &str) -> Result<(), StateError> {
        self.live_codex_status
            .release(view)
            .map_err(StateError::LiveStatus)
    }
    /// The fixture's pinned instant, which is also what tells fixture mode
    /// apart from live mode; none when the app is on live data.
    pub fn fixture_now_ms(&self) -> Option<i64> {
        let guard = self.database.lock().ok()?;
        match &*guard.as_ref()?.metric_context {
            MetricContext::System => None,
            #[cfg(all(debug_assertions, feature = "fixtures"))]
            MetricContext::Fixture { now_ms, .. } => Some(*now_ms),
        }
    }

    /// The database lock for one caller, refused once shutdown has begun:
    /// checked before waiting and again after acquiring, so only a holder
    /// admitted before shutdown finishes its work.
    fn admit(&self) -> Result<MutexGuard<'_, Option<Database>>, StateError> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        let guard = self.database.lock().map_err(|_| StateError::Poisoned)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        Ok(guard)
    }

    #[cfg(test)]
    pub(crate) fn admission_closed(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    pub fn db_counts(&self) -> Result<DbCounts, StateError> {
        self.admit()?
            .as_ref()
            .ok_or(StateError::Closed)?
            .store
            .counts()?
            .try_into()
            .map_err(|_| StateError::CountRange)
    }

    /// The app's own store connection under the state lock, for its writes.
    pub(crate) fn with_store<T>(
        &self,
        use_store: impl FnOnce(&mut Store) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        let mut guard = self.admit()?;
        use_store(&mut guard.as_mut().ok_or(StateError::Closed)?.store)
    }

    /// Session metadata and its window measurements from one read snapshot:
    /// the native index writes on its own connection, so a second read could
    /// otherwise show a row beside numbers taken after it changed.
    pub fn sessions_list(
        &self,
        search: &str,
        host: Option<&str>,
        after: Option<&str>,
        window_days: u32,
    ) -> Result<crate::dto::SessionPage, StateError> {
        self.sessions_query(
            crate::dto::SessionQuery {
                search,
                hosts: host.as_ref().map(std::slice::from_ref),
                with_prs: false,
                after,
            },
            window_days,
        )
    }

    /// [`AppState::sessions_list`] with the full filter: a host set and the
    /// recorded-pull-request condition, both applied before paging.
    pub fn sessions_query(
        &self,
        query: crate::dto::SessionQuery<'_>,
        window_days: u32,
    ) -> Result<crate::dto::SessionPage, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            crate::dto::session_page(
                metrics,
                window_days,
                inputs.now,
                inputs.zone,
                inputs.clock,
                query,
            )
        })
    }

    /// Open one session's original local source and read its transcript.
    ///
    /// The canonical identifier the view holds is resolved **here**, against
    /// the index, into the host, the native identity and the locators that
    /// session was measured through, and read under the home this app was
    /// started with. Nothing about where to read comes from the caller: a view
    /// that could name a path could name any path, and this read opens files.
    ///
    /// The store is consulted under the lock and released before the source is
    /// read, because reading a session is not a database operation and must
    /// not hold the connection the rest of the app queries through. Nothing is
    /// written at any point — opening a session changes no row and no file.
    ///
    /// A Codex or Cursor session is read through the pinned reader `readers`
    /// gives this open — its bundle verified and its interpreter resolved now,
    /// under this open's token — or refused with the reason it could not be.
    /// A Claude session never consults it, and fixture startup never reaches
    /// it: there is no local history to read.
    pub fn session_transcript(
        &self,
        session_id: &str,
        cancel: &xt_ingest::native::readers_cli::CancelToken,
        readers: &crate::native_index::DetailReaders,
    ) -> Result<crate::dto::SessionSourceStatus, StateError> {
        match self.open_source(session_id, cancel, readers)? {
            SourceOpen::NotRead(status) => Ok(status),
            SourceOpen::Read(outcome) => {
                settled(&outcome, cancel).map_err(StateError::SourceEncoding)
            }
        }
    }

    /// The resolution and bounded read behind [`AppState::session_transcript`],
    /// shared with the span detail's prompt words: either the status that
    /// says why nothing was read, or the loader's own outcome, held in memory.
    fn open_source(
        &self,
        session_id: &str,
        cancel: &xt_ingest::native::readers_cli::CancelToken,
        readers: &crate::native_index::DetailReaders,
    ) -> Result<SourceOpen, StateError> {
        use crate::native_index::ReadersUnready;
        use xt_ingest::native::readers_cli::DetailBounds;
        use xt_ingest::native::session_source::{
            DetailReader, SessionSourceRequest, SourceLimits, indexed_sources,
            load_session_source_with, names_one_session,
        };
        /// No local source identity for this session, so nothing was opened
        /// and nothing was looked for.
        const NOT_INDEXED: crate::dto::SessionSourceStatus =
            crate::dto::SessionSourceStatus::Unavailable {
                reason: crate::dto::SessionSourceReason::NotIndexed,
            };
        // Fixture startup reads no local history at all, so there is no source
        // to open and nothing to look for.
        let Some(home) = self.native_home() else {
            return Ok(SourceOpen::NotRead(NOT_INDEXED));
        };
        let resolved = {
            let guard = self.admit()?;
            let store = &guard.as_ref().ok_or(StateError::Closed)?.store;
            let Some(session) = store.session(session_id)? else {
                return Ok(SourceOpen::NotRead(NOT_INDEXED));
            };
            let host = session.meta.host;
            let native = session.meta.native_session_id.unwrap_or_default();
            // A Claude session the index recorded without a native identity
            // has no file this read could name. Another host is refused by the
            // loader on its own terms, whatever identity it was recorded with,
            // so its reason is the loader's to give and not this one's.
            if host == xt_store::Host::Claude && native.trim().is_empty() {
                return Ok(SourceOpen::NotRead(NOT_INDEXED));
            }
            let indexed = indexed_sources(store, host, &native)?;
            (host, native, indexed)
        };
        let (host, native, indexed) = resolved;
        let request = SessionSourceRequest {
            home,
            host,
            native_session_id: &native,
            indexed: &indexed,
            cancel: Some(cancel),
            limits: SourceLimits::default(),
        };
        let resolved_readers = match host {
            // Refused by the read itself, before any interpreter is probed.
            xt_store::Host::Codex | xt_store::Host::Cursor if !names_one_session(&native) => None,
            xt_store::Host::Codex | xt_store::Host::Cursor => match readers.resolve(cancel) {
                Ok(resolved) => Some(resolved),
                Err(ReadersUnready::Cancelled) => {
                    return Ok(SourceOpen::NotRead(
                        crate::dto::SessionSourceStatus::Unavailable {
                            reason: crate::dto::SessionSourceReason::Cancelled,
                        },
                    ));
                }
                Err(ReadersUnready::Unavailable(cause)) => {
                    return Ok(SourceOpen::NotRead(
                        crate::dto::SessionSourceStatus::Unavailable {
                            reason: crate::dto::SessionSourceReason::ReaderUnavailable { cause },
                        },
                    ));
                }
            },
            xt_store::Host::Claude | xt_store::Host::Other => None,
        };
        let reader = resolved_readers.as_ref().map(|resolved| DetailReader {
            python: &resolved.python,
            producer: &resolved.producer,
            bounds: DetailBounds::default(),
        });
        Ok(SourceOpen::Read(load_session_source_with(
            &request,
            reader.as_ref(),
        )))
    }

    /// The host's own title for each named session that has one, read now
    /// from its original local source and never stored.
    ///
    /// The identifiers are resolved **here**, against the index, into each
    /// session's host, native identity and recorded locators, under the lock;
    /// the lock is released before any file is opened, because a title read is
    /// not a database operation. Cursor sessions and sessions with no recorded
    /// source are not read at all. Nothing is written, cached or logged, and
    /// a cancel at any point returns no title for any row.
    ///
    /// A request is one visible set: at most one Sessions page of distinct
    /// identifiers, refused whole beyond that rather than silently cut.
    pub fn session_titles(
        &self,
        session_ids: &[String],
        cancel: &xt_ingest::native::readers_cli::CancelToken,
    ) -> Result<crate::dto::SessionTitles, StateError> {
        use xt_ingest::native::session_titles::{
            MAX_TITLE_SESSIONS, TitleLimits, TitleTarget, read_titles, title_sources,
        };
        let distinct: std::collections::BTreeSet<&str> =
            session_ids.iter().map(String::as_str).collect();
        if session_ids.len() > MAX_TITLE_SESSIONS || distinct.len() != session_ids.len() {
            return Err(StateError::InvalidTitleRequest);
        }
        // Fixture startup reads no local history, so there is nothing to read.
        let Some(home) = self.native_home() else {
            return Ok(crate::dto::SessionTitles::default());
        };
        if session_ids.is_empty() || cancel.is_cancelled() {
            return Ok(crate::dto::SessionTitles::default());
        }
        let (ids, targets) = {
            let guard = self.admit()?;
            let store = &guard.as_ref().ok_or(StateError::Closed)?.store;
            let mut ids = Vec::new();
            let mut targets = Vec::new();
            for id in session_ids {
                let Some(session) = store.session(id)? else {
                    continue;
                };
                let host = session.meta.host;
                if !matches!(host, xt_store::Host::Claude | xt_store::Host::Codex) {
                    continue;
                }
                let Some(native) = session.meta.native_session_id else {
                    continue;
                };
                let locators = title_sources(store, host, &native)?;
                if locators.is_empty() {
                    continue;
                }
                ids.push(id.clone());
                targets.push(TitleTarget {
                    host,
                    native_session_id: native,
                    locators,
                });
            }
            (ids, targets)
        };
        let batch = read_titles(home, &targets, TitleLimits::default(), Some(cancel));
        // A reader who left while the last source was read is as gone as one
        // who left before it.
        if cancel.is_cancelled() {
            return Ok(crate::dto::SessionTitles::default());
        }
        Ok(crate::dto::SessionTitles {
            titles: ids
                .into_iter()
                .zip(batch.outcomes)
                .filter_map(|(id, outcome)| {
                    Some(crate::dto::SessionTitle {
                        id,
                        title: outcome.title()?.to_owned(),
                    })
                })
                .collect(),
        })
    }

    pub fn session_compactions(
        &self,
        session_ids: &[String],
        cancel: &xt_ingest::native::readers_cli::CancelToken,
    ) -> Result<crate::dto::SessionCompactions, StateError> {
        use xt_ingest::native::session_compactions::{
            Counted, MAX_SESSIONS, Reason, Target, sources,
        };
        let distinct: std::collections::BTreeSet<&str> =
            session_ids.iter().map(String::as_str).collect();
        if session_ids.len() > MAX_SESSIONS || distinct.len() != session_ids.len() {
            return Err(StateError::InvalidCompactionRequest);
        }
        // Resolve canonical IDs and native identities under the database lock.
        let (mut outcomes, positions, targets) = {
            let guard = self.admit()?;
            let store = &guard.as_ref().ok_or(StateError::Closed)?.store;
            let mut outcomes = vec![Counted::unknown(Reason::NotIndexed); session_ids.len()];
            let mut positions = Vec::new();
            let mut targets = Vec::new();
            for (position, id) in session_ids.iter().enumerate() {
                let Some(session) = store.session(id)? else {
                    continue;
                };
                let Some(native) = session.meta.native_session_id else {
                    continue;
                };
                let host = session.meta.host;
                if host == xt_store::Host::Other {
                    outcomes[position] = Counted::unknown(Reason::Unsupported);
                    continue;
                }
                positions.push(position);
                targets.push(Target {
                    host,
                    native_session_id: native.clone(),
                    locators: sources(store, host, &native)?,
                });
            }
            (outcomes, positions, targets)
        };
        // No database lock is held while waiting for the bounded reader or
        // reading native files. The reader serializes snapshots across views.
        if let Some(home) = self.native_home() {
            let mut reader = self.compactions.lock().map_err(|_| StateError::Closed)?;
            for (position, answer) in positions
                .into_iter()
                .zip(reader.read_events(home, &targets, cancel))
            {
                outcomes[position] = answer;
            }
        }
        if cancel.is_cancelled() {
            outcomes.fill(Counted::unknown(Reason::Cancelled));
        }
        Ok(crate::dto::SessionCompactions {
            counts: session_ids
                .iter()
                .cloned()
                .zip(outcomes)
                .map(|(id, outcome)| crate::dto::SessionCompaction {
                    id,
                    outcome: outcome.into(),
                })
                .collect(),
        })
    }

    /// Read approved names from the original sources of the saved Claude
    /// summaries in exactly the selected Environment window. One deadline
    /// covers preparing and reading; the database lock is released before any
    /// source is opened.
    pub fn environment_hook_names(
        &self,
        window_days: u32,
        window_end_ms: i64,
        cancel: &xt_ingest::native::readers_cli::CancelToken,
    ) -> Result<crate::dto::HookNames, StateError> {
        use xt_ingest::native::hook_names::{self, Request};
        let window = crate::dashboard::selected_window(window_days, window_end_ms)?;
        let start = jiff::Timestamp::from_millisecond(window.start_ms())
            .map_err(|_| StateError::InvalidMetricWindow)?
            .to_string();
        let end = jiff::Timestamp::from_millisecond(window.end_ms())
            .map_err(|_| StateError::InvalidMetricWindow)?
            .to_string();
        let request = Request::start(Some(cancel));
        if request.stopped() {
            return Err(StateError::ReadStopped);
        }
        let prepared = {
            let guard = self.admit()?;
            let store = &guard.as_ref().ok_or(StateError::Closed)?.store;
            hook_names::prepare(store, &start, &end, &request)?
        }
        .ok_or(StateError::ReadStopped)?;
        let count = |value: usize| u32::try_from(value).map_err(|_| StateError::CountRange);
        let Some(home) = self.native_home() else {
            let requested = count(prepared.requested_summaries)?;
            return Ok(crate::dto::HookNames {
                requested_summaries: requested,
                unavailable_summaries: requested,
                ..crate::dto::HookNames::default()
            });
        };
        let names = hook_names::read(home, &prepared, &request);
        Ok(crate::dto::HookNames {
            requested_summaries: count(names.requested_summaries)?,
            checked_summaries: count(names.checked_summaries)?,
            unavailable_summaries: count(names.unavailable_summaries)?,
            summaries_with_unnamed_commands: count(names.summaries_with_unnamed_commands)?,
            labels: names
                .labels
                .into_iter()
                .map(|label| {
                    Ok(crate::dto::HookNameCount {
                        script_basename: label.script_basename.to_owned(),
                        display_label: label.display_label.to_owned(),
                        summaries_mentioning: count(label.summaries_mentioning)?,
                    })
                })
                .collect::<Result<Vec<_>, StateError>>()?,
        })
    }

    /// Exactly one session's row, or nothing.
    ///
    /// The detail screen names the session it is about; it must not have to
    /// search for it. A substring search matches any number of sessions and
    /// returns a bounded page of them, so a session whose identity is a
    /// substring of fifty others could not be found that way at all.
    pub fn session_row(
        &self,
        session_id: &str,
        window_days: u32,
    ) -> Result<Option<crate::dto::SessionRow>, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            crate::dto::session_row(metrics, window_days, inputs.now, session_id)
        })
    }

    /// M-09's stretches for exactly one session over the selected window, each
    /// with M-20's repeat measurement of the same stretch, read in one snapshot.
    ///
    /// Metadata only: the folds read events, stored block positions and stored
    /// comparison evidence and no content, so nothing here opens a source file.
    /// No comparison key is part of the answer. Resolving a stretch's
    /// first-call position against the session's text is the reader's job,
    /// against content it has verified.
    pub fn session_stretches(
        &self,
        session_id: &str,
        window_days: u32,
    ) -> Result<crate::dto::MetricSessionStretches, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            crate::dto::session_stretches(metrics, window_days, inputs.now, session_id)
        })
    }

    /// One activity-lane span's detail, read on demand. The span is the one
    /// the lane report named; no window preset applies, so it is measured with
    /// the same admission and fresh connection as every other metric read.
    ///
    /// When the index did not keep the last prompt's words, they are read from
    /// the session's original source through the same resolution and bounded
    /// reader a transcript open uses, after the database is released, and the
    /// record is picked by its saved identity alone. The words are held in
    /// memory for the answer and nothing is written: no row, no cache, no file.
    pub fn span_detail(
        &self,
        session_id: &str,
        start_ms: i64,
        end_ms: i64,
        reads: &crate::transcript_reads::TranscriptReads,
        read_id: &str,
        readers: &crate::native_index::DetailReaders,
    ) -> Result<crate::dto::DashboardSpanDetail, StateError> {
        let detail = self.with_metric_inputs(|metrics, _| {
            crate::dashboard::span_detail(metrics, session_id, start_ms, end_ms)
        })?;
        // Only the file read takes a slot in `reads`, under `read_id`, so the
        // measured detail never waits for one. Every slot taken answers
        // `busy` for the words alone; a cancel of `read_id`, before or during
        // the read, returns none of them.
        crate::dashboard::span_detail_dto(detail, || match reads.begin(read_id) {
            Ok(read) => self.span_source(session_id, read.token(), readers),
            Err(_) => Ok(crate::dashboard::SpanSource::Busy),
        })
    }

    /// One session's original source for a span's words, held in memory, or
    /// why it could not be read.
    fn span_source(
        &self,
        session_id: &str,
        cancel: &xt_ingest::native::readers_cli::CancelToken,
        readers: &crate::native_index::DetailReaders,
    ) -> Result<crate::dashboard::SpanSource, StateError> {
        use crate::dashboard::SpanSource;
        use crate::dto::{SessionSourceReason, SessionSourceStatus};
        use xt_ingest::native::session_source::SessionSourceOutcome;
        let unavailable = |status: SessionSourceStatus| match status {
            SessionSourceStatus::Unavailable { reason } => SpanSource::Unavailable(reason),
            // A loaded status is never a reason not to read.
            SessionSourceStatus::Loaded { .. } => SpanSource::NotFound,
        };
        let source = match self.open_source(session_id, cancel, readers)? {
            SourceOpen::NotRead(status) => unavailable(status),
            SourceOpen::Read(SessionSourceOutcome::Loaded(session)) => SpanSource::Loaded(session),
            SourceOpen::Read(outcome) => unavailable(
                SessionSourceStatus::try_from(&outcome).map_err(StateError::SourceEncoding)?,
            ),
        };
        // A cancel at any point returns none of the words.
        Ok(if cancel.is_cancelled() {
            SpanSource::Unavailable(SessionSourceReason::Cancelled)
        } else {
            source
        })
    }

    /// A metric read over a preset window; see [`AppState::with_metric_inputs`].
    fn with_metrics<T>(
        &self,
        window_days: u32,
        read: impl FnOnce(&xt_metrics::MetricsDb, MetricInputs<'_>) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        crate::dashboard::validate_window(window_days)?;
        self.with_metric_inputs(read)
    }

    /// One clock and zone, and a fresh read connection, for a read that
    /// selects its own window. Entry goes through the same admission gate as
    /// every other database read.
    fn with_metric_inputs<T>(
        &self,
        read: impl FnOnce(&xt_metrics::MetricsDb, MetricInputs<'_>) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        self.with_prepared_metrics(|_| Ok(()), |metrics, inputs, ()| read(metrics, inputs))
    }

    /// The metric read path. Under the database lock it takes only what the
    /// read needs from the app's own state — the clock, the zone, the metric
    /// context and the database path, plus whatever `prepare` reads from the
    /// app's store (a saved setting, say) — and a shared guard on
    /// `metric_reads`; then it releases the lock and runs `read` on its own
    /// read-only connection. Every metric read used to hold the lock for its
    /// whole length, so a Dashboard report kept every other command,
    /// including a span's detail, waiting behind it.
    ///
    /// SQLite gives each read connection its own snapshot in WAL mode, so
    /// reads never block one another or the writers (the native index, and
    /// the app's own writes under the lock), and none of them waits for a
    /// read. Shutdown still waits: it takes `metric_reads` exclusively after
    /// the lock, so a fixture's temporary database is never removed while a
    /// read uses it.
    fn with_prepared_metrics<P, T>(
        &self,
        prepare: impl FnOnce(&Store) -> Result<P, StateError>,
        read: impl FnOnce(&xt_metrics::MetricsDb, MetricInputs<'_>, P) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        let (context, path, now, zone, prepared, _reading) = {
            let guard = self.admit()?;
            let database = guard.as_ref().ok_or(StateError::Closed)?;
            let (now, zone) = match &*database.metric_context {
                MetricContext::System => (
                    jiff::Timestamp::now().as_millisecond(),
                    jiff::tz::TimeZone::try_system().map_err(|_| StateError::MetricTimezone)?,
                ),
                #[cfg(all(debug_assertions, feature = "fixtures"))]
                MetricContext::Fixture { now_ms, .. } => (*now_ms, jiff::tz::TimeZone::UTC),
            };
            let prepared = prepare(&database.store)?;
            // Taken while the lock is held: shutdown, which holds the lock
            // before it waits for reads, can never miss one starting.
            let reading = self.metric_reads.read().map_err(|_| StateError::Poisoned)?;
            (
                Arc::clone(&database.metric_context),
                database.metrics_path.clone(),
                now,
                zone,
                prepared,
                reading,
            )
        };
        let inputs = match &*context {
            MetricContext::System => MetricInputs {
                now,
                zone,
                clock: crate::dto::MetricClock::System,
                catalog: None,
                fixture_probe: None,
            },
            #[cfg(all(debug_assertions, feature = "fixtures"))]
            MetricContext::Fixture { catalog, probe, .. } => MetricInputs {
                now,
                zone,
                clock: crate::dto::MetricClock::Fixture,
                catalog: Some(catalog),
                fixture_probe: Some(probe),
            },
        };
        let metrics = xt_metrics::MetricsDb::open(&path)?;
        read(&metrics, inputs, prepared)
    }

    pub fn metrics_dashboard(
        &self,
        window_days: u32,
    ) -> Result<crate::dto::DashboardMetrics, StateError> {
        crate::dashboard::validate_window(window_days)?;
        // One saved speed and one saved break length for both the current and
        // the previous window, read under the lock from the app's own store.
        self.with_prepared_metrics(
            |store| {
                Ok((
                    crate::typing_speed::typing_rate(store.typing_speed()?)?,
                    crate::human_break::break_length(store.human_break()?)?,
                ))
            },
            |metrics, inputs, (typing_rate, break_length)| {
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
                    typing_rate,
                    break_length,
                )
            },
        )
    }

    /// The PRs page report: the preset window ending at one instant captured
    /// under the lock, the same instant the token gate is anchored to.
    pub fn pr_analytics(
        &self,
        window_days: u32,
        confirmed_only: bool,
    ) -> Result<crate::dto::PrAnalyticsPage, StateError> {
        self.with_metrics(window_days, |metrics, inputs| {
            crate::pr_analytics::page(
                metrics,
                window_days,
                inputs.now,
                inputs.zone,
                inputs.clock,
                confirmed_only,
            )
        })
    }

    /// One pull request's exact linked sessions over a report's pinned window.
    /// The request is checked in full before storage is touched, and the
    /// window is the request's anchor, never the clock captured here.
    pub fn pr_sessions(
        &self,
        request: crate::pr_analytics::PrSessionsRequest<'_>,
    ) -> Result<crate::dto::SessionPage, StateError> {
        let request = crate::pr_analytics::validate(request)?;
        self.with_metric_inputs(|metrics, inputs| {
            crate::pr_analytics::sessions(metrics, &request, inputs.zone, inputs.clock)
        })
    }

    /// The tray's local day so far: midnight in the captured zone up to the
    /// captured instant, under the same catalog policy as the Dashboard.
    pub fn today(&self) -> Result<crate::today::TodaySummary, StateError> {
        self.with_metric_inputs(|metrics, inputs| {
            let bundled;
            let catalog = match inputs.catalog {
                Some(catalog) => catalog,
                None => {
                    bundled = xt_metrics::PriceCatalog::bundled()?;
                    &bundled
                }
            };
            crate::today::today(metrics, inputs.now, inputs.zone, inputs.clock, catalog)
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
        crate::dashboard::validate_window(window_days)?;
        // The stored repositories are read under the lock from the app's own
        // store; the probe and the report run after it is released.
        let stored_repos = |store: &Store| -> Result<Vec<Option<String>>, StateError> {
            Ok(store
                .sessions_page("", None, None)?
                .into_iter()
                .map(|session| session.repo)
                .collect())
        };
        self.with_prepared_metrics(stored_repos, |metrics, inputs, stored| {
            let native;
            let probe = match inputs.fixture_probe {
                Some(probe) => probe,
                None => {
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

    /// Every stored pull request with the number of sessions that still link
    /// it. Storage reads both in one snapshot, so a concurrent writer — the
    /// native index has its own connection — cannot be seen half applied. A
    /// row with no link is kept here so a caller can tell an unknown ID from
    /// an unlinked one.
    fn pull_request_snapshot(&self) -> Result<Vec<LinkedPullRequest>, StateError> {
        Ok(self
            .admit()?
            .as_ref()
            .ok_or(StateError::Closed)?
            .store
            .linked_pull_requests()?)
    }

    /// The stored pull requests a session still links, in repository then
    /// number order, with the refresh status storage already holds. Numbers
    /// GitHub said are not pull requests are listed with the status
    /// `not_found_on_github`.
    pub fn pr_list(&self) -> Result<PrList, StateError> {
        pr_list(self.pull_request_snapshot()?).map_err(StateError::PrEncoding)
    }

    /// Resolve a selection of stored pull-request IDs to canonical identities,
    /// in the order they were selected. The lock is taken for this read and
    /// released before it returns, so nothing holds it while a refresh runs.
    pub fn pr_refresh_targets(
        &self,
        ids: &[i64],
    ) -> Result<Vec<crate::pr_refresh::PrTarget>, StateError> {
        pr_refresh_targets(&self.pull_request_snapshot()?, ids).map_err(StateError::PrEncoding)
    }

    /// What an automatic check should look at next, from one snapshot; see
    /// [`crate::pr_refresh::auto_selection`]. The lock is held for the read
    /// only.
    pub fn pr_auto_selection(
        &self,
        now_ms: i64,
        tried: &std::collections::HashSet<i64>,
    ) -> Result<crate::pr_refresh::AutoSelection, StateError> {
        Ok(crate::pr_refresh::auto_selection(
            &self.pull_request_snapshot()?,
            now_ms,
            tried,
        ))
    }

    /// Persist one refresh result of the given origin (a manual failure marks
    /// the pull request; see `RefreshOrigin`). The lock is reacquired for this
    /// write only, and a database already closed writes nothing and says so.
    pub fn record_pr_refresh(
        &self,
        outcome: &RefreshOutcome,
        origin: RefreshOrigin,
    ) -> Result<RefreshWrite, StateError> {
        Ok(self
            .admit()?
            .as_mut()
            .ok_or(StateError::Closed)?
            .store
            .record_pr_refresh_from(outcome, origin)?)
    }

    /// Tauri exits the process without dropping managed state. Close resources
    /// explicitly on its Exit event, serialized with any in-flight database read.
    /// Admission closes first, so the wait is for the current lock holder and
    /// the metric reads already running on their own connections only.
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.live_codex_status.shutdown();
        self.live_identity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let mut database = self
            .database
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // No metric read can start now: one takes its guard only while it
        // holds the database lock. Wait for those already reading.
        let _reads = self
            .metric_reads
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        database.take();
    }
}

/// The listed rows of one snapshot: the pull requests a session still links,
/// in the order storage read them.
pub fn pr_list(snapshot: Vec<LinkedPullRequest>) -> Result<PrList, &'static str> {
    // A number GitHub said is not a pull request stays listed, with its own
    // status, so the refresh dialog can offer it for a check by hand; the
    // Pull requests page leaves it out.
    let rows = snapshot
        .iter()
        .filter(|row| row.linked_sessions > 0)
        .map(PrRow::new)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PrList { rows })
}

/// Resolve selected IDs against one snapshot, in the order they were
/// selected. An ID the snapshot does not hold is skipped as unstored; a row
/// no session links any more is skipped as unlinked.
pub fn pr_refresh_targets(
    snapshot: &[LinkedPullRequest],
    ids: &[i64],
) -> Result<Vec<crate::pr_refresh::PrTarget>, &'static str> {
    use crate::pr_refresh::PrTarget;
    ids.iter()
        .map(|id| {
            let Some(row) = snapshot.iter().find(|row| row.pull_request.id == *id) else {
                return Ok(PrTarget::skipped(*id, None, PrSkipReason::NotStored));
            };
            let reference =
                crate::dto::PrRef::new(row.pull_request.id, &row.pull_request.identity)?;
            Ok(if row.linked_sessions > 0 {
                PrTarget::Ready {
                    reference,
                    identity: row.pull_request.identity.clone(),
                }
            } else {
                PrTarget::skipped(*id, Some(reference), PrSkipReason::NotLinked)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xt_ingest::native::{
        readers_cli::CancelToken,
        session_source::{GenerationBasis, LoadedSession, SessionSourceOutcome},
    };

    /// Manual acceptance only: the caller owns a controlled Claude process
    /// with no tools or session persistence. This test never starts/signals it.
    /// Compile first; start this test, then send work after the ready marker.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires explicit probe home/SID and a controlled live Claude publisher"]
    fn live_status_claude_manual_registry_running_then_idle() {
        use crate::dto::LiveSessionState;
        use std::time::{Duration, Instant};

        let home = PathBuf::from(
            std::env::var_os("XTRACE_CLAUDE_PROBE_HOME")
                .expect("manual probe requires XTRACE_CLAUDE_PROBE_HOME"),
        );
        assert!(home.is_absolute(), "manual probe home must be absolute");
        let sid = std::env::var("XTRACE_CLAUDE_PROBE_SID")
            .expect("manual probe requires XTRACE_CLAUDE_PROBE_SID");
        assert!(
            crate::live_codex_status::canonical_claude(&sid).is_some(),
            "manual probe SID must be an exact canonical Claude UUID"
        );
        // Only this disposable index is written. No native history is indexed
        // or copied, and the supplied home is used only by the real adapter.
        let temporary = tempfile::Builder::new()
            .prefix("xtrace-claude-probe-")
            .tempdir_in("/private/tmp")
            .expect("private temporary probe index");
        let mut store =
            Store::open(temporary.path().join("xtrace.db")).expect("temporary probe database");
        let mut meta =
            xt_store::SessionMeta::new(sid.clone(), "claude", xt_store::SessionSource::Transcript);
        meta.native_session_id = Some(sid.clone());
        store
            .upsert_session(&meta, false)
            .expect("exact controlled UUID in temporary index");
        drop(store);
        let state = AppState::build(
            StartupOptions {
                data_dir: Some(temporary.path().to_owned()),
                native_home: Some(home),
                ..Default::default()
            },
            || panic!("manual probe must not select the live database"),
            || panic!("manual probe must not select an implicit home"),
        )
        .expect("temporary probe State");
        let token = state
            .live_sessions_read(&[], None)
            .expect("manual probe lease registration")
            .view_id;
        // Also release/shut down on an unexpected panic inside the observation.
        struct Cleanup<'a>(&'a AppState, String);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.live_sessions_release(&self.1);
                self.0.shutdown();
            }
        }
        let cleanup = Cleanup(&state, token);
        // Manual acceptance also proves that real native Running -> Idle does
        // not wait behind the mutex held by Dashboard coverage. Declare after
        // Cleanup so unwinding releases this guard before Cleanup shuts down.
        let primary = state.database.lock().unwrap();
        let started = Instant::now();
        let requested = [sid.clone()];
        println!("XTRACE_CLAUDE_PROBE_READY session_id={sid}");
        let observed = (|| -> Result<(), &'static str> {
            let mut running = false;
            let mut last = None;
            loop {
                if started.elapsed() >= Duration::from_secs(40) {
                    return Err("manual probe did not observe Running then Idle within 40 seconds");
                }
                let snapshot = state
                    .live_sessions_read(&requested, Some(&cleanup.1))
                    .map_err(|_| "manual probe leased read failed")?;
                if snapshot.view_id != cleanup.1
                    || snapshot.states.len() != 1
                    || snapshot.states[0].id != sid
                {
                    return Err("manual probe returned a different lease or session");
                }
                if started.elapsed() >= Duration::from_secs(40) {
                    return Err("manual probe read exceeded the observation deadline");
                }
                let status = snapshot.states[0].status;
                if last != Some(status) {
                    println!(
                        "{}",
                        serde_json::json!({
                            "session_id": sid, "status": status,
                            "elapsed_ms": started.elapsed().as_millis(),
                        })
                    );
                    last = Some(status);
                }
                // Every accepted state uses the production adapter's file,
                // user/PID/birth-before-and-after guards. The caller confirms
                // that both states belong to its same still-live child.
                if status == LiveSessionState::Running {
                    running = true;
                }
                if running && status == LiveSessionState::Idle {
                    return Ok(());
                }
                // Unknown while the actor/registry starts is never success.
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        let released = state.live_sessions_release(&cleanup.1);
        let token_refused = state
            .live_sessions_read(&requested, Some(&cleanup.1))
            .is_err();
        drop(primary);
        drop(cleanup);
        drop(state);
        temporary.close().expect("remove temporary probe database");
        assert!(released.is_ok(), "manual probe lease release failed");
        assert!(token_refused, "manual probe released lease was accepted");
        observed.expect("manual Claude registry acceptance failed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_status_claude_accepts_branch_conflict_but_requires_exact_unique_user() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let data = root.path().join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let path = data.join("xtrace.db");
        let native = "11111111-1111-4111-8111-111111111111";
        let other = "22222222-2222-4222-8222-222222222222";
        let mut store = Store::open(&path).unwrap();
        for (id, host, saved) in [
            (native.to_owned(), "claude", native),
            (format!("codex-{native}"), "codex", native),
            (other.to_owned(), "claude", other),
        ] {
            let mut meta =
                xt_store::SessionMeta::new(id, host, xt_store::SessionSource::Transcript);
            meta.native_session_id = Some(saved.to_owned());
            meta.git_branch = Some("main".into());
            store.upsert_session(&meta, false).unwrap();
            if meta.session_id == native {
                assert!(!store.session(native).unwrap().unwrap().has_conflict);
                meta.git_branch = Some("feature".into());
                store.upsert_session(&meta, false).unwrap();
                let saved = store.session(native).unwrap().unwrap();
                assert!(saved.has_conflict);
                assert_eq!(saved.meta.git_branch.as_deref(), Some("main"));
            }
        }
        drop(store);
        let state = AppState::build(
            StartupOptions {
                data_dir: Some(data),
                native_home: Some(home),
                ..Default::default()
            },
            || panic!("explicit data"),
            || panic!("explicit home"),
        )
        .unwrap();
        let view = state.live_sessions_read(&[], None).unwrap().view_id;
        let requested = vec![native.to_owned(), format!("codex-{native}")];
        state.live_sessions_read(&requested, Some(&view)).unwrap();
        assert_eq!(state.live_codex_status.requested_targets(), requested);
        let sql = rusqlite::Connection::open(&path).unwrap();
        let assert_conflict_preserved = || {
            assert!(
                sql.query_row(
                    "SELECT has_conflict FROM sessions WHERE session_id=?1",
                    [native],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
            );
        };
        assert_conflict_preserved();
        for (field, value) in [
            ("kind", "judge"),
            ("host", "codex"),
            ("native_session_id", other),
        ] {
            let old: String = sql
                .query_row(
                    &format!("SELECT {field} FROM sessions WHERE session_id=?1"),
                    [native],
                    |row| row.get(0),
                )
                .unwrap();
            sql.execute(
                &format!("UPDATE sessions SET {field}=?1 WHERE session_id=?2"),
                [value, native],
            )
            .unwrap();
            state
                .live_sessions_read(&[native.to_owned()], Some(&view))
                .unwrap();
            assert!(
                state.live_codex_status.requested_targets().is_empty(),
                "{field}"
            );
            assert_conflict_preserved();
            sql.execute(
                &format!("UPDATE sessions SET {field}=?1 WHERE session_id=?2"),
                [old.as_str(), native],
            )
            .unwrap();
        }
        state
            .live_sessions_read(&[native.to_owned()], Some(&view))
            .unwrap();
        assert_eq!(
            state.live_codex_status.requested_targets(),
            [native.to_owned()]
        );
        assert_conflict_preserved();
        sql.execute(
            "UPDATE sessions SET native_session_id=?1 WHERE session_id=?2",
            [native, other],
        )
        .unwrap();
        state
            .live_sessions_read(&[native.to_owned()], Some(&view))
            .unwrap();
        assert!(state.live_codex_status.requested_targets().is_empty());
        assert_conflict_preserved();
        state
            .live_sessions_read(
                &[
                    format!("claude-{native}"),
                    "33333333-3333-4333-8333-333333333333".into(),
                ],
                Some(&view),
            )
            .unwrap();
        assert!(state.live_codex_status.requested_targets().is_empty());
        assert_conflict_preserved();
        state.shutdown();
    }

    #[test]
    fn live_status_codex_accepts_branch_conflict_but_requires_exact_unique_user() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let data = root.path().join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let path = data.join("xtrace.db");
        let parent = "11111111-1111-4111-8111-111111111111";
        let child = "22222222-2222-4222-8222-222222222222";
        let mut store = Store::open(&path).unwrap();
        for (native, host, saved_native) in [
            (parent, "codex", Some(parent)),
            (child, "codex", Some(child)),
            (
                "33333333-3333-4333-8333-333333333333",
                "claude",
                Some("33333333-3333-4333-8333-333333333333"),
            ),
            ("44444444-4444-4444-8444-444444444444", "codex", None),
            (
                "55555555-5555-4555-8555-555555555555",
                "codex",
                Some(parent),
            ),
        ] {
            let mut meta = xt_store::SessionMeta::new(
                format!("codex-{native}"),
                host,
                xt_store::SessionSource::Transcript,
            );
            meta.native_session_id = saved_native.map(str::to_owned);
            meta.git_branch = Some("main".into());
            store.upsert_session(&meta, false).unwrap();
            if native == parent {
                assert!(
                    !store
                        .session(&meta.session_id)
                        .unwrap()
                        .unwrap()
                        .has_conflict
                );
                meta.git_branch = Some("feature".into());
                store.upsert_session(&meta, false).unwrap();
                let saved = store.session(&meta.session_id).unwrap().unwrap();
                assert!(saved.has_conflict);
                assert_eq!(saved.meta.git_branch.as_deref(), Some("main"));
            }
        }
        // Remove the deliberately mismatched parent's alias from the user
        // kind, then exercise the accepted pair before testing each refusal.
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute(
            "UPDATE sessions SET kind='judge' WHERE session_id=?1",
            ["codex-55555555-5555-4555-8555-555555555555"],
        )
        .unwrap();
        drop(store);
        let state = AppState::build(
            StartupOptions {
                data_dir: Some(data),
                native_home: Some(home),
                ..Default::default()
            },
            || panic!("explicit data path"),
            || panic!("explicit home"),
        )
        .unwrap();
        let view = {
            // Registration and malformed registration never touch metadata.
            let _database = state.database.lock().unwrap();
            assert!(
                state
                    .live_sessions_read(&[format!("codex-{parent}")], None)
                    .is_err()
            );
            state.live_sessions_read(&[], None).unwrap().view_id
        };
        let parent_id = format!("codex-{parent}");
        let child_id = format!("codex-{child}");
        state
            .live_sessions_read(std::slice::from_ref(&child_id), Some(&view))
            .unwrap();
        assert_eq!(
            state.live_codex_status.requested_targets(),
            std::slice::from_ref(&child_id)
        ); // never its parent
        state
            .live_sessions_read(std::slice::from_ref(&parent_id), Some(&view))
            .unwrap();
        assert_eq!(
            state.live_codex_status.requested_targets(),
            std::slice::from_ref(&parent_id)
        );
        let assert_conflict_preserved = || {
            assert!(
                sql.query_row(
                    "SELECT has_conflict FROM sessions WHERE session_id=?1",
                    [&parent_id],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
            );
        };
        assert_conflict_preserved();
        for (field, value) in [
            ("kind", "judge"),
            ("native_session_id", child),
            ("host", "claude"),
        ] {
            let old: String = sql
                .query_row(
                    &format!("SELECT {field} FROM sessions WHERE session_id=?1"),
                    [&parent_id],
                    |r| r.get(0),
                )
                .unwrap();
            sql.execute(
                &format!("UPDATE sessions SET {field}=?1 WHERE session_id=?2"),
                rusqlite::params![value, parent_id],
            )
            .unwrap();
            state
                .live_sessions_read(std::slice::from_ref(&parent_id), Some(&view))
                .unwrap();
            assert!(
                state.live_codex_status.requested_targets().is_empty(),
                "{field}"
            );
            assert_conflict_preserved();
            sql.execute(
                &format!("UPDATE sessions SET {field}=?1 WHERE session_id=?2"),
                rusqlite::params![old, parent_id],
            )
            .unwrap();
        }
        state
            .live_sessions_read(std::slice::from_ref(&parent_id), Some(&view))
            .unwrap();
        assert_eq!(
            state.live_codex_status.requested_targets(),
            std::slice::from_ref(&parent_id)
        );
        assert_conflict_preserved();
        sql.execute(
            "UPDATE sessions SET kind='user' WHERE session_id=?1",
            ["codex-55555555-5555-4555-8555-555555555555"],
        )
        .unwrap();
        state
            .live_sessions_read(std::slice::from_ref(&parent_id), Some(&view))
            .unwrap();
        assert!(state.live_codex_status.requested_targets().is_empty()); // ambiguity
        assert_conflict_preserved();
        let refused = [
            "codex-33333333-3333-4333-8333-333333333333",
            "codex-44444444-4444-4444-8444-444444444444",
            "codex-55555555-5555-4555-8555-555555555555",
            "codex-66666666-6666-4666-8666-666666666666",
            parent,
        ];
        state
            .live_sessions_read(&refused.map(str::to_owned), Some(&view))
            .unwrap();
        assert!(state.live_codex_status.requested_targets().is_empty());
        assert_conflict_preserved();
        state.shutdown();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_status_finishes_with_exact_targets_while_primary_mutex_is_held() {
        use std::{sync::Arc, time::Duration};

        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let data = root.path().join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let native = "11111111-1111-4111-8111-111111111111";
        let requested = vec![native.to_owned(), format!("codex-{native}")];
        let mut store = Store::open(data.join("xtrace.db")).unwrap();
        for (id, host) in requested.iter().zip(["claude", "codex"]) {
            let mut meta =
                xt_store::SessionMeta::new(id.clone(), host, xt_store::SessionSource::Transcript);
            meta.native_session_id = Some(native.into());
            store.upsert_session(&meta, false).unwrap();
        }
        drop(store);
        let state = Arc::new(
            AppState::build(
                StartupOptions {
                    data_dir: Some(data),
                    native_home: Some(home),
                    ..Default::default()
                },
                || panic!("explicit data"),
                || panic!("explicit home"),
            )
            .unwrap(),
        );
        let view = state.live_sessions_read(&[], None).unwrap().view_id;
        let primary = state.database.lock().unwrap();
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        let reader = {
            let state = Arc::clone(&state);
            let requested = requested.clone();
            std::thread::spawn(move || {
                sent.send(state.live_sessions_read(&requested, Some(&view)))
                    .unwrap();
            })
        };
        let bounded = received.recv_timeout(Duration::from_secs(1));
        // Always unblock and join before asserting, including on the old path.
        drop(primary);
        reader.join().unwrap();
        let targets = state.live_codex_status.requested_targets();
        state.shutdown();
        let snapshot = bounded
            .expect("live status waited for the primary database mutex")
            .unwrap();
        assert_eq!(snapshot.states.len(), 2);
        assert_eq!(targets, requested);
    }

    fn live_status_test_state(root: &std::path::Path) -> AppState {
        let data = root.join("data");
        let home = root.join("home");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let native = "11111111-1111-4111-8111-111111111111";
        let mut store = Store::open(data.join("xtrace.db")).unwrap();
        for (id, host) in [
            (native.to_owned(), "claude"),
            (format!("codex-{native}"), "codex"),
        ] {
            let mut meta =
                xt_store::SessionMeta::new(id, host, xt_store::SessionSource::Transcript);
            meta.native_session_id = Some(native.into());
            store.upsert_session(&meta, false).unwrap();
        }
        drop(store);
        AppState::build(
            StartupOptions {
                data_dir: Some(data),
                native_home: Some(home),
                ..Default::default()
            },
            || panic!("explicit data"),
            || panic!("explicit home"),
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn live_status_reader_failures_clear_previous_running_and_retry() {
        use crate::dto::LiveSessionState;
        use serde_json::{Value, json};
        use std::{
            io::{Read, Write},
            os::unix::{
                fs::PermissionsExt,
                net::{UnixListener, UnixStream},
            },
            time::{Duration, Instant},
        };
        fn receive(stream: &mut UnixStream) -> Value {
            let mut header = [0; 4];
            stream.read_exact(&mut header).unwrap();
            let length = u32::from_le_bytes(header) as usize;
            assert!(length <= 1024 * 1024);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            serde_json::from_slice(&body).unwrap()
        }
        fn send(stream: &mut UnixStream, value: Value) {
            let bytes = serde_json::to_vec(&value).unwrap();
            stream
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            stream.write_all(&bytes).unwrap();
        }
        let native = "11111111-1111-4111-8111-111111111111";
        let requested = vec![format!("codex-{native}")];
        for failure in ["reader-busy", "sql-error", "query-budget", "open-error"] {
            // A synthetic peer reports Running through the production Codex
            // protocol. This proves cached-state invalidation, not real Codex.
            let root = tempfile::TempDir::new_in("/private/tmp").unwrap();
            let state = std::sync::Arc::new(live_status_test_state(root.path()));
            let ipc = state.native_home().unwrap().join(".codex/ipc");
            std::fs::create_dir_all(&ipc).unwrap();
            std::fs::set_permissions(&ipc, std::fs::Permissions::from_mode(0o700)).unwrap();
            let listener = UnixListener::bind(ipc.join("ipc.sock")).unwrap();
            listener.set_nonblocking(true).unwrap();
            let view = state.live_sessions_read(&[], None).unwrap().view_id;
            state.live_sessions_read(&requested, Some(&view)).unwrap();
            let start = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed() < Duration::from_secs(3) =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("synthetic peer accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            for (method, result) in [
                ("initialize", json!({"clientId":"observer"})),
                ("thread-owner-discovery", json!({})),
            ] {
                let request = receive(&mut stream);
                assert_eq!(request["method"], method);
                send(
                    &mut stream,
                    json!({"type":"response","requestId":request["requestId"],"method":method,"resultType":"success","handledByClientId":"owner","result":result}),
                );
            }
            assert_eq!(
                receive(&mut stream)["method"],
                "thread-stream-following-changed"
            );
            send(
                &mut stream,
                json!({"type":"broadcast","method":"thread-stream-state-changed","sourceClientId":"owner","version":11,"params":{"hostId":"local","conversationId":native,"change":{"type":"snapshot","revision":0,"conversationState":{"id":native,"hostId":"local","threadRuntimeStatus":{"type":"active","activeFlags":[]},"turns":[]}}}}),
            );
            let start = Instant::now();
            loop {
                let snapshot = state.live_sessions_read(&requested, Some(&view)).unwrap();
                if snapshot.states[0].status == LiveSessionState::Running {
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(3),
                    "synthetic Running not received"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            let path = state.database_path().unwrap();
            let writer = rusqlite::Connection::open(path).unwrap();
            let mut moved = None;
            if failure == "sql-error" || failure == "query-budget" {
                writer
                    .execute_batch("ALTER TABLE sessions RENAME TO saved_sessions")
                    .unwrap();
                if failure == "sql-error" {
                    writer.execute_batch("CREATE VIEW sessions AS SELECT abs(-9223372036854775808) session_id,host,native_session_id,kind FROM saved_sessions").unwrap();
                } else {
                    writer.execute_batch("CREATE VIEW sessions AS WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT s.session_id,s.host,s.native_session_id,s.kind FROM saved_sessions s CROSS JOIN n").unwrap();
                }
            } else if failure == "open-error" {
                state.live_identity.lock().unwrap().take();
                // Removing the configured path is confined to this disposable
                // test directory. Restore it before asserting or shutdown.
                let away = root.path().join("away");
                std::fs::rename(root.path().join("data"), &away).unwrap();
                moved = Some(away);
            }
            let busy = (failure == "reader-busy").then(|| state.live_identity.lock().unwrap());
            let started = Instant::now();
            let (sent, received) = std::sync::mpsc::sync_channel(1);
            let poll = {
                let state = std::sync::Arc::clone(&state);
                let requested = requested.clone();
                let view = view.clone();
                std::thread::spawn(move || {
                    sent.send(state.live_sessions_read(&requested, Some(&view)))
                        .unwrap();
                })
            };
            let snapshot = received.recv_timeout(Duration::from_secs(1));
            drop(busy);
            poll.join().unwrap();
            if let Some(away) = moved {
                std::fs::rename(away, root.path().join("data")).unwrap();
            }
            if failure == "sql-error" || failure == "query-budget" {
                writer
                    .execute_batch(
                        "DROP VIEW sessions; ALTER TABLE saved_sessions RENAME TO sessions",
                    )
                    .unwrap();
            }
            assert!(started.elapsed() < Duration::from_secs(2), "{failure}");
            assert_eq!(
                snapshot.unwrap().unwrap().states[0].status,
                LiveSessionState::Unknown,
                "{failure}"
            );
            assert!(
                state.live_codex_status.requested_targets().is_empty(),
                "{failure}"
            );
            state.live_sessions_read(&requested, Some(&view)).unwrap();
            assert_eq!(
                state.live_codex_status.requested_targets(),
                requested,
                "retry {failure}"
            );
            state.live_sessions_release(&view).unwrap();
            state.shutdown();
        }
    }

    #[test]
    fn live_status_shutdown_refuses_reads_while_primary_and_identity_locks_are_held() {
        use std::{
            sync::{Arc, mpsc},
            time::{Duration, Instant},
        };
        let root = tempfile::TempDir::new().unwrap();
        let state = Arc::new(live_status_test_state(root.path()));
        let requested = vec!["codex-11111111-1111-4111-8111-111111111111".into()];
        let view = state.live_sessions_read(&[], None).unwrap().view_id;
        state.live_sessions_read(&requested, Some(&view)).unwrap();
        let primary = state.database.lock().unwrap();
        let identity = state.live_identity.lock().unwrap();
        let closer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || state.shutdown())
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while !state.closing.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let (sent, received) = mpsc::sync_channel(1);
        let reader = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || {
                sent.send((
                    state.live_sessions_read(&requested, Some(&view)),
                    state.live_sessions_read(&[], None),
                ))
                .unwrap()
            })
        };
        let bounded = received.recv_timeout(Duration::from_secs(1));
        drop(identity);
        drop(primary);
        reader.join().unwrap();
        closer.join().unwrap();
        let (leased, registration) = bounded.unwrap();
        assert!(matches!(leased, Err(StateError::Closed)));
        assert!(matches!(registration, Err(StateError::Closed)));
        assert!(state.live_identity.lock().unwrap().is_none());
        assert!(state.live_codex_status.requested_targets().is_empty());
    }

    #[test]
    fn live_status_released_view_does_not_open_identity_reader() {
        let root = tempfile::TempDir::new().unwrap();
        let state = live_status_test_state(root.path());
        let view = state.live_sessions_read(&[], None).unwrap().view_id;
        state.live_sessions_release(&view).unwrap();
        assert!(
            state
                .live_sessions_read(
                    &["codex-11111111-1111-4111-8111-111111111111".into()],
                    Some(&view)
                )
                .is_err()
        );
        assert!(state.live_identity.lock().unwrap().is_none());
        state.shutdown();
    }

    #[cfg(all(debug_assertions, feature = "fixtures"))]
    #[test]
    fn live_status_fixture_never_opens_identity_reader_or_claims_native_status() {
        let root = tempfile::TempDir::new().unwrap();
        let state = std::sync::Arc::new(
            AppState::build(
                StartupOptions {
                    fixture: Some("F1".into()),
                    data_dir: Some(root.path().to_owned()),
                    ..Default::default()
                },
                || panic!("fixture live DB lookup"),
                || panic!("fixture home lookup"),
            )
            .unwrap(),
        );
        let view = state.live_sessions_read(&[], None).unwrap().view_id;
        let primary = state.database.lock().unwrap();
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        let reader = {
            let state = std::sync::Arc::clone(&state);
            std::thread::spawn(move || {
                sent.send(state.live_sessions_read(
                    &["codex-11111111-1111-4111-8111-111111111111".into()],
                    Some(&view),
                ))
                .unwrap();
            })
        };
        let snapshot = received.recv_timeout(std::time::Duration::from_secs(1));
        drop(primary);
        reader.join().unwrap();
        assert_eq!(
            snapshot.unwrap().unwrap().states[0].status,
            crate::dto::LiveSessionState::Unknown
        );
        assert!(state.live_identity.lock().unwrap().is_none());
        assert!(state.database_path().is_none());
        assert!(state.native_home().is_none());
        assert!(state.live_codex_status.requested_targets().is_empty());
        let directory = PathBuf::from(state.app_info().data_dir);
        state.shutdown();
        assert!(!directory.exists());
    }

    fn loaded() -> SessionSourceOutcome {
        SessionSourceOutcome::Loaded(Box::new(LoadedSession {
            native_session_id: "00000000-0000-4000-8000-00000000aaaa".into(),
            host: xt_store::Host::Claude,
            generation: GenerationBasis::Indexed { appended: false },
            sources: Vec::new(),
            records: Vec::new(),
            other_lines: 0,
            dropped_records: 0,
            gaps: Vec::new(),
        }))
    }

    #[test]
    fn a_cancel_that_lands_after_the_read_still_returns_nothing() {
        // The loader checked the cancel and produced a session; the payload is
        // translated after that, and a reader who left during that work is as
        // gone as one who left during the read. No content is handed over.
        let cancel = CancelToken::new();
        let outcome = loaded();
        assert!(matches!(
            settled(&outcome, &cancel).unwrap(),
            crate::dto::SessionSourceStatus::Loaded { .. }
        ));
        cancel.cancel();
        assert_eq!(
            settled(&outcome, &cancel).unwrap(),
            crate::dto::SessionSourceStatus::Unavailable {
                reason: crate::dto::SessionSourceReason::Cancelled,
            }
        );
    }

    #[test]
    fn a_cancelled_read_carries_no_part_of_what_was_loaded() {
        // Not merely a different label on the same answer: a cancelled read
        // returns nothing at all, so nothing translated can reach a caller.
        let cancel = CancelToken::new();
        cancel.cancel();
        let wire = serde_json::to_string(&settled(&loaded(), &cancel).unwrap()).unwrap();
        assert_eq!(
            wire,
            r#"{"state":"unavailable","reason":{"reason":"cancelled"}}"#
        );
    }

    /// The span detail's own read for a session nobody indexed: it measures
    /// through the metric path and opens no source.
    fn missing_span(state: &AppState) -> Result<crate::dto::DashboardSpanDetail, StateError> {
        state.span_detail(
            "no-such-session",
            1_788_782_400_000,
            1_788_782_460_000,
            &crate::transcript_reads::TranscriptReads::default(),
            "span",
            &crate::native_index::DetailReaders::disabled(),
        )
    }

    /// A metric read in progress: send to end it, then join it.
    type HeldRead = (
        std::sync::mpsc::Sender<()>,
        std::thread::JoinHandle<Result<(), StateError>>,
    );

    /// Start a metric read that stays in progress until `released` answers.
    fn held_metric_read(state: &std::sync::Arc<AppState>) -> HeldRead {
        use std::sync::mpsc;
        let (started, running) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = std::sync::Arc::clone(state);
        let reader = std::thread::spawn(move || {
            holder.with_metric_inputs(|metrics, _| {
                // A real read connection, mid-read.
                metrics.span_detail("no-such-session", 0, 0).unwrap();
                started.send(()).unwrap();
                released.recv().unwrap();
                Ok(())
            })
        });
        running
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        (release, reader)
    }

    /// A metric read in progress holds only its own read connection: a span's
    /// detail, a Dashboard report, a count and a write under the database
    /// lock all answer while it runs.
    #[test]
    fn a_metric_read_in_progress_keeps_no_other_command_waiting() {
        use std::sync::{Arc, mpsc};
        let root = tempfile::TempDir::new().unwrap();
        let state = Arc::new(live_status_test_state(root.path()));
        let (release, reader) = held_metric_read(&state);
        let (sent, answers) = mpsc::channel::<(&str, bool)>();
        type Call = fn(&AppState) -> bool;
        let calls: [(&str, Call); 4] = [
            ("span_detail", |state| {
                matches!(
                    missing_span(state),
                    Ok(crate::dto::DashboardSpanDetail::Missing)
                )
            }),
            ("metrics_dashboard", |state| {
                state.metrics_dashboard(7).is_ok()
            }),
            ("db_counts", |state| state.db_counts().is_ok()),
            ("with_store", |state| {
                state.with_store(|store| Ok(store.counts()?)).is_ok()
            }),
        ];
        for (name, call) in calls {
            let (state, sent) = (Arc::clone(&state), sent.clone());
            std::thread::spawn(move || sent.send((name, call(&state))).unwrap());
        }
        for _ in 0..calls.len() {
            let (name, ok) = answers
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("answered while the metric read is still running");
            assert!(ok, "{name}");
        }
        assert!(!reader.is_finished(), "the held read is still running");
        release.send(()).unwrap();
        reader.join().unwrap().unwrap();
        state.shutdown();
    }

    /// Shutdown closes admission at once, then waits for a metric read that
    /// was already running on its own connection before it closes the
    /// database; reads arriving meanwhile are refused, not queued.
    #[test]
    fn shutdown_waits_for_metric_reads_in_flight_and_refuses_new_ones() {
        use std::sync::Arc;
        let root = tempfile::TempDir::new().unwrap();
        let state = Arc::new(live_status_test_state(root.path()));
        let (release, reader) = held_metric_read(&state);
        let closer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || state.shutdown())
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !state.admission_closed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(matches!(missing_span(&state), Err(StateError::Closed)));
        assert!(matches!(
            state.metrics_dashboard(7),
            Err(StateError::Closed)
        ));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!closer.is_finished(), "shutdown waits for the running read");
        assert!(
            state.database.try_lock().is_err(),
            "shutdown holds the database lock while it waits"
        );
        release.send(()).unwrap();
        reader.join().unwrap().unwrap();
        closer.join().unwrap();
        assert!(state.database.lock().unwrap().is_none());
    }

    /// In fixture mode the temporary database outlives every read that began
    /// before shutdown, and is removed once the last one ends.
    #[cfg(all(debug_assertions, feature = "fixtures"))]
    #[test]
    fn a_fixture_database_outlives_the_metric_reads_shutdown_waits_for() {
        use std::sync::{Arc, mpsc};
        let root = tempfile::TempDir::new().unwrap();
        let state = Arc::new(
            AppState::build(
                StartupOptions {
                    data_dir: Some(root.path().to_owned()),
                    fixture: Some("F1".into()),
                    ..Default::default()
                },
                || panic!("fixture"),
                || panic!("fixture"),
            )
            .unwrap(),
        );
        let directory = PathBuf::from(state.app_info().data_dir);
        let database = directory.join("xtrace.db");
        let (started, running) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = Arc::clone(&state);
        let path = database.clone();
        let reader = std::thread::spawn(move || {
            holder.with_metric_inputs(|metrics, _| {
                started.send(()).unwrap();
                released.recv().unwrap();
                assert!(path.exists(), "the read keeps its database");
                // The connection still reads after shutdown began.
                metrics.span_detail("no-such-session", 0, 0)?;
                Ok(())
            })
        });
        running
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let closer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || state.shutdown())
        };
        while !state.admission_closed() {
            std::thread::yield_now();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(database.exists());
        release.send(()).unwrap();
        assert!(reader.join().unwrap().is_ok());
        closer.join().unwrap();
        assert!(
            !directory.exists(),
            "shutdown removes the disposable database"
        );
    }
}
