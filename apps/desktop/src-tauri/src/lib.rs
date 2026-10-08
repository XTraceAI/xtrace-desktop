#[cfg(all(feature = "fixtures", not(debug_assertions)))]
compile_error!("fixtures feature is forbidden in release builds");

use std::sync::Arc;
use tauri::{Emitter, Manager};
mod account_usage;
mod account_usage_claude;
mod account_usage_claude_local;
mod account_usage_claude_probe;
mod account_usage_claude_screen;
mod account_usage_codex;
mod account_usage_dto;
mod account_usage_history;
mod account_usage_process;
pub mod dashboard;
mod dashboard_dto;
pub mod dto;
mod earlier_install;
pub mod environment;
mod environment_dto;
pub mod gh_cli;
pub mod hook_names;
pub mod human_break;
pub mod live_codex_status;
mod local_updates;
pub mod native_index;
pub mod pr_analytics;
mod pr_analytics_dto;
mod pr_dto;
mod pr_effort_dto;
pub mod pr_refresh;
pub mod privacy;
pub mod rule_activity;
mod rule_activity_dto;
pub mod session_compactions;
mod session_source_dto;
pub mod session_titles;
pub mod startup_failure;
pub mod state;
pub mod today;
pub mod transcript_dto;
pub mod transcript_reads;
pub mod tray;
pub mod typing_speed;
mod update_config;
mod updates;
mod window_controls;

/// The event carrying every native index status change to the frontend.
pub const NATIVE_INDEX_EVENT: &str = "native-index://status";
/// The event announcing that a pull-request refresh committed a change. It is
/// emitted only after a committed write, never for a batch that changed
/// nothing.
pub const PR_REFRESH_EVENT: &str = "prs://refreshed";
/// Published, with no payload, whenever the automatic pull-request check
/// starts, finishes or pauses. The frontend reads `prs_auto_check_status` and
/// the Dashboard reports again.
pub const PR_AUTO_CHECK_EVENT: &str = "prs://auto-check";
/// Set to `off` to switch the automatic pull-request check off for a launch.
/// The manual refresh keeps working.
pub const PR_AUTO_CHECK_ENV: &str = "XTRACE_PR_AUTO_CHECK";
/// Published only after a committed purge cleared stored content.
pub const CONTENT_PURGED_EVENT: &str = "store://content-purged";
/// Published once after a background pass of the native index committed a
/// change to stored sub-session relations (a new relation, or one first
/// withheld as conflicted), with no payload. It re-reads the Sessions list,
/// the exact session row and the Dashboard reports, never the index status.
pub const SESSION_CREATIONS_EVENT: &str = "sessions://creations-changed";

/// The exact private mode is intercepted before Tauri, fixtures, or the
/// database start. No other argument combination can enter this mode.
pub fn claude_usage_helper_exit_code() -> Option<i32> {
    if is_claude_helper_args(std::env::args_os().skip(1)) {
        Some(account_usage_claude::run_one_shot())
    } else {
        None
    }
}

fn is_claude_helper_args(mut args: impl Iterator<Item = std::ffi::OsString>) -> bool {
    args.next()
        .is_some_and(|arg| arg == account_usage_claude::HELPER_FLAG)
        && args.next().is_none()
}

#[cfg(test)]
#[test]
fn private_claude_helper_requires_exact_single_argument() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
    };
    assert!(is_claude_helper_args(
        args(&[account_usage_claude::HELPER_FLAG]).into_iter()
    ));
    assert!(!is_claude_helper_args(
        args(&[account_usage_claude::HELPER_FLAG, "--fixture=F1"]).into_iter()
    ));
    assert!(!is_claude_helper_args(
        args(&["--fixture=F1", account_usage_claude::HELPER_FLAG]).into_iter()
    ));
    assert!(!is_claude_helper_args(args(&[]).into_iter()));
}

#[tauri::command]
fn app_info(state: tauri::State<'_, state::AppState>) -> dto::AppInfo {
    state.app_info()
}

/// The thirteen read commands and the three Settings writes run their state call on
/// the blocking pool, not on the thread that delivered the request (on macOS,
/// the main thread running the webview's URL-scheme handler). Each command
/// owns the handle and its arguments; the state is resolved and its lock taken
/// only on the worker, so the database mutex and shutdown's admission gate
/// still serialize every caller.
async fn on_state_worker<T: Send + 'static>(
    app: tauri::AppHandle,
    failed: &'static str,
    run: impl FnOnce(&state::AppState) -> Result<T, state::StateError> + Send + 'static,
) -> Result<T, String> {
    on_worker(failed, move || {
        let state = app
            .try_state::<state::AppState>()
            .ok_or(state::StateError::Closed)?;
        run(&state)
    })
    .await
}

async fn read_on_worker<T: Send + 'static>(
    app: tauri::AppHandle,
    read: impl FnOnce(&state::AppState) -> Result<T, state::StateError> + Send + 'static,
) -> Result<T, String> {
    on_state_worker(app, READ_WORKER_FAILED, read).await
}

async fn write_on_worker<T: Send + 'static>(
    app: tauri::AppHandle,
    write: impl FnOnce(&state::AppState) -> Result<T, state::StateError> + Send + 'static,
) -> Result<T, String> {
    on_state_worker(app, WRITE_WORKER_FAILED, write).await
}

/// A worker that panicked or was cancelled reports this instead of a result.
const READ_WORKER_FAILED: &str = "application database read did not complete";
/// A write worker that panicked or was cancelled: the store transaction's own
/// outcome is not reported and nothing is retried.
const WRITE_WORKER_FAILED: &str = "application database write did not report its outcome";

async fn on_worker<T: Send + 'static>(
    failed: &'static str,
    run: impl FnOnce() -> Result<T, state::StateError> + Send + 'static,
) -> Result<T, String> {
    match tauri::async_runtime::spawn_blocking(run).await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err(failed.into()),
    }
}

#[tauri::command]
async fn db_counts(app: tauri::AppHandle) -> Result<dto::DbCounts, String> {
    read_on_worker(app, |state| state.db_counts()).await
}

#[tauri::command]
async fn live_sessions_read(
    app: tauri::AppHandle,
    session_ids: Vec<String>,
    view_id: Option<String>,
) -> Result<dto::LiveSessionSnapshot, String> {
    read_on_worker(app, move |state| {
        state.live_sessions_read(&session_ids, view_id.as_deref())
    })
    .await
}

#[tauri::command]
async fn live_sessions_release(app: tauri::AppHandle, view_id: String) -> Result<(), String> {
    read_on_worker(app, move |state| state.live_sessions_release(&view_id)).await
}

#[tauri::command]
async fn account_usage_read(app: tauri::AppHandle) -> dto::AccountUsage {
    // No AppState/database lock is held while provider reads are in flight.
    let service = app
        .state::<Arc<account_usage::AccountUsageService>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || service.read())
        .await
        .unwrap_or_else(|_| dto::AccountUsage {
            claude: account_usage::failed(dto::AccountUsageIssue::SourceUnavailable),
            codex: account_usage::failed(dto::AccountUsageIssue::SourceUnavailable),
        })
}

#[tauri::command]
async fn account_usage_refresh_claude(
    app: tauri::AppHandle,
    webview_window: tauri::WebviewWindow,
) -> Result<dto::AccountUsage, String> {
    if webview_window.label() != tray::MAIN_LABEL {
        return Err("only the main window may refresh Claude usage".into());
    }
    if app.state::<state::AppState>().app_info().fixture.is_some() {
        return Ok(account_usage::fixture_result());
    }
    let service = app
        .state::<Arc<account_usage::AccountUsageService>>()
        .inner()
        .clone();
    Ok(
        tauri::async_runtime::spawn_blocking(move || service.refresh_claude())
            .await
            .unwrap_or_else(|_| dto::AccountUsage {
                claude: account_usage::failed(dto::AccountUsageIssue::SourceUnavailable),
                codex: account_usage::failed(dto::AccountUsageIssue::SourceUnavailable),
            }),
    )
}

