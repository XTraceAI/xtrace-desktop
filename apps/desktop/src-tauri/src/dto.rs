//! Serialized IPC and fixture contracts. Integer constructors enforce JSON precision.
pub use crate::account_usage_dto::*;
pub use crate::dashboard_dto::*;
pub use crate::environment_dto::*;
pub use crate::hook_names::{HookNameCount, HookNames};
pub use crate::pr_analytics_dto::*;
pub use crate::pr_dto::*;
pub use crate::pr_effort_dto::*;
pub use crate::rule_activity_dto::*;
pub use crate::session_compactions::{
    CompactionCount, CompactionEvent, CompactionReason, CompactionTrigger, SessionCompaction,
    SessionCompactions,
};
pub use crate::session_source_dto::*;
pub use crate::session_titles::{SessionTitle, SessionTitles};
use serde::Serialize;
use ts_rs::TS;
use xt_store::StoreCounts;

/// A transient status reported by the existing Codex Desktop peer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LiveSessionState {
    Running,
    WaitingApproval,
    WaitingInput,
    Idle,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct LiveSessionStatus {
    pub id: String,
    // Keep the renderer contract inline, with no separately imported enum.
    #[ts(type = "'running' | 'waiting_approval' | 'waiting_input' | 'idle' | 'unknown'")]
    pub status: LiveSessionState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct LiveSessionSnapshot {
    pub view_id: String,
    pub states: Vec<LiveSessionStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub data_dir: String,
    pub fixture: Option<String>,
    pub schema_version: u32,
    pub listening: bool,
    /// Whether the database already held indexed sessions when this process
    /// opened it, before the native index started. Fixed for the process: the
    /// initial scan fills the database moments later, so live counts cannot
    /// tell a first launch from an upgrade.
    pub had_indexed_history_at_startup: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DbCounts {
    #[ts(type = "number")]
    sessions: u64,
    #[ts(type = "number")]
    records: u64,
    #[ts(type = "number")]
    usage: u64,
}

impl TryFrom<StoreCounts> for DbCounts {
    type Error = &'static str;
    fn try_from(value: StoreCounts) -> Result<Self, Self::Error> {
        if [value.sessions, value.records, value.usage_rows]
            .iter()
            .any(|value| *value >= 1_u64 << 53)
        {
            return Err("database count exceeds the exact JSON integer range");
        }
        Ok(Self {
            sessions: value.sessions,
            records: value.records,
            usage: value.usage_rows,
        })
    }
}

/// The native index the app keeps over the local Claude, Codex and Cursor
/// history: its lifecycle phase, whether live changes reach it, what the
/// reader hosts need, and each host's last scan. Paths of native sources
/// stay out of it; they are local index metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct NativeIndexStatus {
    pub phase: NativeIndexPhase,
    pub freshness: NativeFreshness,
    pub python: PythonRuntime,
    pub readers: ReaderBundle,
    pub hosts: Vec<NativeHostStatus>,
    /// Reconciliations completed: the initial scan, then one per change burst.
    pub reconciles: u32,
    /// Claude transcripts read so far by the initial scan (progress while scanning).
    pub files_scanned: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum NativeIndexPhase {
    /// No index runs in this process: fixture mode, or a data directory the
    /// index must not use.
    Disabled { reason: String },
    /// The initial scan (and the reconciliation of changes made during it).
    Scanning,
    /// Ready: the initial scan is done and live changes are reconciled.
    Ready,
    /// Stopped at shutdown, or the worker ended on its own.
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "freshness", rename_all = "snake_case")]
pub enum NativeFreshness {
    /// Not known yet: the watcher is not registered.
    Unknown,
    Live,
    /// The index reflects the last scan only.
    Degraded {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PythonRuntime {
    /// Discovery runs beside the initial scan; the reader hosts' own scans
    /// resolve the interpreter themselves meanwhile.
    Resolving,
    /// The interpreter the reader hosts run with, as found at startup.
    Available { path: String },
    /// Claude indexing continues; Codex and Cursor report the missing runtime.
    Missing { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReaderBundle {
    /// The bundled reader sources are exactly the pinned producer's objects.
    Verified {
        commit: String,
        plugin_version: String,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct NativeHostStatus {
    /// `claude`, `codex` or `cursor`.
    pub host: String,
    pub state: NativeHostState,
    pub detail: Option<String>,
    pub sessions_imported: u32,
    pub sessions_partial: u32,
    pub sessions_skipped: u32,
    // Bounded local details from this host report, with validated IDs only.
    pub skipped_conversations: Vec<NativeSkippedConversation>,
    pub skipped_conversations_omitted: u32,
    pub records_new: u32,
    pub records_enriched: u32,
    pub diagnostics: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct NativeSkippedConversation {
    // Ordinary conversation ID, or unavailable when it cannot be validated.
    pub conversation_id: Option<String>,
    pub reason: NativeSkipReason,
}

/// Fixed classifications only: never serialize a source's raw error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NativeSkipReason {
    Unreadable,
    InvalidTranscript,
    InvalidHeader,
    IdentityConflict,
    InvalidSurface,
    CheckpointFailed,
    WriteFailed,
    ReaderIncomplete,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NativeHostState {
    /// Not scanned yet in this process.
    Pending,
    Complete,
    Incomplete,
    MissingSource,
    MissingRuntime,
    PinMismatch,
    ReaderFailed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct FixtureExport {
    pub app_info: AppInfo,
    pub db_counts: DbCounts,
    pub native_index: NativeIndexStatus,
    pub sessions: Vec<SessionPage>,
    /// M-09 stretches for each listed session, per window preset.
    pub session_stretches: Vec<FixtureSessionStretches>,
    /// The detail of every span the Dashboard lanes return, as the span
    /// detail command answers it.
    pub span_details: Vec<FixtureSpanDetail>,
    pub dashboards: Vec<DashboardMetrics>,
    pub environments: Vec<EnvironmentMetrics>,
    /// The stored pull requests this fixture's sessions link, before anything
    /// refreshed them.
    pub pull_requests: PrList,
    /// One synthetic refresh of every listed pull request, at the fixture's
    /// pinned instant, produced by the application's own batch.
    pub pr_refresh: PrRefreshReport,
    /// The list that refresh leaves behind.
    pub pull_requests_refreshed: PrList,
    /// Each range's M-19 section after refreshing each non-empty subset of the
    /// listed pull requests from the unrefreshed start. `dashboards` is read
    /// before any refresh, as a fixture database starts.
    pub pr_effort_states: Vec<FixturePrEffortState>,
    /// The PRs page report for each range and confidence mode, read before
    /// any refresh as a fixture database starts: no cached merge facts yet.
    pub pr_analytics: Vec<PrAnalyticsPage>,
    /// Each listed pull request's exact linked sessions for each range and
    /// confidence mode, pinned to that report's window end. Membership is
    /// links, not refreshed facts, so a refresh changes only link titles.
    pub pr_sessions: Vec<FixturePrSessions>,
    pub today: crate::today::TodaySummary,
    /// A rule activity read as fixture startup answers it. Fixture mode has
    /// no native home, so it has no source, and it never reads the user's own.
    pub rule_activity: RuleActivityResult,
}

pub fn export_types(directory: impl AsRef<std::path::Path>) -> Result<(), ts_rs::ExportError> {
    LiveSessionSnapshot::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    AccountUsage::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    TokensByHost::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    SessionPage::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    MetricSessionStretches::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    SessionSourceStatus::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    SessionTitles::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    SessionCompactions::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    DashboardSpanDetail::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    HookNames::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    PrList::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    PrRefreshReport::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    PrAnalyticsPage::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    RuleActivityResult::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    crate::privacy::ContentRetention::export_all(
        &ts_rs::Config::new().with_out_dir(directory.as_ref()),
    )?;
    crate::privacy::ContentPurge::export_all(
        &ts_rs::Config::new().with_out_dir(directory.as_ref()),
    )?;
    FixtureExport::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_counts_preserve_json_precision() {
        let maximum = (1_u64 << 53) - 1;
        let accepted = DbCounts::try_from(StoreCounts {
            sessions: maximum,
            records: 0,
            usage_rows: 0,
        })
        .unwrap();
        assert_eq!(serde_json::to_value(accepted).unwrap()["sessions"], maximum);
        for value in [1_u64 << 53, u64::MAX] {
            for counts in [
                StoreCounts {
                    sessions: value,
                    ..Default::default()
                },
                StoreCounts {
                    records: value,
                    ..Default::default()
                },
                StoreCounts {
                    usage_rows: value,
                    ..Default::default()
                },
            ] {
                assert!(DbCounts::try_from(counts).is_err());
            }
        }
    }
}

/// What one listed session contributed inside the page's explicit window.
/// Mirrors `xt_metrics::SessionWindow` field for field: a listed row is always
/// `indexed`, and the `missing` state keeps an explicit identifier lookup and
/// this list on one contract rather than reporting an absent session as empty.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MetricSessionWindow {
    Missing,
    Indexed {
        #[ts(type = "number")]
        events: u64,
        #[ts(type = "number | null")]
        human_messages: Option<u64>,
        /// Tool-use blocks in the window (the Dashboard tool-call count); `null` when any in-window
        /// record's count is unknown, never a guessed zero.
        #[ts(type = "number | null")]
        tool_calls: Option<u64>,
        tokens: MetricTokenSummary,
        #[ts(type = "number")]
        agent_ms: u64,
    },
}

/// `xt_metrics::SessionHandsOff`, field for field: the median of this session's
/// own M-09 stretches in the page's window, with `n`.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MetricSessionHandsOff {
    Missing,
    /// `excluded_surface` is the surface timestamp-health exclusion; `null`
    /// means this session's own records leave a stretch boundary unknown.
    Unmeasured {
        excluded_surface: Option<MetricExcludedSurface>,
    },
    /// `median_min` is absent exactly when `n` is zero: no eligible stretch is
    /// not a zero-minute median.
    Measured {
        #[ts(type = "number")]
        n: u64,
        median_min: Option<f64>,
    },
}

/// One stored session→pull request link. Its confidence travels with it so an
/// inferred link is never shown as if it were exact.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct SessionPrLink {
    /// Canonical lowercase `owner/repo`.
    pub repository: String,
    #[ts(type = "number")]
    pub number: u64,
    pub url: String,
    pub confidence: MetricPrConfidence,
    /// The refresh-owned title, when a refresh stored one.
    pub title: Option<String>,
}

/// Structural metadata beside this session's window measurements. All of it
/// comes from one read snapshot, so it describes the same committed state.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct SessionRow {
    pub id: String,
    pub host: String,
    /// A title a source already saved; `null` when none was. Never derived.
    pub title: Option<String>,
    /// Verified native Guardian header; a display hint, never a saved title.
    pub automated_review: bool,
    /// The session start in UTC milliseconds: the start the host recorded, or,
    /// for a Claude session (Claude Code records no start), its earliest
    /// imported message. `null` when neither is known.
    #[ts(type = "number | null")]
    pub started_at_ms: Option<i64>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    /// Earliest visible work record, including inherited copies.
    pub first_ts: Option<String>,
    #[ts(type = "number")]
    pub record_count: u64,
    pub has_conflict: bool,
    /// Every stored link, strongest confidence first.
    pub pr_links: Vec<SessionPrLink>,
    /// The session that verifiably created this one, when it is exactly one
    /// indexed user session; `null` otherwise. Display only: it never changes
    /// this row's membership, order, cursor or measurements, and a parent
    /// filtered out or on another page is not added to the list.
    #[ts(optional = nullable)]
    pub parent: Option<SessionParentLink>,
    pub metrics: MetricSessionWindow,
    pub hands_off: MetricSessionHandsOff,
}
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct SessionPage {
    /// The event window every row's measurements were taken over (M-01).
    pub window: DashboardWindow,
    pub rows: Vec<SessionRow>,
    pub next: Option<String>,
}

/// The caller's page selection, passed through to the store unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionQuery<'a> {
    pub search: &'a str,
    /// `None` lists every host; otherwise a non-empty host set.
    pub hosts: Option<&'a [&'a str]>,
    /// Only sessions with a stored pull-request link of any confidence.
    pub with_prs: bool,
    /// The opaque cursor a previous page returned as `next`.
    pub after: Option<&'a str>,
}

/// Assemble one row from its metadata and the measurements taken beside it in
/// the same snapshot. A missing measurement is reported, never filled in.
fn session_row_from(
    summary: xt_store::session_list::SessionSummary,
    measured: &std::collections::BTreeMap<String, xt_metrics::SessionWindow>,
    hands_off: &std::collections::BTreeMap<String, xt_metrics::SessionHandsOff>,
) -> Result<SessionRow, crate::state::StateError> {
    let metrics = match measured.get(&summary.id) {
        Some(window) => crate::dashboard::convert(window)?,
        // Unreachable while the row and its measurements share one snapshot.
        None => MetricSessionWindow::Missing,
    };
    let hands_off = match hands_off.get(&summary.id) {
        Some(answer) => crate::dashboard::convert(answer)?,
        None => MetricSessionHandsOff::Missing,
    };
    let pr_links = summary
        .pr_links
        .into_iter()
        .map(|link| SessionPrLink {
            repository: link.pull_request.repository().to_owned(),
            number: link.pull_request.number(),
            url: link.pull_request.url(),
            confidence: match link.confidence {
                xt_store::pr_link::PrConfidence::Exact => MetricPrConfidence::Exact,
                xt_store::pr_link::PrConfidence::Sha => MetricPrConfidence::Sha,
                xt_store::pr_link::PrConfidence::Inferred => MetricPrConfidence::Inferred,
            },
            title: link.title,
        })
        .collect();
    Ok(SessionRow {
        id: summary.id,
        host: summary.host,
        title: summary.title,
        automated_review: summary.automated_review,
        started_at_ms: summary.started_at_ms,
        repo: summary.repo,
        branch: summary.branch,
        model: summary.model,
        first_ts: summary.first_ts,
        record_count: summary.record_count,
        has_conflict: summary.has_conflict,
        pr_links,
        parent: summary.parent.map(Into::into),
        metrics,
        hands_off,
    })
}

/// One page of session metadata with its window measurements, read through the
/// metrics connection inside a single snapshot. Pagination, search and host
/// filtering remain the store's; nothing here re-derives a metric definition.
///
/// Membership is every indexed user session that passes the filter, whatever
/// the window; the window only slices what each row measured.
pub fn session_page(
    metrics: &xt_metrics::MetricsDb,
    days: u32,
    now_ms: i64,
    zone: jiff::tz::TimeZone,
    clock: MetricClock,
    query: SessionQuery<'_>,
) -> Result<SessionPage, crate::state::StateError> {
    let SessionQuery {
        search,
        hosts,
        with_prs,
        after,
    } = query;
    let window = crate::dashboard::selected_window(days, now_ms)?;
    let cursor = session_cursor(after)?;
    let filter = xt_store::session_list::SessionFilter {
        search,
        hosts,
        with_prs,
        pull_request: None,
    };
    filtered_page(metrics, days, window, zone, clock, &filter, cursor.as_ref())
}

/// The opaque cursor a previous page returned, checked without storage.
pub(crate) fn session_cursor(
    after: Option<&str>,
) -> Result<Option<xt_store::session_list::SessionCursor>, crate::state::StateError> {
    if after.is_some_and(|s| s.len() > 4096) {
        return Err(xt_store::Error::InvalidInput("invalid session cursor").into());
    }
    Ok(after
        .map(serde_json::from_str::<xt_store::session_list::SessionCursor>)
        .transpose()
        .map_err(|_| xt_store::Error::InvalidInput("invalid session cursor"))?)
}

/// One bounded page of the store's filtered list with each row's measurements
/// over `window`, all in one read snapshot: at most 50 rows, and a `next`
/// cursor exactly when the store found a 51st.
pub(crate) fn filtered_page(
    metrics: &xt_metrics::MetricsDb,
    days: u32,
    window: xt_metrics::Window,
    zone: jiff::tz::TimeZone,
    clock: MetricClock,
    filter: &xt_store::session_list::SessionFilter<'_>,
    cursor: Option<&xt_store::session_list::SessionCursor>,
) -> Result<SessionPage, crate::state::StateError> {
    let (rows, next, measured, hands_off) = metrics.read_snapshot(|metrics| {
        let mut rows = metrics.sessions_page_filtered(filter, cursor)?;
        let next = if rows.len() > 50 {
            rows.truncate(50);
            rows.last()
                .map(|row| serde_json::to_string(&row.cursor))
                .transpose()
                .map_err(xt_store::Error::from)?
        } else {
            None
        };
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        let measured = metrics.session_windows(window, &ids)?;
        let hands_off = metrics.session_hands_off(window, &ids)?;
        Ok((rows, next, measured, hands_off))
    })?;
    let page = SessionPage {
        window: crate::dashboard::dashboard_window(days, window, &zone, clock),
        next,
        rows: rows
            .into_iter()
            .map(|r| session_row_from(r, &measured, &hands_off))
            .collect::<Result<_, crate::state::StateError>>()?,
    };
    crate::dashboard::checked_value(&page)?;
    Ok(page)
}

/// Exactly one session's metadata and its window measurements, or nothing.
///
/// The same row the list builds, from the same read, under the same rules — a
/// detail screen and the list it was opened from must not disagree about what
/// a session measured. What differs is how the session is named: exactly,
/// instead of by a substring search that can match any number of sessions and
/// need not put the one that was asked for on its first page.
pub fn session_row(
    metrics: &xt_metrics::MetricsDb,
    days: u32,
    now_ms: i64,
    query: &str,
) -> Result<Option<SessionRow>, crate::state::StateError> {
    let window = crate::dashboard::selected_window(days, now_ms)?;
    let Some((summary, measured, hands_off)) = metrics.read_snapshot(|metrics| {
        // Metadata and measurement come from one snapshot, as the list's do:
        // the native index writes on its own connection, so a second read
        // could show a row beside numbers taken after it changed.
        let Some(summary) = metrics.session_exact(query)? else {
            return Ok(None);
        };
        let measured = metrics.session_windows(window, &[summary.id.as_str()])?;
        let hands_off = metrics.session_hands_off(window, &[summary.id.as_str()])?;
        Ok(Some((summary, measured, hands_off)))
    })?
    else {
        return Ok(None);
    };
    let row = session_row_from(summary, &measured, &hands_off)?;
    crate::dashboard::checked_value(&row)?;
    Ok(Some(row))
}

/// M-09's stretches for exactly one session over the selected window, each
/// with M-20's answer about the same stretch.
///
/// Nothing is measured here. The window is the one every other windowed read
/// selects, the folds are `xt_metrics::MetricsDb::session_stretches` and
/// `session_repeats` with M-20's default thresholds, and each answer crosses
/// through a conversion that refuses any drift from the core's own shape — so
/// a segment on the detail screen is one of the segments the Dashboard's
/// hands-off number was made of, with the same duration.
///
/// Both are read inside one snapshot, so the stretches M-20 counted are the
/// stretches M-09 states while the native writer keeps appending. They are
/// then combined only where they agree: the same state, the same excluded
/// surface, and for every stretch the same endpoints and elapsed duration. Two
/// reports that disagree are refused, not reconciled, because a repeat count
/// beside the wrong stretch is worse than no count.
pub fn session_stretches(
    metrics: &xt_metrics::MetricsDb,
    days: u32,
    now_ms: i64,
    session_id: &str,
) -> Result<MetricSessionStretches, crate::state::StateError> {
    let window = crate::dashboard::selected_window(days, now_ms)?;
    let (stretches, repeats) = metrics.read_snapshot(|metrics| {
        Ok((
            metrics.session_stretches(window, session_id)?,
            metrics.session_repeats(window, session_id, xt_metrics::RepeatThresholds::default())?,
        ))
    })?;
    combine(
        crate::dashboard::convert(&stretches)?,
        crate::dashboard::convert(&repeats)?,
    )
}

/// `xt_metrics::SessionStretches`, field for field: the conversion target for
/// M-09's answer before M-20's is set beside it.
#[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CoreStretches {
    Missing,
    Unmeasured {
        excluded_surface: Option<MetricExcludedSurface>,
    },
    Measured {
        stretches: Vec<CoreStretch>,
    },
}

#[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
struct CoreStretch {
    start_uuid: String,
    end_uuid: String,
    start: String,
    end: String,
    duration_ms: u64,
    first_tool: Option<MetricToolBlock>,
}

/// `xt_metrics::SessionRepeats`, field for field.
#[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CoreRepeats {
    Missing,
    Unmeasured {
        excluded_surface: Option<MetricExcludedSurface>,
    },
    Measured {
        thresholds: MetricRepeatThresholds,
        stretches: Vec<CoreStretchRepeats>,
    },
}

/// `xt_metrics::StretchRepeats`, field for field.
#[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
struct CoreStretchRepeats {
    start_uuid: String,
    end_uuid: String,
    start: String,
    end: String,
    duration_ms: u64,
    active_duration_ms: u64,
    repeats: MetricRepeatDensity,
    circling: Option<bool>,
}

/// Set M-20's answer beside M-09's, or refuse when they are not about the
/// same stretches.
fn combine(
    stretches: CoreStretches,
    repeats: CoreRepeats,
) -> Result<MetricSessionStretches, crate::state::StateError> {
    use crate::state::StateError::MetricDisagreement;
    match (stretches, repeats) {
        (CoreStretches::Missing, CoreRepeats::Missing) => Ok(MetricSessionStretches::Missing),
        (
            CoreStretches::Unmeasured { excluded_surface },
            CoreRepeats::Unmeasured {
                excluded_surface: surface,
            },
        ) if excluded_surface == surface => {
            Ok(MetricSessionStretches::Unmeasured { excluded_surface })
        }
        (
            CoreStretches::Measured { stretches },
            CoreRepeats::Measured {
                thresholds,
                stretches: repeats,
            },
        ) if stretches.len() == repeats.len() => Ok(MetricSessionStretches::Measured {
            stretches: stretches
                .into_iter()
                .zip(repeats)
                .map(|(stretch, repeat)| {
                    let same = stretch.start_uuid == repeat.start_uuid
                        && stretch.end_uuid == repeat.end_uuid
                        && stretch.start == repeat.start
                        && stretch.end == repeat.end
                        && stretch.duration_ms == repeat.duration_ms;
                    same.then_some(MetricSessionStretch {
                        start_uuid: stretch.start_uuid,
                        end_uuid: stretch.end_uuid,
                        start: stretch.start,
                        end: stretch.end,
                        duration_ms: stretch.duration_ms,
                        first_tool: stretch.first_tool,
                        active_duration_ms: repeat.active_duration_ms,
                        repeats: repeat.repeats,
                        circling: repeat.circling,
                    })
                    .ok_or(MetricDisagreement)
                })
                .collect::<Result<_, _>>()?,
            repeat_thresholds: thresholds,
        }),
        _ => Err(MetricDisagreement),
    }
}

/// One session's stretches over one window preset, as a fixture carries them.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct FixtureSessionStretches {
    pub window_days: u32,
    pub session_id: String,
    pub stretches: MetricSessionStretches,
}

