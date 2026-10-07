//! Bounded, read-only account limit reads. Provider data never enters the store.
//!
//! Claude is read automatically: at launch, then every ten minutes, by running
//! the user's own Claude Code `/usage` in a hidden terminal (see
//! `account_usage_claude_probe`). The last good Claude reading is kept in a
//! small file in the app's data folder (never the database) and shown, marked
//! stale with its age, until a newer read succeeds.
//!
//! Each successful reading of either provider is also added to a small
//! history file in the same folder (see `account_usage_history`), from which
//! every returned window gets its pace and the all-models week its daily use.
use crate::account_usage_claude_local::{
    CLAUDE_INTERVAL, CachedUsage, ClaudeUsagePaths, load_last_reading, read_cached_usage,
    retry_delay, save_last_reading,
};
use crate::account_usage_dto::{
    AccountProviderUsage, AccountUsage, AccountUsageIssue, AccountUsageScope, AccountUsageState,
    AccountUsageWindow,
};
use crate::account_usage_history::{Provider, UsageHistory, annotate};
use jiff::{Timestamp, tz::TimeZone};
use serde_json::Value;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime},
};

/// How often the automatic reader wakes to look for a reading Claude Code
/// saved on its own (for example after the user ran `/usage`).
const TICK: Duration = Duration::from_secs(60);
/// A reading is stale this long after its own read time: the read interval
/// plus five minutes, so a reading kept fresh by on-time reads never shows as
/// stale. Every staleness decision goes through [`stale_at`].
pub(super) const STALE_AFTER_SECS: i64 = CLAUDE_INTERVAL.as_secs() as i64 + 5 * 60;
// A saved reading used without a probe (younger than the read interval) is
// never stale on arrival.
const _: () = assert!((CLAUDE_INTERVAL.as_secs() as i64) < STALE_AFTER_SECS);

/// Unix seconds when a reading read at `checked_at` becomes stale. The one
/// staleness rule: the backend's stale state and the `stale_at` sent to the
/// screen both come from it.
pub(super) fn stale_at(checked_at: i64) -> i64 {
    checked_at.saturating_add(STALE_AFTER_SECS)
}

/// A reading read at `checked_at` is fresh at `now`: not from the future and
/// not yet stale.
pub(super) fn fresh_at(checked_at: i64, now: i64) -> bool {
    checked_at <= now && now < stale_at(checked_at)
}
/// A passive read reuses a Codex result younger than this.
const CODEX_REUSE: Duration = Duration::from_secs(60);

pub struct AccountUsageService {
    read: Mutex<ReadState>,
    finished: Condvar,
    /// Present only in the live app; tests and fixtures never start a probe.
    claude_local: Option<ClaudeUsagePaths>,
    /// The reading history; present only in the live app. Never locked while
    /// `read` is held.
    history: Mutex<Option<UsageHistory>>,
}

struct ReadState {
    codex_running: bool,
    codex_generation: u64,
    codex: AccountProviderUsage,
    /// When the last Codex read finished.
    codex_read_at: Option<Instant>,
    /// One Claude read (automatic or manual) at a time.
    claude_refreshing: bool,
    claude_generation: u64,
    claude: ClaudeState,
    /// Set after the credential helper fallback was rate limited.
    claude_retry_after: Option<Instant>,
}

#[derive(Default)]
struct ClaudeState {
    /// Automatic reads are on, so a missing reading is "reading", not
    /// "refresh to load".
    automatic: bool,
    last_good: Option<AccountProviderUsage>,
    /// When a read in this run produced `last_good`. `None` for a reading
    /// loaded from an earlier run.
    good_at: Option<Instant>,
    /// The latest read failed after `last_good` (if any) was obtained.
    last_failure: Option<AccountProviderUsage>,
    failures: u32,
    /// Wall-clock milliseconds of the next automatic read. Wall-clock time
    /// keeps counting while the Mac sleeps, so a read due during sleep runs
    /// soon after waking (a monotonic clock pauses during sleep).
    next_auto: Option<i64>,
    /// Modification time of Claude Code's config when last looked at.
    config_seen: Option<SystemTime>,
}

impl Default for AccountUsageService {
    fn default() -> Self {
        Self {
            read: Mutex::new(ReadState {
                codex_running: false,
                codex_generation: 0,
                codex: unavailable(AccountUsageIssue::SourceUnavailable),
                codex_read_at: None,
                claude_refreshing: false,
                claude_generation: 0,
                claude: ClaudeState::default(),
                claude_retry_after: None,
            }),
            finished: Condvar::new(),
            claude_local: None,
            history: Mutex::new(None),
        }
    }
}

impl AccountUsageService {
    /// The live service: loads the last saved Claude reading (shown as stale
    /// until a new read succeeds) and enables automatic Claude reads.
    pub fn with_claude_local(paths: ClaudeUsagePaths) -> Self {
        let service = Self::default();
        {
            let mut state = service.lock();
            state.claude.automatic = true;
            state.claude.last_good = load_last_reading(&paths.last_reading);
        }
        Self {
            history: Mutex::new(Some(UsageHistory::new(paths.history.clone()))),
            claude_local: Some(paths),
            ..service
        }
    }

    /// Starts the automatic Claude reader thread. It reads at once, then every
    /// ten minutes, backing off after failures. Does nothing without live paths.
    pub fn start_automatic(self: &Arc<Self>) {
        self.start_automatic_with(|work| {
            std::thread::Builder::new()
                .name("claude-usage".into())
                .spawn(work)
                .map(drop)
        });
    }