#[tauri::command]
async fn sessions_list(
    app: tauri::AppHandle,
    sort: Option<xt_store::session_list::SessionSort>,
    search: String,
    hosts: Option<Vec<String>>,
    with_prs: bool,
    after: Option<String>,
    window_days: u32,
) -> Result<dto::SessionPage, String> {
    read_on_worker(app, move |state| {
        let hosts: Option<Vec<&str>> = hosts
            .as_ref()
            .map(|hosts| hosts.iter().map(String::as_str).collect());
        state.sessions_query(
            dto::SessionQuery {
                search: &search,
                sort: sort.unwrap_or_default(),
                hosts: hosts.as_deref(),
                with_prs,
                after: after.as_deref(),
            },
            window_days,
        )
    })
    .await
}

#[tauri::command]
async fn metrics_dashboard(
    app: tauri::AppHandle,
    window_days: u32,
) -> Result<dto::DashboardMetrics, String> {
    read_on_worker(app, move |state| state.metrics_dashboard(window_days)).await
}

#[tauri::command]
async fn metrics_environment(
    app: tauri::AppHandle,
    window_days: u32,
) -> Result<dto::EnvironmentMetrics, String> {
    read_on_worker(app, move |state| state.metrics_environment(window_days)).await
}

/// Read only saved Claude summary details for the report's pinned window.
#[tauri::command]
async fn environment_hook_names(
    app: tauri::AppHandle,
    window_days: u32,
    window_end_ms: i64,
    read_id: String,
) -> Result<dto::HookNames, String> {
    on_worker(READ_WORKER_FAILED, move || {
        let reads = app
            .try_state::<hook_names::HookNameReads>()
            .ok_or(state::StateError::Closed)?;
        let read = reads
            .begin(&read_id)
            .map_err(|_| state::StateError::ReadRefused)?;
        let state = app
            .try_state::<state::AppState>()
            .ok_or(state::StateError::Closed)?;
        state.environment_hook_names(window_days, window_end_ms, read.token())
    })
    .await
}

#[tauri::command]
fn cancel_environment_hook_names(
    reads: tauri::State<'_, hook_names::HookNameReads>,
    read_id: String,
) {
    reads.cancel(&read_id);
}

#[tauri::command]
async fn tokens_by_host(
    app: tauri::AppHandle,
    window_days: u32,
) -> Result<dto::TokensByHost, String> {
    read_on_worker(app, move |state| state.tokens_by_host(window_days)).await
}

/// Exactly one session's row, or nothing. Named, never searched for.
#[tauri::command]
async fn session_row(
    app: tauri::AppHandle,
    session_id: String,
    window_days: u32,
) -> Result<Option<dto::SessionRow>, String> {
    read_on_worker(app, move |state| {
        state.session_row(&session_id, window_days)
    })
    .await
}

/// M-09's stretches for exactly one session over the selected window, each
/// with M-20's repeat measurement of the same stretch.
#[tauri::command]
async fn session_stretches(
    app: tauri::AppHandle,
    session_id: String,
    window_days: u32,
) -> Result<dto::MetricSessionStretches, String> {
    read_on_worker(app, move |state| {
        state.session_stretches(&session_id, window_days)
    })
    .await
}

/// One activity-lane span's detail, read when its bar is pointed at or
/// focused: the most-used tool, output tokens and the last message a person
/// typed in or before it. Metadata from the index; the message's words are the
/// index's own when content retention kept them, otherwise read from the
/// session's original source in memory, never stored. `read_id` is the view's
/// name for this read, which it cancels when the read is abandoned.
#[tauri::command]
async fn metrics_span_detail(
    app: tauri::AppHandle,
    session_id: String,
    start_ms: i64,
    end_ms: i64,
    read_id: String,
) -> Result<dto::DashboardSpanDetail, String> {
    on_worker(READ_WORKER_FAILED, move || {
        let reads = app
            .try_state::<dashboard::SpanDetailReads>()
            .ok_or(state::StateError::Closed)?;
        let index = app
            .try_state::<native_index::NativeIndex>()
            .ok_or(state::StateError::Closed)?;
        let state = app
            .try_state::<state::AppState>()
            .ok_or(state::StateError::Closed)?;
        // Only the file read takes a slot; the measured detail never waits.
        state.span_detail(
            &session_id,
            start_ms,
            end_ms,
            &reads,
            &read_id,
            index.detail_readers(),
        )
    })
    .await
}
/// Abandon a span detail read. Cancelling one that has not begun is not a
/// mistake: it begins cancelled and returns none of the words.
#[tauri::command]
fn cancel_metrics_span_detail(
    reads: tauri::State<'_, dashboard::SpanDetailReads>,
    read_id: String,
) {
    reads.cancel(&read_id);
}

/// One session's original local source, read on demand.
///
/// Async so the read runs off the main thread: a session is read whole, and a
/// window that stopped drawing while a large one loaded would be the cost of
/// showing it. `read_id` is the view's own name for this open, which it
/// cancels when the open is replaced or abandoned. A Codex or Cursor session
/// is read through the index's pinned reader, verified and resolved for this
/// open under its own token.
#[tauri::command]
async fn session_transcript(
    state: tauri::State<'_, state::AppState>,
    reads: tauri::State<'_, transcript_reads::TranscriptReads>,
    index: tauri::State<'_, native_index::NativeIndex>,
    session_id: String,
    read_id: String,
) -> Result<dto::SessionSourceStatus, String> {
    let read = reads
        .begin(&read_id)
        .map_err(|_| state::StateError::ReadRefused.to_string())?;
    state
        .session_transcript(&session_id, read.token(), index.detail_readers())
        .map_err(|error| error.to_string())
}

/// The host's own title for each named session that has one, read from its
/// original local source for the rows a view is showing and never stored.
///
/// `session_ids` are canonical identifiers, at most one Sessions page of them;
/// `read_id` is the view's own name for this read, which it cancels when the
/// rows it asked about are no longer the rows it shows. Resolution happens
/// against the index; the sources are read on the blocking pool with the
/// database released.
#[tauri::command]
async fn session_titles(
    app: tauri::AppHandle,
    session_ids: Vec<String>,
    read_id: String,
) -> Result<dto::SessionTitles, String> {
    on_worker(READ_WORKER_FAILED, move || {
        let reads = app
            .try_state::<session_titles::TitleReads>()
            .ok_or(state::StateError::Closed)?;
        let read = reads
            .begin(&read_id)
            .map_err(|_| state::StateError::ReadRefused)?;
        let state = app
            .try_state::<state::AppState>()
            .ok_or(state::StateError::Closed)?;
        state.session_titles(&session_ids, read.token())
    })
    .await
}

/// Abandon a title read, whether or not it has started.
#[tauri::command]
fn cancel_session_titles(reads: tauri::State<'_, session_titles::TitleReads>, read_id: String) {
    reads.cancel(&read_id);
}

#[tauri::command]
async fn session_compactions(
    app: tauri::AppHandle,
    session_ids: Vec<String>,
    read_id: String,
) -> Result<dto::SessionCompactions, String> {
    on_worker(READ_WORKER_FAILED, move || {
        let reads = app
            .try_state::<session_compactions::CompactionReads>()
            .ok_or(state::StateError::Closed)?;
        let read = reads
            .begin(&read_id)
            .map_err(|_| state::StateError::ReadRefused)?;
        let state = app
            .try_state::<state::AppState>()
            .ok_or(state::StateError::Closed)?;
        state.session_compactions(&session_ids, read.token())
    })
    .await
}
#[tauri::command]
fn cancel_session_compactions(
    reads: tauri::State<'_, session_compactions::CompactionReads>,
    read_id: String,
) {
    reads.cancel(&read_id);
}

/// Abandon a transcript read. Cancelling one that has not started yet is not a
/// mistake: it begins cancelled and returns nothing.
#[tauri::command]
fn cancel_session_transcript(
    reads: tauri::State<'_, transcript_reads::TranscriptReads>,
    read_id: String,
) {
    reads.cancel(&read_id);
}