/// The stretches of every session a fixture lists, for every window preset,
/// read through the same command path and pinned clock as the fixture's pages
/// — so a fixture answers the same request the native command does.
pub fn fixture_session_stretches(
    path: &std::path::Path,
    now_ms: i64,
) -> Result<Vec<FixtureSessionStretches>, crate::state::StateError> {
    let metrics = xt_metrics::MetricsDb::open(path)?;
    let mut out = Vec::new();
    for page in fixture_session_pages(path, now_ms)? {
        for row in page.rows {
            out.push(FixtureSessionStretches {
                window_days: page.window.days,
                stretches: session_stretches(&metrics, page.window.days, now_ms, &row.id)?,
                session_id: row.id,
            });
        }
    }
    Ok(out)
}

/// Existing fixture export paths supply their pinned clock; one page per window
/// preset, so a fixture answers the same requests the native command does.
pub fn fixture_session_pages(
    path: &std::path::Path,
    now_ms: i64,
) -> Result<Vec<SessionPage>, crate::state::StateError> {
    let metrics = xt_metrics::MetricsDb::open(path)?;
    crate::dashboard::WINDOW_PRESETS
        .into_iter()
        .map(|days| {
            session_page(
                &metrics,
                days,
                now_ms,
                jiff::tz::TimeZone::UTC,
                MetricClock::Fixture,
                SessionQuery::default(),
            )
        })
        .collect()
}