    fn start_automatic_with(
        self: &Arc<Self>,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    ) {
        if cfg!(feature = "fixtures") || self.claude_local.is_none() {
            return;
        }
        let service = Arc::clone(self);
        let started = spawn(Box::new(move || {
            loop {
                let wait = service.automatic_claude_tick();
                std::thread::sleep(wait);
            }
        }));
        if started.is_err() {
            // No automatic reads will run: say so instead of "reading"
            // forever. Refresh usage still reads.
            let mut state = self.lock();
            state.claude.automatic = false;
            state.claude.last_failure = Some(unavailable(AccountUsageIssue::SourceUnavailable));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ReadState> {
        self.read.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Adds a successful reading to the history file. A write failure only
    /// loses that sample.
    fn record_history(&self, provider: Provider, usage: &AccountProviderUsage) {
        let mut history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(history) = history.as_mut() {
            let _ = history.record(provider, usage, Timestamp::now().as_second());
        }
    }

    /// Keeps only the weekly limits, then adds each window's pace (from the
    /// reading alone) and its daily use and series (from the history), and
    /// each reading's stale time. Every returned reading passes through here.
    fn with_pace(&self, mut usage: AccountUsage) -> AccountUsage {
        weekly_only(&mut usage.claude);
        weekly_only(&mut usage.codex);
        let now = Timestamp::now().as_second();
        let zone = TimeZone::system();
        let mut history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        let samples = history
            .as_mut()
            .map_or(&[][..], |history| history.samples());
        annotate(&mut usage.claude, Provider::Claude, samples, now, &zone);
        annotate(&mut usage.codex, Provider::Codex, samples, now, &zone);
        for provider in [&mut usage.claude, &mut usage.codex] {
            provider.stale_at = provider.checked_at.map(stale_at);
        }
        usage
    }

    /// Passive reads may refresh Codex. They never start a Claude read; they
    /// return the Claude reading the automatic or manual reads hold. A Codex
    /// result less than a minute old is reused, so the window can look for a
    /// finished Claude read often without starting Codex each time.
    pub fn read(&self) -> AccountUsage {
        self.read_with_codex(|| {
            let recent = {
                let state = self.lock();
                state
                    .codex_read_at
                    .is_some_and(|at| at.elapsed() < CODEX_REUSE)
                    .then(|| state.codex.clone())
            };
            recent.unwrap_or_else(|| self.read_codex())
        })
    }

    fn read_with_codex(&self, codex_read: impl FnOnce() -> AccountProviderUsage) -> AccountUsage {
        if cfg!(feature = "fixtures") {
            return fixture_result();
        }
        let codex = codex_read();
        // Snapshot after the Codex read. A passive read that began before a
        // Claude read finished cannot return its older Claude state afterward.
        let claude = self.lock().claude_snapshot();
        self.with_pace(AccountUsage { claude, codex })
    }

    fn automatic_claude_tick(&self) -> Duration {
        let Some(paths) = self.claude_local.clone() else {
            return TICK;
        };
        self.automatic_claude_tick_with(&paths, super::account_usage_claude_probe::read_local)
    }

    /// One wakeup of the automatic reader: use a newer reading Claude Code
    /// saved on its own, else probe when a read is due. Returns the wait
    /// until the next wakeup.
    fn automatic_claude_tick_with(
        &self,
        paths: &ClaudeUsagePaths,
        probe: impl FnOnce(&ClaudeUsagePaths) -> AccountProviderUsage,
    ) -> Duration {
        let mut state = self.lock();
        if state.claude_refreshing {
            return Duration::from_secs(5);
        }
        let now_ms = Timestamp::now().as_millisecond();
        // A next read further away than the longest wait means the clock was
        // set back: read now rather than wait for it.
        let due = state.claude.next_auto.is_none_or(|at| {
            now_ms >= at || at - now_ms > retry_delay(u32::MAX).as_millis() as i64
        });
        let seen = state.claude.config_seen;
        let held = state
            .claude
            .last_good
            .as_ref()
            .and_then(|good| good.checked_at);
        state.claude_refreshing = true;
        drop(state);

        let modified = std::fs::metadata(&paths.config_file)
            .and_then(|meta| meta.modified())
            .ok();
        let mut result = None;
        if modified.is_some() && modified != seen {
            result = read_cached_usage(&paths.config_file)
                .filter(|cached| usable_without_probe(cached, held, Timestamp::now()))
                .map(CachedUsage::into_usage);
        }
        if result.is_none() && due {
            result = Some(probe(paths));
        }

        let mut state = self.lock();
        state.claude.config_seen = modified;
        let save = result.and_then(|result| state.finish_claude(result));
        let wait = state
            .claude
            .next_auto
            .map(|at| {
                Duration::from_millis(
                    u64::try_from(at - Timestamp::now().as_millisecond()).unwrap_or(0),
                )
            })
            .unwrap_or(TICK)
            .clamp(Duration::from_secs(1), TICK);
        state.claude_generation = state.claude_generation.wrapping_add(1);
        state.claude_refreshing = false;
        self.finished.notify_all();
        drop(state);
        if let Some(reading) = save {
            let _ = save_last_reading(&paths.last_reading, &reading);
            self.record_history(Provider::Claude, &reading);
        }
        wait
    }

    /// The main-window Refresh command. It bypasses the automatic backoff,
    /// waits for a Claude read already running, and falls back to the
    /// credential helper only if the local Claude Code read fails.
    pub fn refresh_claude(&self) -> AccountUsage {
        if cfg!(feature = "fixtures") {
            return fixture_result();
        }
        let clicked = Instant::now();
        let mut state = self.lock();
        while state.claude_refreshing {
            state = self
                .finished
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        if state.claude.good_at.is_some_and(|at| at >= clicked) {
            let usage = AccountUsage {
                claude: state.claude_snapshot(),
                codex: state.codex.clone(),
            };
            drop(state);
            return self.with_pace(usage);
        }
        let helper_blocked = state
            .claude_retry_after
            .is_some_and(|until| Instant::now() < until);
        state.claude_refreshing = true;
        drop(state);
        let (claude, helper_ran) = std::thread::scope(|scope| {
            let codex = scope.spawn(|| self.read_codex());
            let claude = self.manual_claude_read(helper_blocked);
            let _ = codex.join();
            claude
        });
        let mut state = self.lock();
        if helper_ran {
            state.claude_retry_after = (claude.issue == Some(AccountUsageIssue::RateLimited))
                .then(|| Instant::now() + Duration::from_secs(120));
        }
        let save = state.finish_claude(claude);
        let result = AccountUsage {
            claude: state.claude_snapshot(),
            codex: state.codex.clone(),
        };
        state.claude_generation = state.claude_generation.wrapping_add(1);
        state.claude_refreshing = false;
        self.finished.notify_all();
        drop(state);
        if let Some(reading) = save {
            if let Some(paths) = &self.claude_local {
                let _ = save_last_reading(&paths.last_reading, &reading);
            }
            self.record_history(Provider::Claude, &reading);
        }
        self.with_pace(result)
    }

    /// Returns the reading and whether the credential helper ran.
    fn manual_claude_read(&self, helper_blocked: bool) -> (AccountProviderUsage, bool) {
        let local = match &self.claude_local {
            Some(paths) => super::account_usage_claude_probe::read_local(paths),
            None => unavailable(AccountUsageIssue::SourceUnavailable),
        };
        if local.state == AccountUsageState::Available {
            return (local, false);
        }
        if helper_blocked {
            return (
                if self.claude_local.is_some() {
                    local
                } else {
                    failed(AccountUsageIssue::RateLimited)
                },
                false,
            );
        }
        let helper = super::account_usage_claude::read_via_helper();
        let keep_local = self.claude_local.is_some()
            && helper.state != AccountUsageState::Available
            && local.issue != Some(AccountUsageIssue::SourceUnavailable);
        (if keep_local { local } else { helper }, true)
    }

    fn read_codex(&self) -> AccountProviderUsage {
        let mut state = self.lock();
        if state.codex_running {
            let generation = state.codex_generation;
            while state.codex_running && state.codex_generation == generation {
                state = self
                    .finished
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            return state.codex.clone();
        }
        state.codex_running = true;
        drop(state);
        let codex = super::account_usage_codex::read();
        let mut state = self.lock();
        state.codex = codex.clone();
        state.codex_read_at = Some(Instant::now());
        state.codex_generation = state.codex_generation.wrapping_add(1);
        state.codex_running = false;
        self.finished.notify_all();
        drop(state);
        self.record_history(Provider::Codex, &codex);
        codex
    }
}

/// A reading Claude Code saved on its own is used without a probe when it is
/// newer than the one held and younger than the read interval.
fn usable_without_probe(cached: &CachedUsage, held: Option<i64>, now: Timestamp) -> bool {
    let age_ms = now.as_millisecond() - cached.fetched_at_ms;
    let newer = held.is_none_or(|held| cached.fetched_at_ms.div_euclid(1000) > held);
    newer && (0..CLAUDE_INTERVAL.as_millis() as i64).contains(&age_ms)
}

impl ReadState {
    /// Records a finished Claude read. Returns the reading to save when it
    /// succeeded with a reading at least as new as the one held. A success
    /// with an older reading (Claude Code showed an older saved copy) still
    /// counts as a read but keeps the newer reading held. A failure keeps the
    /// last good reading (shown stale) and pushes the next automatic read back.
    fn finish_claude(&mut self, result: AccountProviderUsage) -> Option<AccountProviderUsage> {
        let claude = &mut self.claude;
        let now = Instant::now();
        let now_ms = Timestamp::now().as_millisecond();
        if result.state == AccountUsageState::Available {
            claude.good_at = Some(now);
            claude.last_failure = None;
            claude.failures = 0;
            claude.next_auto = Some(now_ms + CLAUDE_INTERVAL.as_millis() as i64);
            // A held reading timed in the future (the clock was set back) is
            // never kept over a new one: it would hold until the clock passes it.
            let now_secs = now_ms.div_euclid(1000);
            let held = claude
                .last_good
                .as_ref()
                .and_then(|good| good.checked_at)
                .filter(|&held| held <= now_secs);
            if held.is_some_and(|held| result.checked_at.is_none_or(|at| at < held)) {
                return None;
            }
            claude.last_good = Some(result.clone());
            Some(result)
        } else {
            claude.last_failure = Some(result);
            claude.failures = claude.failures.saturating_add(1);
            claude.next_auto = Some(now_ms + retry_delay(claude.failures).as_millis() as i64);
            None
        }
    }

    fn claude_snapshot(&self) -> AccountProviderUsage {
        let claude = &self.claude;
        if let Some(good) = &claude.last_good {
            let mut shown = good.clone();
            let now = Timestamp::now().as_second();
            let recent = good.checked_at.is_some_and(|at| fresh_at(at, now));
            let fresh = claude.good_at.is_some() && claude.last_failure.is_none() && recent;
            if !fresh {
                shown.state = AccountUsageState::Stale;
                shown.issue = match &claude.last_failure {
                    Some(failure) => failure.issue.clone(),
                    // A saved reading from an earlier run while this run's
                    // first read is still going.
                    None if claude.automatic && claude.good_at.is_none() => {
                        Some(AccountUsageIssue::Reading)
                    }
                    None => None,
                };
            }
            return shown;
        }
        if let Some(failure) = &claude.last_failure {
            return failure.clone();
        }
        unavailable(if claude.automatic {
            AccountUsageIssue::Reading
        } else {
            AccountUsageIssue::NotFetched
        })
    }
}

/// A provider report a reader or file produced is plausible: bounded window
/// count and label sizes, and every percent in [0, 100].
pub(super) fn valid_report(report: &AccountProviderUsage) -> bool {
    report.windows.len() <= 48
        && report.windows.iter().all(|window| {
            window.used_percent.is_finite()
                && (0.0..=100.0).contains(&window.used_percent)
                && window.bucket_key.len() <= 128
                && window.window_key.len() <= 128
                && window.name.len() <= 128
                && window.window.len() <= 128
        })
}

pub(super) fn fixture_result() -> AccountUsage {
    AccountUsage {
        claude: unavailable(AccountUsageIssue::SourceUnavailable),
        codex: unavailable(AccountUsageIssue::SourceUnavailable),
    }
}

pub(super) fn unavailable(issue: AccountUsageIssue) -> AccountProviderUsage {
    AccountProviderUsage {
        state: AccountUsageState::Unavailable,
        issue: Some(issue),
        checked_at: None,
        stale_at: None,
        windows: Vec::new(),
    }
}

pub(super) fn failed(issue: AccountUsageIssue) -> AccountProviderUsage {
    AccountProviderUsage {
        state: AccountUsageState::Failed,
        issue: Some(issue),
        checked_at: None,
        stale_at: None,
        windows: Vec::new(),
    }
}

/// The app shows weekly limits only: a session (five-hour) limit, or any
/// limit of another or unknown length, is dropped before the reading is
/// shown. A reading left with no weekly limit is shown as unavailable.
fn weekly_only(usage: &mut AccountProviderUsage) {
    if usage.windows.is_empty() {
        return;
    }
    usage
        .windows
        .retain(|window| window.duration_minutes == Some(10080));
    if usage.windows.is_empty() {
        *usage = unavailable(AccountUsageIssue::SourceUnavailable);
    }
}

pub(super) fn available(windows: Vec<AccountUsageWindow>) -> AccountProviderUsage {
    if windows.is_empty() {
        return unavailable(AccountUsageIssue::SourceUnavailable);
    }
    AccountProviderUsage {
        state: AccountUsageState::Available,
        issue: None,
        checked_at: Some(Timestamp::now().as_second()),
        stale_at: None,
        windows,
    }
}

fn valid_percent(value: &Value) -> Option<f64> {
    let percent = value.as_f64()?;
    (percent.is_finite() && (0.0..=100.0).contains(&percent)).then_some(percent)
}

fn reset_seconds(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    if let Some(seconds) = value.as_i64() {
        return (seconds > 0).then_some(seconds);
    }
    value
        .as_str()
        .and_then(|text| text.parse::<Timestamp>().ok())
        .map(|instant| instant.as_second())
}

fn short_label(text: &str) -> String {
    text.chars().take(64).filter(|c| !c.is_control()).collect()
}

fn duration_label(minutes: Option<u32>, fallback: &str) -> String {
    match minutes {
        Some(10080) => "Week".into(),
        Some(1440) => "Day".into(),
        Some(n) if n % 60 == 0 && n > 0 => format!("{} hours", n / 60),
        Some(n) if n > 0 => format!("{n} minutes"),
        _ => fallback.into(),
    }
}

pub(super) fn parse_codex(value: &Value) -> Result<Vec<AccountUsageWindow>, AccountUsageIssue> {
    let payload = value.get("result").unwrap_or(value);
    let mut buckets: Vec<(String, &Value)> = Vec::new();
    if let Some(by_id) = payload
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
    {
        for (id, limit) in by_id.iter().take(24) {
            buckets.push((id.clone(), limit));
        }
    }
    if buckets.is_empty()
        && let Some(limit) = payload.get("rateLimits").filter(|v| v.is_object())
    {
        buckets.push(("codex".into(), limit));
    }
    let mut windows = Vec::new();
    for (bucket, limit) in buckets {
        let label = limit
            .get("limitName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(short_label)
            .unwrap_or_else(|| short_label(&bucket));
        for key in ["primary", "secondary"] {
            let Some(window) = limit.get(key).filter(|v| v.is_object()) else {
                continue;
            };
            let Some(used_percent) = window.get("usedPercent").and_then(valid_percent) else {
                return Err(AccountUsageIssue::InvalidResponse);
            };
            let duration_minutes = window
                .get("windowDurationMins")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok());
            windows.push(AccountUsageWindow {
                bucket_key: short_label(&bucket),
                window_key: key.into(),
                scope: if bucket == "codex" {
                    AccountUsageScope::AllModels
                } else {
                    AccountUsageScope::Unspecified
                },
                name: label.clone(),
                window: duration_label(duration_minutes, key),
                used_percent,
                duration_minutes,
                resets_at: reset_seconds(window.get("resetsAt")),
                pace: None,
                daily: None,
                series: None,
            });
        }
    }
    if windows.is_empty() {
        return Err(AccountUsageIssue::SourceUnavailable);
    }
    Ok(windows)
}

pub(super) fn parse_claude(value: &Value) -> Result<Vec<AccountUsageWindow>, AccountUsageIssue> {
    let body = value
        .as_object()
        .ok_or(AccountUsageIssue::InvalidResponse)?;
    let mut new_windows = Vec::new();
    let mut replacements = std::collections::HashSet::new();
    let mut structured_session = false;
    let mut structured_week = false;
    if let Some(limits) = body.get("limits").and_then(Value::as_array) {
        for entry in limits.iter().take(24) {
            let model = entry.get("scope").and_then(|scope| scope.get("model"));
            let model_id = model
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty());
            let model_name = model
                .and_then(|m| m.get("display_name"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty());
            let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
                continue;
            };
            let is_model = model_id.is_some() || model_name.is_some();
            let known_session = !is_model && kind == "session";
            let known_week = !is_model && kind == "weekly_all";
            let known_model_week = is_model && kind == "weekly_scoped";
            if !known_session && !known_week && !known_model_week {
                continue;
            }
            let Some(used_percent) = entry.get("percent").and_then(valid_percent) else {
                continue;
            };
            let group = entry
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or("limit");
            if (known_session && structured_session) || (known_week && structured_week) {
                continue;
            }
            structured_session |= known_session;
            structured_week |= known_week;
            let stable_id = model_id.or(model_name).unwrap_or(kind);
            let key = format!(
                "limit:{}:{}:{}",
                short_label(stable_id),
                short_label(kind),
                short_label(group)
            );
            let window = if known_session { "Session" } else { "Week" };
            if known_model_week {
                for model_label in [model_id, model_name].into_iter().flatten() {
                    replacements.insert(normalize_model(model_label));
                }
            }
            new_windows.push(AccountUsageWindow {
                bucket_key: key,
                window_key: short_label(kind),
                scope: if known_model_week {
                    AccountUsageScope::Model
                } else {
                    AccountUsageScope::AllModels
                },
                name: if known_session || known_week {
                    "Claude".into()
                } else {
                    model_name
                        .or(model_id)
                        .map(short_label)
                        .unwrap_or_else(|| short_label(kind))
                },
                window: short_label(window),
                used_percent,
                duration_minutes: (window == "Week").then_some(10080),
                resets_at: reset_seconds(entry.get("resets_at")),
                pace: None,
                daily: None,
                series: None,
            });
        }
    }
    let mut windows = Vec::new();
    for (key, entry) in body {
        let is_main = key == "five_hour" || key == "seven_day";
        let is_model_key = key.strip_prefix("seven_day_").is_some_and(|suffix| {
            !suffix.is_empty()
                && !matches!(
                    key.as_str(),
                    "seven_day_oauth_apps"
                        | "seven_day_routines"
                        | "seven_day_claude_routines"
                        | "seven_day_cowork"
                        | "seven_day_breakdown"
                )
                && !key.ends_with("_breakdown")
        });
        if (!is_main && !is_model_key) || !entry.is_object() {
            continue;
        }
        if (key == "five_hour" && structured_session) || (key == "seven_day" && structured_week) {
            continue;
        }
        let Some(percent_value) = entry.get("utilization") else {
            continue;
        };
        if percent_value.is_null() {
            continue;
        }
        let used_percent =
            valid_percent(percent_value).ok_or(AccountUsageIssue::InvalidResponse)?;
        let scope = if is_model_key {
            AccountUsageScope::Model
        } else {
            AccountUsageScope::AllModels
        };
        if is_model_key
            && key
                .strip_prefix("seven_day_")
                .is_some_and(|model| replacements.contains(&normalize_model(model)))
        {
            continue;
        }
        let name = match key.as_str() {
            "five_hour" | "seven_day" => "Claude".into(),
            other => short_label(
                &other
                    .strip_prefix("seven_day_")
                    .unwrap_or(other)
                    .replace('_', " "),
            ),
        };
        let window = if key.starts_with("seven_day") {
            "Week"
        } else if key == "five_hour" {
            "5 hours"
        } else {
            key
        };
        windows.push(AccountUsageWindow {
            bucket_key: key.clone(),
            window_key: key.clone(),
            scope,
            name,
            window: short_label(window),
            used_percent,
            duration_minutes: match key.as_str() {
                "five_hour" => Some(300),
                key if key.starts_with("seven_day") => Some(10080),
                _ => None,
            },
            resets_at: reset_seconds(entry.get("resets_at")),
            pace: None,
            daily: None,
            series: None,
        });
    }
    windows.extend(new_windows);
    if windows.is_empty() {
        return Err(AccountUsageIssue::SourceUnavailable);
    }
    Ok(windows)
}

fn normalize_model(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_keeps_buckets_and_real_durations_without_inventing_null_windows() {
        let input = json!({"result": {"rateLimitsByLimitId": {
            "codex": {"limitName": "Codex", "primary": {"usedPercent": 26, "windowDurationMins": 10080, "resetsAt": 1791050717}, "secondary": null},
            "other": {"limitName": "Other", "primary": {"usedPercent": 40, "windowDurationMins": 300, "resetsAt": null}}
        }}});
        let windows = parse_codex(&input).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].bucket_key, "codex");
        assert_eq!(windows[0].window_key, "primary");
        assert_eq!(windows[0].window, "Week");
        assert_eq!(windows[0].used_percent, 26.0);
        assert_eq!(windows[0].resets_at, Some(1791050717));
        assert_eq!(windows[1].resets_at, None);
    }

    #[test]
    fn claude_exhausted_week_is_separate_from_unused_session_and_model_limit() {
        let input = json!({
            "five_hour": {"utilization": 0, "resets_at": "2026-09-28T18:00:00Z"},
            "seven_day": {"utilization": 100, "resets_at": "2026-09-29T03:00:00Z"},
            "limits": [{"kind": "weekly_scoped", "group": "weekly", "percent": 68, "resets_at": "2026-09-29T03:00:00Z", "scope": {"model": {"id": "fable", "display_name": "Fable"}}, "is_active": true}]
        });
        let windows = parse_claude(&input).unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(
            windows
                .iter()
                .find(|w| w.window_key == "five_hour")
                .unwrap()
                .used_percent,
            0.0
        );
        assert_eq!(
            windows
                .iter()
                .find(|w| w.window_key == "seven_day")
                .unwrap()
                .used_percent,
            100.0
        );
        let model = windows.iter().find(|w| w.name == "Fable").unwrap();
        assert_eq!(model.scope, AccountUsageScope::Model);
        assert_eq!(model.used_percent, 68.0);
        assert_eq!(model.resets_at, Some(1790650800));
    }

    #[test]
    fn claude_mixed_live_shape_has_one_session_one_week_and_fable() {
        let input = json!({
            "five_hour": {"utilization": 0, "resets_at": null},
            "seven_day": {"utilization": 100, "resets_at": "2026-09-29T03:00:00.249291+00:00"},
            "seven_day_breakdown": {"utilization": null},
            "iguana_necktie": {"utilization": 0, "resets_at": "2026-11-05T08:00:00Z"},
            "nimbus_quill": {"utilization": 0, "resets_at": null},
            "limits": [
                {"group": "session", "kind": "session", "percent": 0, "resets_at": null, "is_active": false},
                {"group": "weekly", "kind": "weekly_all", "percent": 100, "resets_at": "2026-09-29T03:00:00.249291+00:00", "is_active": true},
                {"group": "weekly", "kind": "weekly_scoped", "percent": 68, "resets_at": "2026-09-29T03:00:00.249477+00:00", "scope": {"model": {"id": "fable", "display_name": "Fable"}}, "is_active": false}
            ]
        });
        let windows = parse_claude(&input).unwrap();
        assert_eq!(windows.len(), 3);
        let session = windows.iter().find(|w| w.window_key == "session").unwrap();
        assert_eq!(
            (
                session.scope.clone(),
                session.used_percent,
                session.name.as_str(),
                session.window.as_str()
            ),
            (AccountUsageScope::AllModels, 0.0, "Claude", "Session")
        );
        let week = windows
            .iter()
            .find(|w| w.window_key == "weekly_all")
            .unwrap();
        assert_eq!(
            (
                week.scope.clone(),
                week.used_percent,
                week.name.as_str(),
                week.window.as_str()
            ),
            (AccountUsageScope::AllModels, 100.0, "Claude", "Week")
        );
        assert_eq!(week.resets_at, Some(1790650800));
        let fable = windows.iter().find(|w| w.name == "Fable").unwrap();
        assert_eq!(
            (
                fable.scope.clone(),
                fable.used_percent,
                fable.window.as_str()
            ),
            (AccountUsageScope::Model, 68.0, "Week")
        );
        assert_eq!(fable.resets_at, Some(1790650800));
        assert!(
            !windows
                .iter()
                .any(|w| w.window_key == "five_hour" || w.window_key == "seven_day")
        );
        assert!(
            !windows
                .iter()
                .any(|w| w.name == "weekly_all" || w.window == "weekly_all")
        );
        assert!(
            !windows
                .iter()
                .any(|w| w.name == "iguana necktie" || w.name == "nimbus quill")
        );
    }

    #[test]
    fn unknown_structured_kinds_and_known_legacy_nonmodels_are_not_windows() {
        let input = json!({
            "seven_day": {"utilization": 100},
            "seven_day_oauth_apps": {"utilization": 12},
            "seven_day_routines": {"utilization": 13},
            "seven_day_claude_routines": {"utilization": 14},
            "seven_day_cowork": {"utilization": 15},
            "limits": [
                {"kind": "mystery_quota", "group": "weekly", "percent": 44, "scope": {"model": {"display_name": "Mystery"}}},
                {"kind": "weekly_scoped", "group": "weekly", "percent": 68, "scope": {"model": {"display_name": "Fable"}}}
            ]
        });
        let windows = parse_claude(&input).unwrap();
        assert_eq!(windows.len(), 2);
        assert!(windows.iter().any(|w| w.window_key == "seven_day"));
        assert!(windows.iter().any(|w| w.name == "Fable"));
    }

    #[test]
    fn structured_only_and_legacy_only_claude_windows_stay_available() {
        let structured = json!({"limits": [
            {"group": "session", "kind": "session", "percent": 0, "is_active": false},
            {"group": "weekly", "kind": "weekly_all", "percent": 100},
            {"group": "weekly", "kind": "weekly_scoped", "percent": 68, "scope": {"model": {"display_name": "Fable"}}, "is_active": false}
        ]});
        let structured_windows = parse_claude(&structured).unwrap();
        assert_eq!(structured_windows.len(), 3);
        assert!(
            structured_windows
                .iter()
                .any(|w| w.window == "Session" && w.used_percent == 0.0)
        );
        assert!(structured_windows.iter().any(|w| w.window == "Week"
            && w.scope == AccountUsageScope::AllModels
            && w.used_percent == 100.0));

        let legacy = json!({
            "five_hour": {"utilization": 0},
            "seven_day": {"utilization": 100},
            "seven_day_fable": {"utilization": 68}
        });
        let legacy_windows = parse_claude(&legacy).unwrap();
        assert_eq!(legacy_windows.len(), 3);
        assert!(legacy_windows.iter().any(|w| w.window_key == "five_hour"));
        assert!(legacy_windows.iter().any(|w| w.window_key == "seven_day"));
        assert!(
            legacy_windows
                .iter()
                .any(|w| w.window_key == "seven_day_fable")
        );
    }

    #[test]
    fn invalid_structured_main_rows_leave_valid_legacy_fallbacks() {
        let input = json!({
            "five_hour": {"utilization": 0},
            "seven_day": {"utilization": 100},
            "limits": [
                {"group": "session", "kind": "session", "percent": null},
                {"group": "weekly", "kind": "weekly_all", "percent": 105}
            ]
        });
        let windows = parse_claude(&input).unwrap();
        assert_eq!(windows.len(), 2);
        assert!(
            windows
                .iter()
                .any(|w| w.window_key == "five_hour" && w.used_percent == 0.0)
        );
        assert!(
            windows
                .iter()
                .any(|w| w.window_key == "seven_day" && w.used_percent == 100.0)
        );
    }

    #[test]
    fn unrelated_or_invalid_new_limit_keeps_legacy_fable_window() {
        let legacy =
            json!({"seven_day_fable": {"utilization": 68, "resets_at": "2026-09-29T03:00:00Z"}});
        for limit in [
            json!({"kind": "weekly_scoped", "group": "weekly", "percent": 22, "scope": {"model": {"id": "sonnet", "display_name": "Sonnet"}}}),
            json!({"kind": "weekly_scoped", "group": "weekly", "percent": null, "scope": {"model": {"id": "fable", "display_name": "Fable"}}}),
            json!({"kind": "other"}),
        ] {
            let mut input = legacy.clone();
            input["limits"] = json!([limit]);
            let windows = parse_claude(&input).unwrap();
            let fable = windows
                .iter()
                .find(|window| window.window_key == "seven_day_fable")
                .unwrap();
            assert_eq!(fable.used_percent, 68.0);
        }
    }

    #[test]
    fn valid_matching_new_model_window_replaces_only_that_legacy_window() {
        let input = json!({
            "seven_day_fable": {"utilization": 68},
            "seven_day_sonnet": {"utilization": 30},
            "limits": [{"kind": "weekly_scoped", "group": "weekly", "percent": 69, "scope": {"model": {"id": "fable-v2", "display_name": "Fable"}}}]
        });
        let windows = parse_claude(&input).unwrap();
        assert_eq!(
            windows
                .iter()
                .filter(|window| window.name == "Fable")
                .count(),
            1
        );
        assert_eq!(
            windows
                .iter()
                .find(|window| window.name == "Fable")
                .unwrap()
                .used_percent,
            69.0
        );
        assert!(
            !windows
                .iter()
                .any(|window| window.window_key == "seven_day_fable")
        );
        assert!(
            windows
                .iter()
                .any(|window| window.window_key == "seven_day_sonnet")
        );
    }

    #[test]
    fn invalid_percent_never_becomes_available_and_failures_have_no_old_windows() {
        let malformed = json!({"seven_day": {"utilization": 105}});
        assert!(matches!(
            parse_claude(&malformed),
            Err(AccountUsageIssue::InvalidResponse)
        ));
        let failure = failed(AccountUsageIssue::RateLimited);
        assert_eq!(failure.state, AccountUsageState::Failed);
        assert!(failure.windows.is_empty());
        assert_eq!(failure.checked_at, None);
    }

    #[test]
    fn serialized_provider_result_contains_no_credential_or_account_identifier() {
        let result = AccountUsage {
            claude: failed(AccountUsageIssue::Unauthorized),
            codex: unavailable(AccountUsageIssue::SourceUnavailable),
        };
        let wire = serde_json::to_string(&result).unwrap();
        assert!(!wire.contains("token"));
        assert!(!wire.contains("account_id"));
    }

    fn week(percent: f64) -> AccountUsageWindow {
        AccountUsageWindow {
            bucket_key: "seven_day".into(),
            window_key: "seven_day".into(),
            scope: AccountUsageScope::AllModels,
            name: "Claude".into(),
            window: "Week".into(),
            used_percent: percent,
            duration_minutes: Some(10080),
            resets_at: None,
            pace: None,
            daily: None,
            series: None,
        }
    }

    #[test]
    fn only_weekly_limits_are_shown() {
        let session = AccountUsageWindow {
            bucket_key: "five_hour".into(),
            window_key: "five_hour".into(),
            window: "Session".into(),
            used_percent: 48.0,
            duration_minutes: Some(300),
            ..week(0.0)
        };
        let unknown = AccountUsageWindow {
            window_key: "secondary".into(),
            window: "secondary".into(),
            duration_minutes: None,
            ..week(0.0)
        };
        let shown = AccountUsageService::default().with_pace(AccountUsage {
            claude: available(vec![session.clone(), week(3.0)]),
            codex: available(vec![session, unknown]),
        });
        assert_eq!(shown.claude.state, AccountUsageState::Available);
        assert_eq!(shown.claude.windows.len(), 1);
        assert_eq!(shown.claude.windows[0].used_percent, 3.0);
        assert_eq!(shown.codex.state, AccountUsageState::Unavailable);
        assert!(shown.codex.windows.is_empty());
    }

    #[cfg(not(feature = "fixtures"))]
    #[test]
    fn passive_read_before_any_claude_read_is_not_fetched_or_reading() {
        let service = AccountUsageService::default();
        let result = service.read_with_codex(|| unavailable(AccountUsageIssue::SourceUnavailable));
        assert_eq!(result.claude.state, AccountUsageState::Unavailable);
        assert_eq!(result.claude.issue, Some(AccountUsageIssue::NotFetched));
        assert_eq!(result.claude.checked_at, None);
        assert!(result.claude.windows.is_empty());
        assert_eq!(
            result.codex.issue,
            Some(AccountUsageIssue::SourceUnavailable)
        );
        // With automatic reads on, the first read is pending: "reading".
        service.lock().claude.automatic = true;
        let pending = service.read_with_codex(|| unavailable(AccountUsageIssue::SourceUnavailable));
        assert_eq!(pending.claude.issue, Some(AccountUsageIssue::Reading));
        // A real read that finds no source stays an error.
        service
            .lock()
            .finish_claude(unavailable(AccountUsageIssue::SourceUnavailable));
        let after = service.read_with_codex(|| unavailable(AccountUsageIssue::SourceUnavailable));
        assert_eq!(
            after.claude.issue,
            Some(AccountUsageIssue::SourceUnavailable)
        );
    }

    #[cfg(not(feature = "fixtures"))]
    #[test]
    fn passive_read_uses_new_claude_result_after_codex_finishes() {
        let service = AccountUsageService::default();
        let result = service.read_with_codex(|| {
            service.lock().finish_claude(available(vec![week(100.0)]));
            unavailable(AccountUsageIssue::SourceUnavailable)
        });
        assert_eq!(result.claude.windows[0].used_percent, 100.0);
        assert_eq!(result.claude.state, AccountUsageState::Available);
    }

    #[test]
    fn old_claude_result_is_stale_with_original_checked_time() {
        let service = AccountUsageService::default();
        let mut state = service.lock();
        let mut old = available(vec![week(80.0)]);
        old.checked_at = Some(Timestamp::now().as_second() - STALE_AFTER_SECS - 1);
        state.finish_claude(old.clone());
        let cached = state.claude_snapshot();
        assert_eq!(cached.state, AccountUsageState::Stale);
        assert_eq!(cached.issue, None);
        assert_eq!(cached.checked_at, old.checked_at);
        assert_eq!(cached.windows[0].used_percent, 80.0);
    }

    /// The state the backend gives and the stale time it sends come from the
    /// same rule, so the screen turns a reading stale exactly when a new
    /// read of the same reading would.
    #[cfg(not(feature = "fixtures"))]
    #[test]
    fn stale_state_and_stale_time_follow_one_rule() {
        let service = AccountUsageService::default();
        let now = Timestamp::now().as_second();
        let mut reading = available(vec![week(40.0)]);
        // A minute before its stale time: fresh, and says when it turns stale.
        reading.checked_at = Some(now - STALE_AFTER_SECS + 60);
        service.lock().finish_claude(reading.clone());
        let shown = service.read_with_codex(|| unavailable(AccountUsageIssue::SourceUnavailable));
        assert_eq!(shown.claude.state, AccountUsageState::Available);
        assert_eq!(shown.claude.stale_at, Some(now + 60));
        // A reading at its stale time (with nothing newer held): stale, with
        // the same stale time.
        reading.checked_at = Some(now - STALE_AFTER_SECS);
        service.lock().claude.last_good = None;
        service.lock().finish_claude(reading);
        let shown = service.read_with_codex(|| {
            let mut codex = available(vec![week(10.0)]);
            codex.checked_at = Some(now - 30);
            codex
        });
        assert_eq!(shown.claude.state, AccountUsageState::Stale);
        assert_eq!(shown.claude.stale_at, Some(now));
        // Codex readings carry their stale time by the same rule.
        assert_eq!(shown.codex.stale_at, Some(now - 30 + STALE_AFTER_SECS));
        // No read time, no stale time.
        let none = service.read_with_codex(|| unavailable(AccountUsageIssue::SourceUnavailable));
        assert_eq!(none.codex.stale_at, None);
        // A successful read of an older reading (Claude Code showed an older
        // saved copy) keeps the newer reading held, saves nothing and does
        // not back off.
        let mut held = available(vec![week(55.0)]);
        held.checked_at = Some(now - 120);
        let mut state = service.lock();
        state.claude.failures = 3;
        assert!(state.finish_claude(held.clone()).is_some());
        let mut older = available(vec![week(50.0)]);
        older.checked_at = Some(now - 50 * 60);
        assert_eq!(state.finish_claude(older), None);
        assert_eq!(state.claude.failures, 0);
        assert!(state.claude.last_failure.is_none());
        let shown = state.claude_snapshot();
        assert_eq!(shown.windows[0].used_percent, 55.0);
        assert_eq!(shown.checked_at, held.checked_at);
        // A held reading timed in the future (the clock was set back) does
        // not block a new reading.
        let mut future = available(vec![week(70.0)]);
        future.checked_at = Some(now + 3600);
        state.claude.last_good = Some(future);
        let mut current = available(vec![week(45.0)]);
        current.checked_at = Some(now - 60);
        assert!(state.finish_claude(current.clone()).is_some());
        assert_eq!(state.claude_snapshot().checked_at, current.checked_at);
        assert_eq!(state.claude_snapshot().windows[0].used_percent, 45.0);
        // With no reading held, an old copy is adopted with its own time and
        // shown stale by the one rule, not as a failure.
        let mut newer_old = available(vec![week(60.0)]);
        newer_old.checked_at = Some(now - 40 * 60);
        state.claude.last_good = None;
        assert!(state.finish_claude(newer_old.clone()).is_some());
        let shown = state.claude_snapshot();
        assert_eq!(shown.state, AccountUsageState::Stale);
        assert_eq!(shown.checked_at, newer_old.checked_at);
        drop(state);
        // The stale time is never saved with the reading.
        assert!(
            !serde_json::to_string(&service.lock().claude_snapshot())
                .unwrap()
                .contains("stale_at")
        );
    }

    fn live_paths(root: &std::path::Path) -> ClaudeUsagePaths {
        ClaudeUsagePaths::new(&root.join("data"), &root.join("home"), None)
    }

    fn write_config(paths: &ClaudeUsagePaths, fetched_at_ms: i64, percent: u32) {
        std::fs::create_dir_all(paths.config_file.parent().unwrap()).unwrap();
        let config = serde_json::json!({"cachedUsageUtilization": {
            "fetchedAtMs": fetched_at_ms,
            "utilization": {"seven_day": {"utilization": percent, "resets_at": null}}
        }});
        std::fs::write(&paths.config_file, config.to_string()).unwrap();
    }

    #[test]
    fn automatic_reads_probe_at_launch_then_wait_for_the_interval() {
        let root = tempfile::tempdir().unwrap();
        let paths = live_paths(root.path());
        let service = AccountUsageService::with_claude_local(paths.clone());
        assert_eq!(
            service.lock().claude_snapshot().issue,
            Some(AccountUsageIssue::Reading)
        );
        let probes = std::cell::Cell::new(0);
        let wait = service.automatic_claude_tick_with(&paths, |_| {
            probes.set(probes.get() + 1);
            available(vec![week(37.0)])
        });
        assert_eq!(probes.get(), 1);
        assert!(wait <= TICK);
        let shown = service.lock().claude_snapshot();
        assert_eq!(shown.state, AccountUsageState::Available);
        assert_eq!(shown.windows[0].used_percent, 37.0);
        // The good reading was saved for the next launch, which shows it stale.
        assert_eq!(load_last_reading(&paths.last_reading), Some(shown.clone()));
        let relaunched = AccountUsageService::with_claude_local(paths.clone());
        let restored = relaunched.lock().claude_snapshot();
        assert_eq!(restored.state, AccountUsageState::Stale);
        assert_eq!(restored.issue, Some(AccountUsageIssue::Reading));
        assert_eq!(restored.checked_at, shown.checked_at);
        // Not due again yet: no second probe.
        service.automatic_claude_tick_with(&paths, |_| panic!("probe ran before the interval"));
        let left = until_next(&service);
        assert!(left > CLAUDE_INTERVAL - Duration::from_secs(5) && left <= CLAUDE_INTERVAL);
    }

    #[cfg(not(feature = "fixtures"))]
    #[test]
    fn readings_are_added_to_the_history_and_returned_with_pace() {
        let root = tempfile::tempdir().unwrap();
        let paths = live_paths(root.path());
        let service = AccountUsageService::with_claude_local(paths.clone());
        let now = Timestamp::now().as_second();
        // Three of seven days passed with 30% used: 70% by the reset.
        let mut current = week(30.0);
        current.resets_at = Some(now + 4 * 86_400);
        service.automatic_claude_tick_with(&paths, |_| available(vec![current.clone()]));
        let lines = std::fs::read_to_string(&paths.history).unwrap();
        assert_eq!(lines.lines().count(), 1);
        assert!(lines.contains("\"provider\":\"claude\""));
        let shown = service.read_with_codex(|| unavailable(AccountUsageIssue::SourceUnavailable));
        let pace = shown.claude.windows[0].pace.as_ref().unwrap();
        assert_eq!(pace.projected_percent_at_reset, 70.0);
        assert!(shown.claude.windows[0].daily.is_some());
        // The saved last reading holds only what the provider reported.
        let saved = load_last_reading(&paths.last_reading).unwrap();
        assert_eq!(saved.windows[0].pace, None);
        assert_eq!(saved.windows[0].daily, None);
        assert!(
            !std::fs::read_to_string(&paths.last_reading)
                .unwrap()
                .contains("pace")
        );
    }

    fn until_next(service: &AccountUsageService) -> Duration {
        let next = service.lock().claude.next_auto.unwrap();
        Duration::from_millis(u64::try_from(next - Timestamp::now().as_millisecond()).unwrap_or(0))
    }

    #[test]
    fn a_read_due_while_the_mac_slept_runs_at_the_next_wakeup() {
        let root = tempfile::tempdir().unwrap();
        let paths = live_paths(root.path());
        let service = AccountUsageService::with_claude_local(paths.clone());
        service.automatic_claude_tick_with(&paths, |_| available(vec![week(37.0)]));
        // Wall-clock time passed (as during sleep) although little monotonic
        // time did: the read is due.
        service.lock().claude.next_auto = Some(Timestamp::now().as_millisecond() - 1);
        let probes = std::cell::Cell::new(0);
        service.automatic_claude_tick_with(&paths, |_| {
            probes.set(probes.get() + 1);
            available(vec![week(38.0)])
        });
        assert_eq!(probes.get(), 1);
        // A clock set back far beyond the longest wait reads now too.
        service.lock().claude.next_auto =
            Some(Timestamp::now().as_millisecond() + 5 * 60 * 60 * 1000);
        service.automatic_claude_tick_with(&paths, |_| {
            probes.set(probes.get() + 1);
            available(vec![week(39.0)])
        });
        assert_eq!(probes.get(), 2);
    }

    #[cfg(not(feature = "fixtures"))]
    #[test]
    fn a_reader_thread_that_cannot_start_is_reported_not_left_reading() {
        let root = tempfile::tempdir().unwrap();
        let service = Arc::new(AccountUsageService::with_claude_local(live_paths(
            root.path(),
        )));
        service.start_automatic_with(|_| Err(std::io::ErrorKind::OutOfMemory.into()));
        let shown = service.lock().claude_snapshot();
        assert_eq!(shown.state, AccountUsageState::Unavailable);
        assert_eq!(shown.issue, Some(AccountUsageIssue::SourceUnavailable));
        // A started thread leaves the first read pending as "reading".
        let other = Arc::new(AccountUsageService::with_claude_local(live_paths(
            root.path(),
        )));
        let ran = std::cell::Cell::new(false);
        other.start_automatic_with(|_| {
            ran.set(true);
            Ok(())
        });
        assert!(ran.get());
        assert_eq!(
            other.lock().claude_snapshot().issue,
            Some(AccountUsageIssue::Reading)
        );
    }

    #[test]
    fn failures_keep_the_last_reading_stale_and_back_off() {
        let root = tempfile::tempdir().unwrap();
        let paths = live_paths(root.path());
        let service = AccountUsageService::with_claude_local(paths.clone());
        service.automatic_claude_tick_with(&paths, |_| available(vec![week(37.0)]));
        let mut expected = Vec::new();
        for failures in 1..=5u32 {
            service.lock().claude.next_auto = Some(Timestamp::now().as_millisecond());
            service.automatic_claude_tick_with(&paths, |_| failed(AccountUsageIssue::Timeout));
            let left = until_next(&service);
            let state = service.lock();
            expected.push((left + Duration::from_secs(5)).as_secs() / 60);
            assert_eq!(state.claude.failures, failures);
            let shown = state.claude_snapshot();
            assert_eq!(shown.state, AccountUsageState::Stale);
            assert_eq!(shown.issue, Some(AccountUsageIssue::Timeout));
            assert_eq!(shown.windows[0].used_percent, 37.0);
        }
        assert_eq!(expected, vec![10, 20, 40, 60, 60]);
        // A success resets the schedule and the stale mark.
        service.lock().claude.next_auto = Some(Timestamp::now().as_millisecond());
        service.automatic_claude_tick_with(&paths, |_| available(vec![week(40.0)]));
        let state = service.lock();
        assert_eq!(state.claude.failures, 0);
        assert_eq!(state.claude_snapshot().state, AccountUsageState::Available);
    }

    #[test]
    fn a_reading_claude_code_saved_on_its_own_is_used_without_a_probe() {
        let root = tempfile::tempdir().unwrap();
        let paths = live_paths(root.path());
        let service = AccountUsageService::with_claude_local(paths.clone());
        let now_ms = Timestamp::now().as_millisecond();
        write_config(&paths, now_ms - 60_000, 55);
        service.automatic_claude_tick_with(&paths, |_| {
            panic!("probe ran despite a fresh saved reading")
        });
        let shown = service.lock().claude_snapshot();
        assert_eq!(shown.windows[0].used_percent, 55.0);
        assert_eq!(shown.checked_at, Some((now_ms - 60_000).div_euclid(1000)));
        // An old saved reading is not enough: the due read probes.
        let other =
            AccountUsageService::with_claude_local(live_paths(tempfile::tempdir().unwrap().path()));
        write_config(&paths, now_ms - CLAUDE_INTERVAL.as_millis() as i64 - 1, 20);
        let probes = std::cell::Cell::new(0);
        other.automatic_claude_tick_with(&paths, |_| {
            probes.set(probes.get() + 1);
            available(vec![week(21.0)])
        });
        assert_eq!(probes.get(), 1);
    }

    #[test]
    fn shortcut_needs_a_newer_and_recent_reading() {
        let now: Timestamp = "2026-10-01T19:00:00Z".parse().unwrap();
        let cached = |ms| CachedUsage {
            fetched_at_ms: ms,
            windows: vec![week(1.0)],
        };
        let now_ms = now.as_millisecond();
        assert!(usable_without_probe(&cached(now_ms - 1_000), None, now));
        assert!(usable_without_probe(
            &cached(now_ms - 1_000),
            Some(now.as_second() - 120),
            now
        ));
        assert!(!usable_without_probe(
            &cached(now_ms - 1_000),
            Some(now.as_second()),
            now
        ));
        assert!(!usable_without_probe(
            &cached(now_ms - CLAUDE_INTERVAL.as_millis() as i64),
            None,
            now
        ));
        assert!(!usable_without_probe(&cached(now_ms + 5_000), None, now));
    }

    #[cfg(not(feature = "fixtures"))]
    #[test]
    fn passive_reads_reuse_a_recent_codex_result() {
        let service = AccountUsageService::default();
        {
            let mut state = service.lock();
            state.codex = available(vec![week(26.0)]);
            state.codex_read_at = Some(Instant::now());
        }
        // A fresh Codex result is reused: `read` would otherwise start Codex.
        let result = service.read();
        assert_eq!(result.codex.windows[0].used_percent, 26.0);
    }

    #[test]
    fn one_claude_read_at_a_time() {
        let root = tempfile::tempdir().unwrap();
        let paths = live_paths(root.path());
        let service = AccountUsageService::with_claude_local(paths.clone());
        service.lock().claude_refreshing = true;
        let wait = service.automatic_claude_tick_with(&paths, |_| panic!("second concurrent read"));
        assert_eq!(wait, Duration::from_secs(5));
    }

    #[cfg(feature = "fixtures")]
    #[test]
    fn fixture_mode_never_starts_a_provider_read() {
        let result = AccountUsageService::default().read();
        assert_eq!(result.claude.state, AccountUsageState::Unavailable);
        assert_eq!(result.codex.state, AccountUsageState::Unavailable);
        assert!(result.claude.windows.is_empty());
        assert!(result.codex.windows.is_empty());
        let guarded = AccountUsageService::default()
            .read_with_codex(|| panic!("fixture must not launch Codex"));
        assert_eq!(guarded.codex.state, AccountUsageState::Unavailable);
    }
}