/// One bounded snapshot of the default local rulebook's recorded activity
/// over the fixed trailing 14 days. `read_id` is the view's own name for this
/// read and the only thing it sends: no root, path, range or filter. One read
/// runs at a time; another is answered `busy` at once without reading. The
/// read runs on the blocking pool and holds no database lock. Every outcome,
/// including a refused or failed read, is a typed state, never an error text.
#[tauri::command]
async fn rule_activity_read<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    read_id: String,
) -> dto::RuleActivityResult {
    match app.try_state::<rule_activity::RuleActivityService>() {
        Some(service) => rule_activity::read_on_worker(&service, &read_id).await,
        None => dto::RuleActivityResult::Closed,
    }
}

/// Abandon a rule activity read, whether or not it has started.
#[tauri::command]
fn rule_activity_cancel(
    service: tauri::State<'_, rule_activity::RuleActivityService>,
    read_id: String,
) {
    service.cancel(&read_id);
}

/// Every stored pull request a session still links, with the refresh status
/// storage holds for it. Reading this starts nothing.
#[tauri::command]
async fn prs_list(app: tauri::AppHandle) -> Result<dto::PrList, String> {
    read_on_worker(app, |state| state.pr_list()).await
}

/// The PRs page's cached per-PR linked-session report for one preset and
/// confidence mode. A local read only: nothing refreshes or reads a source.
#[tauri::command]
async fn prs_analytics(
    app: tauri::AppHandle,
    window_days: u32,
    confirmed_only: bool,
) -> Result<dto::PrAnalyticsPage, String> {
    read_on_worker(app, move |state| {
        state.pr_analytics(window_days, confirmed_only)
    })
    .await
}

/// One page of exactly one pull request's linked sessions over the displayed
/// report's window, pinned to that report's `window.end_ms`.
#[tauri::command]
async fn prs_sessions(
    app: tauri::AppHandle,
    repository: String,
    number: u64,
    confirmed_only: bool,
    window_days: u32,
    window_end_ms: i64,
    after: Option<String>,
) -> Result<dto::SessionPage, String> {
    read_on_worker(app, move |state| {
        state.pr_sessions(pr_analytics::PrSessionsRequest {
            repository: &repository,
            number,
            confirmed_only,
            window_days,
            window_end_ms,
            after: after.as_deref(),
        })
    })
    .await
}

/// Refresh a selection of stored pull requests. The identifiers are storage's
/// own; no URL, repository or executable crosses this boundary. An automatic
/// check that is running gives way to this request.
///
/// A batch blocks for as long as its budget allows, so it runs on the command
/// thread pool rather than the main thread: the window stays responsive and
/// `prs_refresh_cancel` can still reach it.
#[tauri::command(async)]
fn prs_refresh(
    state: tauri::State<'_, state::AppState>,
    refresh: tauri::State<'_, pr_refresh::PrRefreshService>,
    ids: Vec<i64>,
) -> Result<dto::PrRefreshReport, String> {
    refresh
        .refresh(&state, &ids)
        .map_err(|error| error.to_string())
}

/// Ask the running refresh to stop. False means none was running.
#[tauri::command]
fn prs_refresh_cancel(refresh: tauri::State<'_, pr_refresh::PrRefreshService>) -> bool {
    refresh.cancel()
}

/// Where the automatic pull-request check stands. Reading this starts nothing.
#[tauri::command]
fn prs_auto_check_status(
    refresh: tauri::State<'_, pr_refresh::PrRefreshService>,
) -> dto::PrAutoCheckStatus {
    refresh.auto_status()
}

/// The Dashboard was shown: ask the automatic check to look soon. It decides
/// for itself whether anything is due; this never waits for it.
#[tauri::command]
fn prs_auto_check(
    refresh: tauri::State<'_, pr_refresh::PrRefreshService>,
    worker: tauri::State<'_, pr_refresh::AutoCheckWorker>,
) -> dto::PrAutoCheckStatus {
    worker.wake();
    refresh.auto_status()
}

/// Today's local-calendar summary for the tray. The window is the local day,
/// not a Dashboard range preset, so it takes no `window_days`.
#[tauri::command]
async fn today_summary(app: tauri::AppHandle) -> Result<today::TodaySummary, String> {
    read_on_worker(app, |state| state.today()).await
}

/// Popover-only: any other window is refused.
#[tauri::command]
fn tray_hide(webview_window: tauri::WebviewWindow) -> Result<(), String> {
    popover(&webview_window).map(tray::dismiss_popover)
}

/// Popover-only: hide it and bring the existing main window forward.
#[tauri::command]
fn tray_open_main(
    app: tauri::AppHandle,
    webview_window: tauri::WebviewWindow,
) -> Result<(), String> {
    tray::hide_popover(popover(&webview_window)?);
    tray::show_main(&app);
    Ok(())
}

/// Popover-only: whether it is on screen, for a view that mounted after a show.
#[tauri::command]
fn tray_visible(webview_window: tauri::WebviewWindow) -> Result<bool, String> {
    popover(&webview_window)?
        .is_visible()
        .map_err(|error| error.to_string())
}

fn popover(window: &tauri::WebviewWindow) -> Result<&tauri::WebviewWindow, String> {
    if window.label() == tray::TRAY_LABEL {
        Ok(window)
    } else {
        Err("only the tray popover may do this".into())
    }
}

#[tauri::command]
fn native_index_status(
    index: tauri::State<'_, native_index::NativeIndex>,
) -> dto::NativeIndexStatus {
    index.status()
}

#[tauri::command]
async fn content_retention(app: tauri::AppHandle) -> Result<privacy::ContentRetention, String> {
    read_on_worker(app, |state| state.content_retention()).await
}

#[tauri::command]
async fn set_content_retention(
    app: tauri::AppHandle,
    mode: privacy::ContentRetention,
) -> Result<privacy::ContentRetention, String> {
    write_on_worker(app, move |state| state.set_content_retention(mode)).await
}

/// Words per minute for the typing estimate; 40 when nothing was saved.
#[tauri::command]
async fn typing_speed(app: tauri::AppHandle) -> Result<u32, String> {
    read_on_worker(app, |state| state.typing_speed()).await
}

/// A whole number of words per minute, 1 to 300; anything else is refused
/// before storage is touched. Returns the speed read back after commit.
#[tauri::command]
async fn set_typing_speed(app: tauri::AppHandle, wpm: u32) -> Result<u32, String> {
    write_on_worker(app, move |state| state.set_typing_speed(wpm)).await
}

/// Minutes between two of your messages that still count as one stretch of
/// human time; 60 when nothing was saved.
#[tauri::command]
async fn human_break(app: tauri::AppHandle) -> Result<u32, String> {
    read_on_worker(app, |state| state.human_break()).await
}

/// A whole number of minutes, 5 to 240; anything else is refused before
/// storage is touched. Returns the length read back after commit.
#[tauri::command]
async fn set_human_break(app: tauri::AppHandle, minutes: u32) -> Result<u32, String> {
    write_on_worker(app, move |state| state.set_human_break(minutes)).await
}

/// Main window only: opens the fixed official test page in the system
/// browser. The renderer cannot name a URL, and the app never navigates.
#[tauri::command]
async fn open_typing_test(webview_window: tauri::WebviewWindow) -> Result<(), String> {
    if webview_window.label() != tray::MAIN_LABEL {
        return Err("only the main window may do this".into());
    }
    on_worker(OPEN_WORKER_FAILED, typing_speed::open_typing_test).await
}
const OPEN_WORKER_FAILED: &str = "the typing test could not be opened in the browser";

/// No URL argument: this only hands the compiled release page to the OS browser.
#[tauri::command]
async fn open_public_releases<R: tauri::Runtime>(
    webview_window: tauri::WebviewWindow<R>,
) -> Result<(), String> {
    local_updates::require_main_window(webview_window.label())?;
    let app = webview_window.app_handle();
    local_updates::require_local_updates(
        app.state::<state::AppState>().app_info().fixture.is_some(),
        updates::updates_enabled(app.clone()),
    )?;
    on_worker(
        "public releases could not be opened in the browser",
        local_updates::open_public_releases,
    )
    .await
}