#[cfg(test)]
mod combine_tests {
    use super::*;
    use crate::state::StateError;

    fn m09(start_uuid: &str, duration_ms: u64) -> CoreStretch {
        CoreStretch {
            start_uuid: start_uuid.into(),
            end_uuid: "end".into(),
            start: "2026-09-07T12:00:00Z".into(),
            end: "2026-09-07T12:04:00Z".into(),
            duration_ms,
            first_tool: None,
        }
    }
    fn m20(start_uuid: &str, duration_ms: u64) -> CoreStretchRepeats {
        CoreStretchRepeats {
            start_uuid: start_uuid.into(),
            end_uuid: "end".into(),
            start: "2026-09-07T12:00:00Z".into(),
            end: "2026-09-07T12:04:00Z".into(),
            duration_ms,
            active_duration_ms: 1,
            repeats: MetricRepeatDensity::Unknown {
                reason: MetricUnknownRepeats::MissingKey,
            },
            circling: None,
        }
    }
    fn thresholds() -> MetricRepeatThresholds {
        MetricRepeatThresholds {
            active_ms: 240_000,
            repeats: 5,
        }
    }
    fn measured(stretches: Vec<CoreStretchRepeats>) -> CoreRepeats {
        CoreRepeats::Measured {
            thresholds: thresholds(),
            stretches,
        }
    }
    fn refused(stretches: CoreStretches, repeats: CoreRepeats) -> bool {
        matches!(
            combine(stretches, repeats),
            Err(StateError::MetricDisagreement)
        )
    }