/// The renderer confirms first; this clears registered content fields in one
/// store transaction and never touches original host files. The event is
/// published from the worker after the commit, once the lock is released.
#[tauri::command]
async fn purge_stored_content(app: tauri::AppHandle) -> Result<privacy::ContentPurge, String> {
    let publisher = app.clone();
    write_on_worker(app, move |state| {
        state.purge_stored_content(move || {
            let _ = publisher.emit(CONTENT_PURGED_EVENT, ());
        })
    })
    .await
}

/// The startup options from this launch's environment and arguments.
fn startup_options() -> Result<state::StartupOptions, state::StateError> {
    state::StartupOptions::parse(
        std::env::var_os("XTRACE_DATA_DIR").map(Into::into),
        std::env::var_os("XTRACE_FIXTURE"),
        std::env::args().skip(1),
    )?
    .with_native_environment(
        std::env::var_os("XTRACE_NATIVE_HOME"),
        std::env::var_os("XTRACE_PYTHON"),
    )?
    .with_github_cli(std::env::var_os(gh_cli::GH_ENV))
}

/// Copies the earlier install's data and screen settings when
/// [`earlier_install::decide`] says so. It runs as a plugin's setup because
/// Tauri initializes plugins while building the app, before it creates the
/// windows in `tauri.conf.json` and before `setup`; WebKit storage copied
/// any later would be copied under a running webview. Registered after the
/// single-instance plugin, so a second launch has already handed over to the
/// first and ended. Options that do not parse copy nothing here; `start`
/// reports them.
fn copy_earlier_install<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("earlier-install")
        .setup(|app, _| {
            if let Err(error) = copy_earlier_install_now(app) {
                // Nothing has started yet: no window to hide, no service to
                // stop. The alert is the one `fail_setup` shows.
                startup_failure::exit(&startup_failure::message(&error));
            }
            Ok(())
        })
        .build()
}

fn copy_earlier_install_now<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), earlier_install::CopyError> {
    let Ok(options) = startup_options() else {
        return Ok(());
    };
    let earlier_install::Decision::Copy(folders) = earlier_install::decide(&options, || {
        app.path()
            .app_data_dir()
            .map_err(|_| state::StateError::InvalidOption)
    })?
    else {
        return Ok(());
    };
    earlier_install::copy(&folders, earlier_install::earlier_app_running)?;
    // The screen settings are kept when they can be; without them the app
    // still starts, with the default theme and the welcome screen.
    if let Ok(home) = app.path().home_dir()
        && let Err(error) = earlier_install::copy_screen_settings(&home, &app.config().identifier)
    {
        eprintln!("XTrace could not copy the earlier screen settings: {error}");
    }
    Ok(())
}

/// Starts the app's services over the database startup selects. An error
/// here is shown to the person by `fail_setup`, never handed to Tauri, which
/// would end the app with a panic and nothing on screen.
fn start(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    updates::setup(app.handle())?;
    let options = startup_options()?;
    let python = options.python.clone();
    let github_cli = options.github_cli.clone();
    let state = state::AppState::build(
        options,
        || {
            app.path()
                .app_data_dir()
                .map_err(|_| state::StateError::InvalidOption)
        },
        || {
            app.path()
                .home_dir()
                .map_err(|_| state::StateError::InvalidOption)
        },
    )?;
    let publish: native_index::Publish = {
        let handle = app.handle().clone();
        Arc::new(move |status: &dto::NativeIndexStatus| {
            // A window not open yet, or already gone, drops the event;
            // the status remains queryable.
            let _ = handle.emit(NATIVE_INDEX_EVENT, status);
        })
    };
    let creations: native_index::PublishCreations = {
        let handle = app.handle().clone();
        Arc::new(move || {
            // A window not open yet, or already gone, drops the event;
            // the relations stay queryable.
            let _ = handle.emit(SESSION_CREATIONS_EVENT, ());
        })
    };
    let index = match (state.database_path(), state.native_home()) {
        // Fixture startup never selects live data: nothing is indexed.
        (Some(db), Some(home)) => native_index::NativeIndex::start_with_creations(
            native_index::NativeIndexOptions {
                home: home.to_path_buf(),
                db: db.to_path_buf(),
                bundle: app
                    .path()
                    .resource_dir()
                    .map_err(|_| state::StateError::InvalidOption)?
                    .join(native_index::BUNDLE_RESOURCE),
                python,
            },
            publish,
            creations,
        ),
        _ => {
            native_index::NativeIndex::disabled("fixture mode uses a disposable database", publish)
        }
    };
    let refreshed: pr_refresh::Publish = {
        let handle = app.handle().clone();
        Arc::new(move || {
            // A window not open yet, or already gone, drops the event;
            // the refreshed rows stay queryable.
            let _ = handle.emit(PR_REFRESH_EVENT, ());
        })
    };
    // Fixture startup never selects live data and never resolves or
    // runs the GitHub CLI: its refresh is synthetic, deterministic and
    // stamped with the fixture's own instant, over the disposable
    // database.
    let refresh = match state.fixture_now_ms() {
        Some(now_ms) => pr_refresh::PrRefreshService::fixture(now_ms, refreshed),
        None => pr_refresh::PrRefreshService::production(github_cli, refreshed),
    };
    let auto_checked: pr_refresh::Publish = {
        let handle = app.handle().clone();
        Arc::new(move || {
            let _ = handle.emit(PR_AUTO_CHECK_EVENT, ());
        })
    };
    let refresh = refresh.with_auto_publish(auto_checked);
    // The automatic check reads GitHub through the same service, so in
    // fixture mode it is the same synthetic source and never runs the
    // GitHub CLI. It runs on one background thread: on window focus,
    // when the Dashboard is shown and about once an hour.
    let auto_off = std::env::var_os(PR_AUTO_CHECK_ENV).is_some_and(|value| value == "off");
    let (refresh, auto_worker) = if auto_off {
        (
            refresh.without_auto_check(),
            pr_refresh::AutoCheckWorker::disabled(),
        )
    } else {
        let handle = app.handle().clone();
        let worker = pr_refresh::AutoCheckWorker::start(pr_refresh::AUTO_INTERVAL, move || {
            let (Some(state), Some(refresh)) = (
                handle.try_state::<state::AppState>(),
                handle.try_state::<pr_refresh::PrRefreshService>(),
            ) else {
                return true;
            };
            !matches!(refresh.auto_check(&state), pr_refresh::AutoRun::Stopped)
        });
        (refresh, worker)
    };
    // The default rulebook source under the native home startup
    // selected; none in fixture mode. Nothing is read until a view
    // asks.
    app.manage(rule_activity::RuleActivityService::new(state.native_home()));
    // Live startup reads Claude usage automatically through the user's
    // own Claude Code, from a probe folder in the app's data folder.
    // Fixture startup never starts a probe.
    let info = state.app_info();
    let usage = match (info.fixture.is_some(), app.path().home_dir()) {
        (false, Ok(home)) if !info.data_dir.is_empty() => {
            account_usage::AccountUsageService::with_claude_local(
                account_usage_claude_local::ClaudeUsagePaths::new(
                    std::path::Path::new(&info.data_dir),
                    &home,
                    std::env::var_os("CLAUDE_CONFIG_DIR").map(Into::into),
                ),
            )
        }
        _ => account_usage::AccountUsageService::default(),
    };
    let usage = Arc::new(usage);
    usage.start_automatic();
    app.manage(state);
    app.manage(usage);
    app.manage(index);
    app.manage(transcript_reads::TranscriptReads::default());
    app.manage(session_titles::TitleReads::default());
    app.manage(session_compactions::CompactionReads::default());
    app.manage(dashboard::SpanDetailReads::default());
    app.manage(hook_names::HookNameReads::default());
    app.manage(refresh);
    app.manage(auto_worker);
    tray::setup(app)?;
    Ok(())
}

/// A failed start: the main window is hidden, whatever started before the
/// failure is stopped as quitting stops it, and the alert is shown before the
/// app exits with a non-zero status. Setup runs on the main thread, where the
/// alert must run.
fn fail_setup(app: &tauri::App, error: &(dyn std::error::Error + 'static)) -> ! {
    if let Some(window) = app.get_webview_window(tray::MAIN_LABEL) {
        let _ = window.hide();
    }
    let failure = startup_failure::message(error);
    stop_services(app.handle());
    app.handle().cleanup_before_exit();
    startup_failure::exit(&failure)
}

/// Stops every service in the order quitting needs, each one only if it was
/// started.
fn stop_services(app: &tauri::AppHandle) {
    // Open transcript reads end first: each is cancelled, and
    // this waits (within its bound) until each has returned — a
    // pinned reader one started killed with its group and reaped
    // by its supervisor. The process exits as soon as this handler
    // returns, so without the wait a reader could outlive the app.
    if let Some(reads) = app.try_state::<transcript_reads::TranscriptReads>() {
        reads.shutdown();
    }
    // Title reads start no process; each ends at its next chunk.
    if let Some(reads) = app.try_state::<session_titles::TitleReads>() {
        reads.shutdown();
    }
    if let Some(reads) = app.try_state::<session_compactions::CompactionReads>() {
        reads.shutdown();
    }
    // A span read may run a pinned reader, as a transcript open does.
    if let Some(reads) = app.try_state::<dashboard::SpanDetailReads>() {
        reads.shutdown();
    }
    if let Some(reads) = app.try_state::<hook_names::HookNameReads>() {
        reads.shutdown();
    }
    // A rule activity read starts no process and holds no
    // database; it is cancelled and waited for within its bound.
    if let Some(service) = app.try_state::<rule_activity::RuleActivityService>() {
        service.close();
    }
    // Every worker stops before the database closes. A refresh in
    // progress is cancelled next, so its child is killed and its
    // last result persisted while storage is still open; storage
    // refuses a write after it closes either way.
    if let Some(worker) = app.try_state::<pr_refresh::AutoCheckWorker>() {
        worker.stop();
    }
    if let Some(refresh) = app.try_state::<pr_refresh::PrRefreshService>() {
        refresh.shutdown();
    }
    // The index stops before the database closes: its scan is
    // cancelled, a running reader killed and reaped, within a bound.
    if let Some(index) = app.try_state::<native_index::NativeIndex>() {
        index.shutdown();
    }
    if let Some(state) = app.try_state::<state::AppState>() {
        state.shutdown();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tray::show_main(app);
        }))
        .plugin(copy_earlier_install())
        .setup(|app| {
            if let Err(error) = start(app) {
                fail_setup(app, error.as_ref());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            tray::on_window_event(window, event);
            if window.label() != tray::TRAY_LABEL {
                window_controls::on_window_event(window, event);
            }
            // The main window gaining focus asks the automatic pull-request
            // check to look; it skips the run when one finished recently.
            if window.label() == tray::MAIN_LABEL
                && matches!(event, tauri::WindowEvent::Focused(true))
                && let Some(worker) = window.try_state::<pr_refresh::AutoCheckWorker>()
            {
                worker.wake();
            }
        })
        .invoke_handler(tauri::generate_handler![
            updates::updates_enabled,
            updates::restart_after_update,
            app_info,
            account_usage_read,
            account_usage_refresh_claude,
            db_counts,
            live_sessions_read,
            live_sessions_release,
            sessions_list,
            native_index_status,
            metrics_dashboard,
            metrics_environment,
            metrics_span_detail,
            cancel_metrics_span_detail,
            environment_hook_names,
            cancel_environment_hook_names,
            tokens_by_host,
            session_row,
            session_stretches,
            session_transcript,
            cancel_session_transcript,
            session_titles,
            cancel_session_titles,
            session_compactions,
            cancel_session_compactions,
            rule_activity_read,
            rule_activity_cancel,
            prs_list,
            prs_analytics,
            prs_sessions,
            prs_refresh,
            prs_refresh_cancel,
            prs_auto_check_status,
            prs_auto_check,
            today_summary,
            tray_hide,
            tray_open_main,
            tray_visible,
            content_retention,
            set_content_retention,
            purge_stored_content,
            typing_speed,
            set_typing_speed,
            open_typing_test,
            open_public_releases,
            human_break,
            set_human_break
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| startup_failure::exit(&startup_failure::message(&error)))
        .run(|app, event| {
            // The Dock icon brings the main window back: one that is minimized
            // or, with the menu-bar item on, one its close button hid. With
            // the item off a closed main window has ended the app, so there is
            // none to bring back and the Dock starts the app again.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show_main(app);
            }
            if matches!(event, tauri::RunEvent::Exit) {
                stop_services(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::{AppState, StartupOptions};
    use std::sync::Arc;
    use tauri::async_runtime::block_on;

    type Read =
        Arc<dyn Fn(&AppState) -> Result<serde_json::Value, state::StateError> + Send + Sync>;

    fn json<T: serde::Serialize>(value: T) -> serde_json::Value {
        serde_json::to_value(value).unwrap()
    }

    /// A session the F1 fixture holds, for the two reads that name one.
    const FIXTURE_SESSION: &str = "00000000-0000-4000-8000-000000000001";
    /// A pinned report end for the linked-session read; any valid anchor.
    const FIXTURE_ANCHOR: i64 = 1_788_825_600_000;

    /// The thirteen read commands' state calls, as their wrappers issue them.
    /// `today_summary` reads the local calendar day, so it ignores the range.
    fn reads(window_days: u32) -> Vec<(&'static str, Read)> {
        vec![
            (
                "db_counts",
                Arc::new(|s: &AppState| s.db_counts().map(json)),
            ),
            (
                "sessions_list",
                Arc::new(move |s: &AppState| {
                    s.sessions_list("", None, None, window_days).map(json)
                }),
            ),
            (
                "metrics_dashboard",
                Arc::new(move |s: &AppState| s.metrics_dashboard(window_days).map(json)),
            ),
            (
                "metrics_environment",
                Arc::new(move |s: &AppState| s.metrics_environment(window_days).map(json)),
            ),
            (
                "tokens_by_host",
                Arc::new(move |s: &AppState| s.tokens_by_host(window_days).map(json)),
            ),
            (
                "content_retention",
                Arc::new(|s: &AppState| s.content_retention().map(json)),
            ),
            (
                "typing_speed",
                Arc::new(|s: &AppState| s.typing_speed().map(json)),
            ),
            (
                "human_break",
                Arc::new(|s: &AppState| s.human_break().map(json)),
            ),
            (
                "session_row",
                Arc::new(move |s: &AppState| s.session_row(FIXTURE_SESSION, window_days).map(json)),
            ),
            (
                "session_stretches",
                Arc::new(move |s: &AppState| {
                    s.session_stretches(FIXTURE_SESSION, window_days).map(json)
                }),
            ),
            ("prs_list", Arc::new(|s: &AppState| s.pr_list().map(json))),
            (
                "prs_analytics",
                Arc::new(move |s: &AppState| s.pr_analytics(window_days, true).map(json)),
            ),
            (
                "prs_sessions",
                Arc::new(move |s: &AppState| {
                    s.pr_sessions(pr_analytics::PrSessionsRequest {
                        repository: "example/atlas",
                        number: 1,
                        confirmed_only: false,
                        window_days,
                        window_end_ms: FIXTURE_ANCHOR,
                        after: None,
                    })
                    .map(json)
                }),
            ),
            (
                "today_summary",
                Arc::new(|s: &AppState| s.today().map(json)),
            ),
        ]
    }

    fn on_worker_with(state: &Arc<AppState>, read: &Read) -> Result<serde_json::Value, String> {
        let (state, read) = (Arc::clone(state), Arc::clone(read));
        block_on(on_worker(READ_WORKER_FAILED, move || read(&state)))
    }

    fn live(root: &std::path::Path) -> Arc<AppState> {
        std::fs::create_dir_all(root.join("home")).unwrap();
        Arc::new(
            AppState::build(
                StartupOptions {
                    data_dir: Some(root.join("data")),
                    ..Default::default()
                },
                || panic!("default path must not be resolved"),
                || Ok(root.join("home")),
            )
            .unwrap(),
        )
    }

    #[cfg(all(debug_assertions, feature = "fixtures"))]
    fn fixture(root: &std::path::Path) -> Arc<AppState> {
        Arc::new(
            AppState::build(
                StartupOptions {
                    data_dir: Some(root.to_owned()),
                    fixture: Some("F1".into()),
                    ..Default::default()
                },
                || panic!("fixture must not resolve live directory"),
                || panic!("fixture must not resolve the home directory"),
            )
            .unwrap(),
        )
    }

    /// Off the calling thread, every read returns the DTO or error string the
    /// inline call does, for each range and for a range the metrics refuse.
    #[cfg(all(debug_assertions, feature = "fixtures"))]
    #[test]
    fn worker_reads_return_the_inline_fixture_dtos_and_errors() {
        let root = tempfile::TempDir::new().unwrap();
        let state = fixture(root.path());
        for window_days in [7, 14, 30, 0, 8] {
            for (name, read) in reads(window_days) {
                let inline = read(&state).map_err(|error| error.to_string());
                assert_eq!(
                    on_worker_with(&state, &read),
                    inline,
                    "{name} {window_days}"
                );
                // Today's window is the local day, never a range preset, so
                // an unsupported range must not refuse it.
                let refused = ![7, 14, 30].contains(&window_days)
                    && !matches!(
                        name,
                        "db_counts"
                            | "content_retention"
                            | "typing_speed"
                            | "human_break"
                            | "prs_list"
                            | "today_summary"
                    );
                assert_eq!(inline.is_err(), refused, "{name} {window_days}: {inline:?}");
                if refused {
                    assert_eq!(
                        inline.unwrap_err(),
                        "metric range must be 7, 14, or 30 days"
                    );
                }
            }
        }
        // The named-session reads compared a session that is there; one that
        // is not gives the same empty answer off the calling thread.
        assert!(state.session_row(FIXTURE_SESSION, 30).unwrap().is_some());
        let missing: [(&str, Read); 2] = [
            (
                "session_row",
                Arc::new(|s: &AppState| s.session_row("no-such-session", 7).map(json)),
            ),
            (
                "session_stretches",
                Arc::new(|s: &AppState| s.session_stretches("no-such-session", 7).map(json)),
            ),
        ];
        for (name, read) in missing {
            let inline = read(&state).map_err(|error| error.to_string());
            assert_eq!(on_worker_with(&state, &read), inline, "{name}");
        }
        assert_eq!(state.session_row("no-such-session", 7).unwrap(), None);
        state.shutdown();
    }

    #[test]
    fn worker_reads_after_shutdown_report_the_closed_database() {
        let root = tempfile::TempDir::new().unwrap();
        let state = live(root.path());
        for (name, read) in reads(7) {
            assert!(on_worker_with(&state, &read).is_ok(), "{name}");
        }
        state.shutdown();
        for (name, read) in reads(7) {
            assert_eq!(
                on_worker_with(&state, &read),
                Err("application database is closed".into()),
                "{name}"
            );
        }
    }

    #[test]
    fn reads_run_off_the_calling_thread() {
        let caller = std::thread::current().id();
        let worker = block_on(on_worker(READ_WORKER_FAILED, || {
            Ok(std::thread::current().id())
        }))
        .unwrap();
        assert_ne!(worker, caller);
    }

    /// While another caller holds the database lock, each read waits for it on
    /// a worker, not on the thread that issued it, and answers once released.
    #[test]
    fn reads_wait_for_a_held_lock_on_the_worker() {
        use std::sync::mpsc;
        let root = tempfile::TempDir::new().unwrap();
        let state = live(root.path());
        let (held, holding) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = Arc::clone(&state);
        let first = std::thread::spawn(move || {
            holder.with_store(|_| {
                held.send(()).unwrap();
                released.recv().unwrap();
                Ok(())
            })
        });
        holding.recv().unwrap();

        let caller = std::thread::current().id();
        let (ran_on, threads) = mpsc::channel();
        let (answered, answers) = mpsc::channel();
        for (name, read) in reads(7) {
            let (state, ran_on, answered) = (Arc::clone(&state), ran_on.clone(), answered.clone());
            // Spawning returns at once: nothing here waits for the lock.
            drop(tauri::async_runtime::spawn(async move {
                let answer = on_worker(READ_WORKER_FAILED, move || {
                    ran_on.send(std::thread::current().id()).unwrap();
                    read(&state)
                })
                .await;
                answered.send((name, answer)).unwrap();
            }));
        }
        drop((ran_on, answered));
        let count = reads(7).len();
        for _ in 0..count {
            let worker = threads
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            assert_ne!(worker, caller);
        }
        assert!(
            answers
                .recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "no read answers while the lock is held"
        );

        release.send(()).unwrap();
        first.join().unwrap().unwrap();
        for _ in 0..count {
            let (name, answer) = answers
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            assert!(answer.is_ok(), "{name}: {answer:?}");
        }
        state.shutdown();
    }

    /// A worker that dies reports a fixed error; one that dies holding the
    /// lock leaves later reads with the existing poisoned-database error.
    #[test]
    fn a_failed_worker_is_a_controlled_error() {
        let root = tempfile::TempDir::new().unwrap();
        let state = live(root.path());
        let failed: Result<(), String> =
            block_on(on_worker(READ_WORKER_FAILED, || panic!("worker failure")));
        assert_eq!(failed, Err(READ_WORKER_FAILED.into()));
        let holder = Arc::clone(&state);
        let failed: Result<(), String> = block_on(on_worker(READ_WORKER_FAILED, move || {
            holder.with_store(|_| panic!("worker failure"))
        }));
        assert_eq!(failed, Err(READ_WORKER_FAILED.into()));
        for (name, read) in reads(7) {
            assert_eq!(
                on_worker_with(&state, &read),
                Err("application database is unavailable".into()),
                "{name}"
            );
        }
        state.shutdown();
    }

    /// Shutdown closes admission before it waits, lets the admitted holder
    /// finish on its disposable database, and every read or write entry queued
    /// meanwhile, whichever check it reaches, returns the closed database.
    #[cfg(all(debug_assertions, feature = "fixtures"))]
    #[test]
    fn shutdown_waits_only_for_the_admitted_holder_and_closes_queued_entries() {
        use std::sync::mpsc;
        let root = tempfile::TempDir::new().unwrap();
        let state = fixture(root.path());
        let directory = std::path::PathBuf::from(state.app_info().data_dir);
        let database = directory.join("xtrace.db");

        let (held, holding) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = Arc::clone(&state);
        let path = database.clone();
        let first = tauri::async_runtime::spawn(on_worker(READ_WORKER_FAILED, move || {
            holder.with_store(|store| {
                held.send(()).unwrap();
                released.recv().unwrap();
                assert!(path.exists(), "the admitted holder keeps its database");
                Ok(store.counts()?.sessions)
            })
        }));
        holding.recv().unwrap();

        let queued_reads: Vec<_> = reads(7)
            .into_iter()
            .map(|(name, read)| {
                let state = Arc::clone(&state);
                (
                    name,
                    tauri::async_runtime::spawn(on_worker(READ_WORKER_FAILED, move || {
                        read(&state)
                    })),
                )
            })
            .collect();
        let queued_writes: Vec<_> = [
            {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    state
                        .set_content_retention(privacy::ContentRetention::FullContent)
                        .map(json)
                })
            },
            {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    state
                        .purge_stored_content(|| panic!("a refused purge publishes nothing"))
                        .map(json)
                })
            },
        ]
        .into_iter()
        .collect();
        let closer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || state.shutdown())
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !state.admission_closed() {
            assert!(
                std::time::Instant::now() < deadline,
                "shutdown closes admission"
            );
            std::thread::yield_now();
        }
        assert!(database.exists());
        // Entries arriving now do not wait behind the holder at all.
        for (name, read) in reads(7) {
            let (sent, answer) = mpsc::channel();
            let state = Arc::clone(&state);
            std::thread::spawn(move || sent.send(read(&state).map_err(|e| e.to_string())));
            assert_eq!(
                answer.recv_timeout(std::time::Duration::from_secs(10)),
                Ok(Err("application database is closed".into())),
                "{name}"
            );
        }

        release.send(()).unwrap();
        assert!(block_on(first).unwrap().unwrap() > 0);
        closer.join().unwrap();
        assert!(
            !directory.exists(),
            "shutdown removes the disposable database"
        );
        for (name, read) in queued_reads {
            assert_eq!(
                block_on(read).unwrap(),
                Err("application database is closed".into()),
                "{name}"
            );
        }
        for write in queued_writes {
            assert!(matches!(
                write.join().unwrap(),
                Err(state::StateError::Closed)
            ));
        }
        for (name, read) in reads(7) {
            assert_eq!(
                on_worker_with(&state, &read),
                Err("application database is closed".into()),
                "{name}"
            );
        }
        state.shutdown();
    }

    /// The pull-request and transcript entries take the same admission gate:
    /// once shutdown has begun they are refused at once rather than queued
    /// behind the holder shutdown is waiting for.
    #[test]
    fn pr_and_transcript_entries_are_refused_once_admission_closes() {
        use std::sync::mpsc;
        let root = tempfile::TempDir::new().unwrap();
        let state = live(root.path());

        let (held, holding) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = Arc::clone(&state);
        let first = std::thread::spawn(move || {
            holder.with_store(|_| {
                held.send(()).unwrap();
                released.recv().unwrap();
                Ok(())
            })
        });
        holding.recv().unwrap();
        let closer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || state.shutdown())
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !state.admission_closed() {
            assert!(
                std::time::Instant::now() < deadline,
                "shutdown closes admission"
            );
            std::thread::yield_now();
        }

        let entries: Vec<(&str, Read)> = vec![
            ("prs_list", Arc::new(|s: &AppState| s.pr_list().map(json))),
            (
                "prs_refresh targets",
                Arc::new(|s: &AppState| s.pr_refresh_targets(&[1]).map(|t| json(t.len()))),
            ),
            (
                "session_transcript",
                Arc::new(|s: &AppState| {
                    s.session_transcript(
                        "synthetic",
                        &xt_ingest::native::readers_cli::CancelToken::new(),
                        &native_index::DetailReaders::disabled(),
                    )
                    .map(json)
                }),
            ),
        ];
        for (name, entry) in entries {
            let (sent, answer) = mpsc::channel();
            let state = Arc::clone(&state);
            std::thread::spawn(move || sent.send(entry(&state).map_err(|e| e.to_string())));
            assert_eq!(
                answer.recv_timeout(std::time::Duration::from_secs(10)),
                Ok(Err("application database is closed".into())),
                "{name}"
            );
        }

        release.send(()).unwrap();
        first.join().unwrap().unwrap();
        closer.join().unwrap();
    }

    /// One synthetic session with content, written through the canonical writer.
    fn import_content(db: &std::path::Path) {
        use xt_ingest::{
            canonical::{Parsed, SourceContext, parse_line},
            writer::{WriteBatch, write_batch},
        };
        let line = serde_json::json!({"uuid":"archived","type":"assistant","parentUuid":"parent","timestamp":"2026-09-10T00:00:00Z","message":{"role":"assistant","model":"synthetic-model","content":[{"type":"text","text":"Synthetic transcript"},{"type":"tool_use","name":"Read","input":{"path":"synthetic.txt"}}],"usage":{"input_tokens":7,"output_tokens":3}}});
        let Parsed::Record(record) = parse_line(&line.to_string()).unwrap() else {
            panic!("synthetic record");
        };
        let context = SourceContext {
            conversation_id: Some("synthetic".into()),
            source: Some(xt_store::SessionSource::Transcript),
            ..Default::default()
        };
        let records = [*record];
        let batch = WriteBatch {
            namespace: None,
            context: &context,
            declared_host: None,
            records: &records,
            hook_summaries: &[],
            pr_witnesses: &[],
            title: Some("Synthetic title"),
            cwd: None,
            git_branch: None,
            keep_content: true,
            observed_at: 10,
            receipt: None,
            cursor: None,
            discovery: None,
            checkpoint: None,
        };
        write_batch(&mut xt_store::Store::open(db).unwrap(), &batch).unwrap();
    }

    fn has_content(db: &std::path::Path) -> bool {
        xt_store::Store::open(db)
            .unwrap()
            .records("synthetic")
            .unwrap()
            .iter()
            .any(|record| record.content_json.is_some())
    }

    /// The two write commands' state calls, as their wrappers issue them.
    fn set_on_worker(
        state: &Arc<AppState>,
        mode: privacy::ContentRetention,
    ) -> Result<serde_json::Value, String> {
        let state = Arc::clone(state);
        block_on(on_worker(WRITE_WORKER_FAILED, move || {
            state.set_content_retention(mode).map(json)
        }))
    }

    fn purge_on_worker(
        state: &Arc<AppState>,
        publish: impl FnOnce() + Send + 'static,
    ) -> Result<serde_json::Value, String> {
        let state = Arc::clone(state);
        block_on(on_worker(WRITE_WORKER_FAILED, move || {
            state.purge_stored_content(publish).map(json)
        }))
    }

    /// Off the calling thread, both writes return what the inline call does,
    /// and a purge that cleared content publishes once, after its commit and
    /// after the state lock is released; an empty purge publishes nothing.
    #[test]
    fn worker_writes_match_inline_writes_and_publish_after_commit() {
        use privacy::ContentRetention::{FullContent, MetadataOnly};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (inline_root, worker_root) = (
            tempfile::TempDir::new().unwrap(),
            tempfile::TempDir::new().unwrap(),
        );
        let (inline, worker) = (live(inline_root.path()), live(worker_root.path()));
        for mode in [FullContent, MetadataOnly, FullContent] {
            assert_eq!(
                set_on_worker(&worker, mode),
                inline
                    .set_content_retention(mode)
                    .map(json)
                    .map_err(|e| e.to_string())
            );
        }
        let worker_db = worker_root.path().join("data/xtrace.db");
        import_content(&inline_root.path().join("data/xtrace.db"));
        import_content(&worker_db);

        let inline_published = AtomicUsize::new(0);
        let expected = inline
            .purge_stored_content(|| {
                inline_published.fetch_add(1, Ordering::SeqCst);
            })
            .map(json)
            .map_err(|e| e.to_string());
        let published = Arc::new(AtomicUsize::new(0));
        let (count, state, db) = (
            Arc::clone(&published),
            Arc::clone(&worker),
            worker_db.clone(),
        );
        let purged = purge_on_worker(&worker, move || {
            assert!(!has_content(&db), "published after the commit");
            let (sent, answer) = std::sync::mpsc::channel();
            std::thread::spawn(move || sent.send(state.db_counts().is_ok()));
            assert_eq!(
                answer.recv_timeout(std::time::Duration::from_secs(10)),
                Ok(true),
                "published after the state lock is released"
            );
            count.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(purged, expected);
        assert_eq!(purged.unwrap()["invalidate_content"], true);
        assert_eq!(
            (
                published.load(Ordering::SeqCst),
                inline_published.load(Ordering::SeqCst)
            ),
            (1, 1)
        );

        let count = Arc::clone(&published);
        let empty = purge_on_worker(&worker, move || {
            count.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert_eq!(empty["invalidate_content"], false);
        assert_eq!(published.load(Ordering::SeqCst), 1);
        assert_eq!(
            block_on(on_worker(READ_WORKER_FAILED, {
                let worker = Arc::clone(&worker);
                move || worker.content_retention().map(json)
            })),
            Ok(json(FullContent))
        );
        inline.shutdown();
        worker.shutdown();
    }

    /// A refused or failed write publishes nothing: the closed database, the
    /// poisoned one, and a worker that dies, which is reported without
    /// claiming a rollback and is not retried.
    #[test]
    fn failed_worker_writes_publish_nothing() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = tempfile::TempDir::new().unwrap();
        let state = live(root.path());
        let db = root.path().join("data/xtrace.db");
        state
            .set_content_retention(privacy::ContentRetention::FullContent)
            .unwrap();
        import_content(&db);
        let published = Arc::new(AtomicUsize::new(0));

        let attempts = Arc::new(AtomicUsize::new(0));
        let tries = Arc::clone(&attempts);
        let died: Result<(), String> = block_on(on_worker(WRITE_WORKER_FAILED, move || {
            tries.fetch_add(1, Ordering::SeqCst);
            panic!("write worker failure")
        }));
        assert_eq!(died, Err(WRITE_WORKER_FAILED.into()));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        let holder = Arc::clone(&state);
        let poisoned: Result<(), String> = block_on(on_worker(WRITE_WORKER_FAILED, move || {
            holder.with_store(|_| panic!("write worker failure"))
        }));
        assert_eq!(poisoned, Err(WRITE_WORKER_FAILED.into()));
        let count = Arc::clone(&published);
        assert_eq!(
            purge_on_worker(&state, move || {
                count.fetch_add(1, Ordering::SeqCst);
            }),
            Err("application database is unavailable".into())
        );
        assert_eq!(
            set_on_worker(&state, privacy::ContentRetention::MetadataOnly),
            Err("application database is unavailable".into())
        );

        state.shutdown();
        let count = Arc::clone(&published);
        assert_eq!(
            purge_on_worker(&state, move || {
                count.fetch_add(1, Ordering::SeqCst);
            }),
            Err("application database is closed".into())
        );
        assert_eq!(
            set_on_worker(&state, privacy::ContentRetention::MetadataOnly),
            Err("application database is closed".into())
        );
        assert_eq!(published.load(Ordering::SeqCst), 0);
        assert!(has_content(&db));
        assert_eq!(
            xt_store::Store::open(&db)
                .unwrap()
                .retention_mode()
                .unwrap(),
            xt_store::retention::RetentionMode::FullContent
        );
    }

    /// The write admitted before shutdown commits; worker writes queued behind
    /// it are refused once admission closes, and nothing is published.
    #[test]
    fn shutdown_lets_the_admitted_write_commit_and_refuses_queued_writes() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        let root = tempfile::TempDir::new().unwrap();
        let state = live(root.path());
        let db = root.path().join("data/xtrace.db");
        state
            .set_content_retention(privacy::ContentRetention::FullContent)
            .unwrap();
        import_content(&db);

        let (held, holding) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = Arc::clone(&state);
        let admitted = tauri::async_runtime::spawn(on_worker(WRITE_WORKER_FAILED, move || {
            holder.with_store(|store| {
                store.set_retention_mode(xt_store::retention::RetentionMode::MetadataOnly)?;
                held.send(()).unwrap();
                released.recv().unwrap();
                Ok(())
            })
        }));
        holding.recv().unwrap();

        let published = Arc::new(AtomicUsize::new(0));
        let queued_set = {
            let state = Arc::clone(&state);
            tauri::async_runtime::spawn(on_worker(WRITE_WORKER_FAILED, move || {
                state
                    .set_content_retention(privacy::ContentRetention::FullContent)
                    .map(json)
            }))
        };
        let queued_purge = {
            let (state, count) = (Arc::clone(&state), Arc::clone(&published));
            tauri::async_runtime::spawn(on_worker(WRITE_WORKER_FAILED, move || {
                state
                    .purge_stored_content(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                    })
                    .map(json)
            }))
        };
        let closer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || state.shutdown())
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !state.admission_closed() {
            assert!(
                std::time::Instant::now() < deadline,
                "shutdown closes admission"
            );
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        assert_eq!(block_on(admitted).unwrap(), Ok(()));
        closer.join().unwrap();
        for queued in [queued_set, queued_purge] {
            assert_eq!(
                block_on(queued).unwrap(),
                Err("application database is closed".into())
            );
        }
        assert_eq!(published.load(Ordering::SeqCst), 0);
        assert_eq!(
            xt_store::Store::open(&db)
                .unwrap()
                .retention_mode()
                .unwrap(),
            xt_store::retention::RetentionMode::MetadataOnly,
            "the admitted write committed; the queued one did not"
        );
        assert!(has_content(&db), "the queued purge did not run");
    }

    /// The two rule activity commands through Tauri's own IPC dispatch: the
    /// view's `readId` is the only argument read, every answer is a typed
    /// state, and an extra argument naming a root is ignored.
    #[test]
    fn rule_activity_commands_answer_typed_states_over_ipc() {
        use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
        let home = tempfile::TempDir::new().unwrap();
        let ledger = xt_rulebook::activity::default_root(home.path()).join("ledger");
        std::fs::create_dir_all(&ledger).unwrap();
        std::fs::write(ledger.join("schema_version"), b"2\n").unwrap();
        std::fs::write(ledger.join("fires.jsonl"), b"").unwrap();
        let elsewhere = tempfile::TempDir::new().unwrap();

        let invoke =
            |app: &tauri::App<tauri::test::MockRuntime>, command: &str, body: serde_json::Value| {
                let window = app.get_webview_window("main").unwrap();
                get_ipc_response(
                    &window,
                    tauri::webview::InvokeRequest {
                        cmd: command.into(),
                        callback: tauri::ipc::CallbackFn(0),
                        error: tauri::ipc::CallbackFn(1),
                        url: "tauri://localhost".parse().unwrap(),
                        body: tauri::ipc::InvokeBody::Json(body),
                        headers: Default::default(),
                        invoke_key: INVOKE_KEY.to_string(),
                    },
                )
                .map(|body| body.deserialize::<serde_json::Value>().unwrap())
            };
        let build = |service: Option<rule_activity::RuleActivityService>| {
            let builder = mock_builder().invoke_handler(tauri::generate_handler![
                rule_activity_read,
                rule_activity_cancel
            ]);
            let builder = match service {
                Some(service) => builder.manage(service),
                None => builder,
            };
            let app = builder.build(mock_context(noop_assets())).unwrap();
            tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
                .build()
                .unwrap();
            app
        };

        let app = build(Some(rule_activity::RuleActivityService::new(Some(
            home.path(),
        ))));
        let read = invoke(
            &app,
            "rule_activity_read",
            serde_json::json!({"readId": "read-1", "root": elsewhere.path()}),
        )
        .unwrap();
        assert_eq!(
            (
                &read["state"],
                &read["read_id"],
                &read["counts"]["precision"]
            ),
            (&json("loaded"), &json("read-1"), &json("exact"))
        );
        assert!(
            !read.to_string().contains(home.path().to_str().unwrap()),
            "{read}"
        );
        assert_eq!(
            invoke(
                &app,
                "rule_activity_read",
                serde_json::json!({"readId": "a/b"})
            ),
            Ok(serde_json::json!({"state": "invalid_read_id"}))
        );
        assert_eq!(
            invoke(
                &app,
                "rule_activity_cancel",
                serde_json::json!({"readId": "read-2"})
            ),
            Ok(serde_json::Value::Null)
        );
        assert_eq!(
            invoke(
                &app,
                "rule_activity_read",
                serde_json::json!({"readId": "read-2"})
            ),
            Ok(serde_json::json!({"state": "interrupted", "reason": "cancelled"}))
        );
        assert!(invoke(&app, "rule_activity_read", serde_json::json!({})).is_err());
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        app.state::<rule_activity::RuleActivityService>().close();
        assert_eq!(
            invoke(
                &app,
                "rule_activity_read",
                serde_json::json!({"readId": "read-3"})
            ),
            Ok(serde_json::json!({"state": "closed"}))
        );

        // Fixture startup manages a service over no source.
        let app = build(Some(rule_activity::RuleActivityService::new(None)));
        assert_eq!(
            invoke(
                &app,
                "rule_activity_read",
                serde_json::json!({"readId": "read-4"})
            ),
            Ok(serde_json::json!({
                "state": "unavailable", "source": "default_local_rulebook",
                "part": "root", "reason": "not_configured", "found_version": null
            }))
        );
        // No service at all (never managed, or already gone) is closed.
        let app = build(None);
        assert_eq!(
            invoke(
                &app,
                "rule_activity_read",
                serde_json::json!({"readId": "read-5"})
            ),
            Ok(serde_json::json!({"state": "closed"}))
        );
    }
}