    #[test]
    fn agreeing_reports_are_combined_in_m09_s_order() {
        let combined = combine(
            CoreStretches::Measured {
                stretches: vec![m09("b", 2), m09("a", 1)],
            },
            measured(vec![m20("b", 2), m20("a", 1)]),
        )
        .unwrap();
        let MetricSessionStretches::Measured {
            stretches,
            repeat_thresholds,
        } = combined
        else {
            panic!("measured");
        };
        assert_eq!(repeat_thresholds, thresholds());
        let order: Vec<_> = stretches.iter().map(|s| s.start_uuid.as_str()).collect();
        assert_eq!(order, ["b", "a"]);
        assert!(matches!(
            combine(CoreStretches::Missing, CoreRepeats::Missing),
            Ok(MetricSessionStretches::Missing)
        ));
    }

    #[test]
    fn reports_that_are_not_about_the_same_stretches_are_refused() {
        let one = || CoreStretches::Measured {
            stretches: vec![m09("a", 1)],
        };
        // Another state.
        assert!(refused(one(), CoreRepeats::Missing));
        assert!(refused(CoreStretches::Missing, measured(vec![])));
        assert!(refused(
            CoreStretches::Unmeasured {
                excluded_surface: None
            },
            measured(vec![])
        ));
        // Another excluded surface.
        assert!(refused(
            CoreStretches::Unmeasured {
                excluded_surface: None
            },
            CoreRepeats::Unmeasured {
                excluded_surface: Some(MetricExcludedSurface {
                    host: "claude".into(),
                    surface: None,
                    qualifying_sessions: 3,
                    degenerate_sessions: 3,
                }),
            }
        ));
        // Another number of stretches.
        assert!(refused(one(), measured(vec![])));
        assert!(refused(one(), measured(vec![m20("a", 1), m20("b", 1)])));
        // Another stretch at the same place, or the same one of another length.
        assert!(refused(one(), measured(vec![m20("b", 1)])));
        assert!(refused(one(), measured(vec![m20("a", 2)])));
        for field in 0..3 {
            let mut other = m20("a", 1);
            match field {
                0 => other.end_uuid = "other".into(),
                1 => other.start = "2026-09-07T12:00:01Z".into(),
                _ => other.end = "2026-09-07T12:04:01Z".into(),
            }
            assert!(refused(one(), measured(vec![other])), "{field}");
        }
    }
}
