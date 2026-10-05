//! A small local history of account-limit readings, kept in a JSON-lines file
//! in the app's data folder (never the database), and what the app works out
//! from it: each window's pace toward its reset and the all-models week's use
//! per local calendar day.
//!
//! Each line is one window of one successful reading: provider, window keys,
//! scope, name, percent used, reset time, window length and the reading's own
//! read time. No login, token, email or account identifier is written.
use crate::account_usage_dto::{
    AccountProviderUsage, AccountUsageDay, AccountUsagePace, AccountUsagePaceBasis,
    AccountUsagePoint, AccountUsageScope, AccountUsageState, AccountUsageWindow,
};
use jiff::{Timestamp, civil::Date, tz::TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const HISTORY_FILE: &str = "account-usage-history.jsonl";
/// Samples older than this are dropped.
const KEEP_SECS: i64 = 60 * 86_400;
/// The file is rewritten once its oldest sample is this old, so a rewrite
/// happens about once a day rather than on every reading.
const TRIM_AFTER_SECS: i64 = KEEP_SECS + 86_400;
/// A hard bound on samples kept, newest first, whatever their age.
const MAX_SAMPLES: usize = 40_000;
/// A larger file is not read; the next reading starts a new one.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// An unchanged window is recorded again after this long.
const UNCHANGED_EVERY_SECS: i64 = 60 * 60;
/// A changed window is recorded at most this often.
const CHANGED_EVERY_SECS: i64 = 15 * 60;
/// Reset times within this much of each other are the same window. Claude
/// Code's panel can show a reset rounded to the hour.
const SAME_RESET_SECS: i64 = 60 * 60;

/// Pace is hidden until this share of the window has passed.
const MIN_ELAPSED_FRACTION: f64 = 0.03;
/// The recent rate looks back this far...
const RECENT_SECS: i64 = 24 * 3600;
/// ...and needs readings spanning at least this long.
const RECENT_MIN_SPAN_SECS: i64 = 2 * 3600;
const SESSION_MINUTES: u32 = 300;
/// The week's series is thinned to at most this many points.
const MAX_SERIES_POINTS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Sample {
    pub(super) provider: Provider,
    pub(super) bucket_key: String,
    pub(super) window_key: String,
    pub(super) scope: AccountUsageScope,
    pub(super) name: String,
    pub(super) used_percent: f64,
    pub(super) resets_at: Option<i64>,
    pub(super) duration_minutes: Option<u32>,
    /// Unix seconds: the reading's own read time.
    pub(super) at: i64,
}

impl Sample {
    fn same_window(&self, provider: Provider, window: &AccountUsageWindow) -> bool {
        self.provider == provider
            && self.bucket_key == window.bucket_key
            && self.window_key == window.window_key
    }

    fn valid(&self) -> bool {
        self.used_percent.is_finite()
            && (0.0..=100.0).contains(&self.used_percent)
            && self.at > 0
            && self.bucket_key.len() <= 128
            && self.window_key.len() <= 128
            && self.name.len() <= 128
    }
}

/// The history file and the samples read from it. Loaded on first use.
pub(super) struct UsageHistory {
    path: PathBuf,
    samples: Vec<Sample>,
    loaded: bool,
    /// The file held a damaged line or ended mid-line: rewrite it whole on
    /// the next write instead of appending after the damage.
    rewrite: bool,
}

impl UsageHistory {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            samples: Vec::new(),
            loaded: false,
            rewrite: false,
        }
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let (samples, damaged) = read_file(&self.path);
        self.samples = samples;
        self.rewrite = damaged;
    }

    pub(super) fn samples(&mut self) -> &[Sample] {
        self.load();
        &self.samples
    }

    /// Records the windows of a successful reading that differ from what the
    /// history already holds. `now` decides what has aged out.
    pub(super) fn record(
        &mut self,
        provider: Provider,
        usage: &AccountProviderUsage,
        now: i64,
    ) -> std::io::Result<()> {
        self.load();
        let added = new_samples(&self.samples, provider, usage);
        if added.is_empty() && !self.rewrite {
            return Ok(());
        }
        self.samples.extend(added.iter().cloned());
        let oldest = self.samples.iter().map(|sample| sample.at).min();
        let trim =
            oldest.is_some_and(|at| now - at > TRIM_AFTER_SECS) || self.samples.len() > MAX_SAMPLES;
        if trim || self.rewrite {
            self.samples.retain(|sample| now - sample.at <= KEEP_SECS);
            self.samples.sort_by_key(|sample| sample.at);
            let excess = self.samples.len().saturating_sub(MAX_SAMPLES);
            self.samples.drain(..excess);
            let written = write_all(&self.path, &self.samples);
            // Keep the flag until a rewrite succeeds, so a damaged file is
            // never appended to.
            self.rewrite = written.is_err();
            return written;
        }
        let appended = append(&self.path, &added);
        // A failed append may leave a torn line: rewrite whole next time.
        self.rewrite = appended.is_err();
        appended
    }
}

/// Reads every valid line. Returns whether any line was damaged or the file
/// ended mid-line.
fn read_file(path: &Path) -> (Vec<Sample>, bool) {
    let Ok(file) = std::fs::File::open(path) else {
        return (Vec::new(), false);
    };
    if file
        .metadata()
        .map_or(true, |meta| meta.len() > MAX_FILE_BYTES)
    {
        return (Vec::new(), true);
    }
    let mut bytes = Vec::new();
    if file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).is_err() {
        return (Vec::new(), true);
    }
    parse_lines(&bytes)
}

pub(super) fn parse_lines(bytes: &[u8]) -> (Vec<Sample>, bool) {
    let mut damaged = !bytes.is_empty() && !bytes.ends_with(b"\n");
    let mut samples = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<Sample>(line) {
            Ok(sample) if sample.valid() => samples.push(sample),
            _ => damaged = true,
        }
    }
    samples.sort_by_key(|sample| sample.at);
    (samples, damaged)
}

fn to_lines(samples: &[Sample]) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for sample in samples {
        serde_json::to_writer(&mut bytes, sample)?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn append(path: &Path, samples: &[Sample]) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&to_lines(samples)?)
}

/// Replaces the file atomically.
fn write_all(path: &Path, samples: &[Sample]) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&to_lines(samples)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// The samples a reading adds. A window is skipped when the history already
/// has a sample of it at or after this reading's time, when it is unchanged
/// and under an hour has passed, or when it changed but under 15 minutes
/// have passed (a new reset always counts as a change worth recording).
pub(super) fn new_samples(
    existing: &[Sample],
    provider: Provider,
    usage: &AccountProviderUsage,
) -> Vec<Sample> {
    let Some(at) = usage.checked_at.filter(|at| *at > 0) else {
        return Vec::new();
    };
    if usage.state != AccountUsageState::Available {
        return Vec::new();
    }
    let mut added = Vec::new();
    for window in &usage.windows {
        let sample = Sample {
            provider,
            bucket_key: window.bucket_key.clone(),
            window_key: window.window_key.clone(),
            scope: window.scope.clone(),
            name: window.name.clone(),
            used_percent: window.used_percent,
            resets_at: window.resets_at,
            duration_minutes: window.duration_minutes,
            at,
        };
        if !sample.valid() {
            continue;
        }
        let previous = existing
            .iter()
            .filter(|old| old.same_window(provider, window))
            .max_by_key(|old| old.at);
        let keep = match previous {
            None => true,
            Some(previous) if at <= previous.at => false,
            Some(previous) => {
                let new_reset = !same_reset(previous.resets_at, window.resets_at);
                let unchanged = previous.used_percent == window.used_percent && !new_reset;
                let gap = at - previous.at;
                new_reset
                    || (unchanged && gap >= UNCHANGED_EVERY_SECS)
                    || (!unchanged && gap >= CHANGED_EVERY_SECS)
            }
        };
        if keep
            && !added
                .iter()
                .any(|other: &Sample| other.same_window(provider, window))
        {
            added.push(sample);
        }
    }
    added
}

fn same_reset(a: Option<i64>, b: Option<i64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= SAME_RESET_SECS,
        (None, None) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Pace and daily use

/// The window's length: the provider's, or five hours for a session window
/// whose length the provider did not state.
fn window_minutes(window: &AccountUsageWindow) -> Option<u32> {
    window.duration_minutes.filter(|m| *m > 0).or_else(|| {
        let session = matches!(window.window_key.as_str(), "five_hour" | "session")
            || window.window == "Session";
        session.then_some(SESSION_MINUTES)
    })
}

/// The all-models week: the window that gets a per-day breakdown.
fn is_all_models_week(window: &AccountUsageWindow) -> bool {
    window.scope == AccountUsageScope::AllModels
        && (window.duration_minutes == Some(10_080)
            || (window.duration_minutes.is_none() && window.window == "Week"))
}

/// This window's earlier samples within the current window (same reset).
fn window_points(
    history: &[Sample],
    provider: Provider,
    window: &AccountUsageWindow,
    start: i64,
    as_of: i64,
) -> Vec<(i64, f64)> {
    let mut points: Vec<(i64, f64)> = history
        .iter()
        .filter(|sample| {
            sample.same_window(provider, window)
                && sample.at >= start
                && sample.at < as_of
                && same_reset(sample.resets_at, window.resets_at)
        })
        .map(|sample| (sample.at, sample.used_percent))
        .collect();
    points.sort_by_key(|point| point.0);
    points
}

/// Where the window is heading. `as_of` is the reading's own read time; `now`
/// only decides whether the reset has passed.
pub(super) fn pace(
    window: &AccountUsageWindow,
    provider: Provider,
    history: &[Sample],
    as_of: i64,
    now: i64,
) -> Option<AccountUsagePace> {
    let used = window.used_percent;
    if !used.is_finite() || used <= 0.0 || used >= 100.0 {
        return None;
    }
    let reset = window.resets_at?;
    let length = i64::from(window_minutes(window)?) * 60;
    if reset <= now || reset <= as_of {
        return None;
    }
    let start = reset - length;
    let elapsed = as_of - start;
    if elapsed <= 0 || (elapsed as f64) < MIN_ELAPSED_FRACTION * length as f64 {
        return None;
    }
    // Per second, in percentage points.
    let recent = window_points(history, provider, window, start, as_of)
        .into_iter()
        .find(|(at, _)| as_of - at <= RECENT_SECS)
        .filter(|(at, earlier)| as_of - at >= RECENT_MIN_SPAN_SECS && used >= *earlier)
        .map(|(at, earlier)| (used - earlier) / (as_of - at) as f64);
    let (basis, rate) = match recent {
        Some(rate) => (AccountUsagePaceBasis::Recent, rate),
        None => (AccountUsagePaceBasis::WindowAverage, used / elapsed as f64),
    };
    let left = (reset - as_of) as f64;
    let projected = used + rate * left;
    let run_out_at = (projected >= 100.0 && rate > 0.0)
        .then(|| as_of + ((100.0 - used) / rate).round() as i64)
        .map(|at| at.min(reset));
    Some(AccountUsagePace {
        basis,
        projected_percent_at_reset: round1(projected.min(100.0)),
        expected_percent: round1(elapsed as f64 / length as f64 * 100.0),
        run_out_at,
    })
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn local_date(at: i64, zone: &TimeZone) -> Option<Date> {
    Some(
        Timestamp::from_second(at)
            .ok()?
            .to_zoned(zone.clone())
            .date(),
    )
}

/// Use per local calendar day from the day the window started through the
/// day of this reading. A day is the rise within its own readings plus the
/// rise since the previous day's last reading. A day without a reading is
/// unknown; a day whose previous day has no reading counts only its own
/// readings and is marked partial, and with a single reading it has nothing to
/// compare against, so it is unknown too. The window's start counts as a 0% reading
/// on the first day only when that day also has a real reading.
pub(super) fn daily(
    window: &AccountUsageWindow,
    provider: Provider,
    history: &[Sample],
    as_of: i64,
    zone: &TimeZone,
) -> Option<Vec<AccountUsageDay>> {
    if !is_all_models_week(window) {
        return None;
    }
    let reset = window.resets_at?;
    let start = reset - i64::from(window_minutes(window)?) * 60;
    if as_of < start || as_of > reset {
        return None;
    }
    let mut points = window_points(history, provider, window, start, as_of);
    points.push((as_of, window.used_percent));
    let first = local_date(start, zone)?;
    let last = local_date(as_of, zone)?;
    let mut days = Vec::new();
    let mut previous_last: Option<f64> = None;
    let mut date = first;
    loop {
        let mut values: Vec<f64> = points
            .iter()
            .filter(|(at, _)| local_date(*at, zone) == Some(date))
            .map(|(_, used)| *used)
            .collect();
        let day = if values.is_empty() {
            previous_last = None;
            AccountUsageDay {
                date: date.to_string(),
                used_points: None,
                partial: false,
            }
        } else {
            if date == first {
                values.insert(0, 0.0);
            }
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let partial = date != first && previous_last.is_none();
            let carry = previous_last.map_or(0.0, |before| (values[0] - before).max(0.0));
            previous_last = values.last().copied();
            let comparable = !partial || values.len() >= 2;
            AccountUsageDay {
                date: date.to_string(),
                used_points: comparable.then(|| round1(high - low + carry)),
                partial: partial && comparable,
            }
        };
        days.push(day);
        if date >= last || days.len() > 9 {
            break;
        }
        date = date.tomorrow().ok()?;
    }
    Some(days)
}

/// Percent used over the all-models week so far: this window's saved
/// readings (same reset, from the window's start) and then the reading
/// itself, oldest first. More than [`MAX_SERIES_POINTS`] are thinned evenly,
/// always keeping the first and the current reading.
pub(super) fn series(
    window: &AccountUsageWindow,
    provider: Provider,
    history: &[Sample],
    as_of: i64,
) -> Option<Vec<AccountUsagePoint>> {
    if !is_all_models_week(window) || !window.used_percent.is_finite() {
        return None;
    }
    let reset = window.resets_at?;
    let start = reset - i64::from(window_minutes(window)?) * 60;
    if as_of < start || as_of > reset {
        return None;
    }
    let mut points = window_points(history, provider, window, start, as_of);
    points.push((as_of, window.used_percent));
    Some(thin(&points, MAX_SERIES_POINTS))
}

fn thin(points: &[(i64, f64)], max: usize) -> Vec<AccountUsagePoint> {
    let point = |&(at, used_percent): &(i64, f64)| AccountUsagePoint { at, used_percent };
    if points.len() <= max || max < 2 {
        return points.iter().map(point).collect();
    }
    let last = points.len() - 1;
    (0..max)
        .map(|i| point(&points[(i * last + (max - 1) / 2) / (max - 1)]))
        .collect()
}

/// Adds pace and daily use to each window of a reading that has windows.
pub(super) fn annotate(
    usage: &mut AccountProviderUsage,
    provider: Provider,
    history: &[Sample],
    now: i64,
    zone: &TimeZone,
) {
    if !matches!(
        usage.state,
        AccountUsageState::Available | AccountUsageState::Stale
    ) {
        return;
    }
    let as_of = usage.checked_at.unwrap_or(now).min(now);
    for window in &mut usage.windows {
        window.pace = pace(window, provider, history, as_of, now);
        window.daily = daily(window, provider, history, as_of, zone);
        window.series = series(window, provider, history, as_of);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600;
    const DAY: i64 = 86_400;

    fn ts(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_second()
    }

    fn week(used: f64, resets_at: i64) -> AccountUsageWindow {
        AccountUsageWindow {
            bucket_key: "seven_day".into(),
            window_key: "seven_day".into(),
            scope: AccountUsageScope::AllModels,
            name: "Claude".into(),
            window: "Week".into(),
            used_percent: used,
            duration_minutes: Some(10_080),
            resets_at: Some(resets_at),
            pace: None,
            daily: None,
            series: None,
        }
    }

    fn reading(at: i64, windows: Vec<AccountUsageWindow>) -> AccountProviderUsage {
        AccountProviderUsage {
            state: AccountUsageState::Available,
            issue: None,
            checked_at: Some(at),
            windows,
        }
    }

    fn sample(at: i64, used: f64, resets_at: i64) -> Sample {
        new_samples(
            &[],
            Provider::Claude,
            &reading(at, vec![week(used, resets_at)]),
        )
        .remove(0)
    }

    #[test]
    fn history_appends_skips_repeats_and_keeps_the_reading_time() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(HISTORY_FILE);
        let reset = ts("2026-10-05T03:00:00Z");
        let t0 = ts("2026-10-01T12:00:00Z");
        let mut history = UsageHistory::new(path.clone());
        history
            .record(
                Provider::Claude,
                &reading(t0, vec![week(20.0, reset)]),
                t0 + 5,
            )
            .unwrap();
        // Same reading time again (a reused result): nothing added.
        history
            .record(
                Provider::Claude,
                &reading(t0, vec![week(20.0, reset)]),
                t0 + 600,
            )
            .unwrap();
        // Unchanged within the hour: skipped. Unchanged after an hour: kept.
        history
            .record(
                Provider::Claude,
                &reading(t0 + 1800, vec![week(20.0, reset)]),
                t0 + 1800,
            )
            .unwrap();
        history
            .record(
                Provider::Claude,
                &reading(t0 + HOUR, vec![week(20.0, reset)]),
                t0 + HOUR,
            )
            .unwrap();
        // Changed but within 15 minutes: skipped; after 15 minutes: kept.
        history
            .record(
                Provider::Claude,
                &reading(t0 + HOUR + 300, vec![week(21.0, reset)]),
                0,
            )
            .unwrap();
        history
            .record(
                Provider::Claude,
                &reading(t0 + HOUR + 900, vec![week(22.0, reset)]),
                0,
            )
            .unwrap();
        // A new window (new reset) is always kept, even right away.
        history
            .record(
                Provider::Claude,
                &reading(t0 + HOUR + 960, vec![week(0.0, reset + 7 * DAY)]),
                0,
            )
            .unwrap();
        // The same keys from Codex are a different window.
        history
            .record(
                Provider::Codex,
                &reading(t0 + HOUR + 960, vec![week(5.0, reset)]),
                0,
            )
            .unwrap();
        // A failed or stale reading adds nothing.
        let mut stale = reading(t0 + 2 * HOUR, vec![week(50.0, reset)]);
        stale.state = AccountUsageState::Stale;
        history.record(Provider::Claude, &stale, 0).unwrap();

        let (saved, damaged) = parse_lines(&std::fs::read(&path).unwrap());
        assert!(!damaged);
        let summary: Vec<(Provider, i64, f64)> = saved
            .iter()
            .map(|s| (s.provider, s.at - t0, s.used_percent))
            .collect();
        assert_eq!(
            summary,
            vec![
                (Provider::Claude, 0, 20.0),
                (Provider::Claude, HOUR, 20.0),
                (Provider::Claude, HOUR + 900, 22.0),
                (Provider::Claude, HOUR + 960, 0.0),
                (Provider::Codex, HOUR + 960, 5.0),
            ]
        );
        // Only window facts are written.
        let text = std::fs::read_to_string(&path).unwrap();
        let line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let mut keys: Vec<&str> = line
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "at",
                "bucket_key",
                "duration_minutes",
                "name",
                "provider",
                "resets_at",
                "scope",
                "used_percent",
                "window_key"
            ]
        );
        // A new process reads the same history back.
        let mut reopened = UsageHistory::new(path);
        assert_eq!(reopened.samples(), saved.as_slice());
    }

    #[test]
    fn history_drops_old_samples_and_rewrites_atomically() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(HISTORY_FILE);
        let reset = ts("2026-10-05T03:00:00Z");
        let now = ts("2026-10-01T12:00:00Z");
        let old = [
            sample(now - 70 * DAY, 10.0, reset - 70 * DAY),
            sample(now - 59 * DAY, 11.0, reset - 56 * DAY),
        ];
        std::fs::write(&path, to_lines(&old).unwrap()).unwrap();
        let mut history = UsageHistory::new(path.clone());
        history
            .record(
                Provider::Claude,
                &reading(now, vec![week(30.0, reset)]),
                now,
            )
            .unwrap();
        let (saved, damaged) = parse_lines(&std::fs::read(&path).unwrap());
        assert!(!damaged);
        assert_eq!(
            saved.iter().map(|s| s.used_percent).collect::<Vec<_>>(),
            vec![11.0, 30.0]
        );
        // No temporary file is left beside it.
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_failed_append_makes_the_next_write_a_full_rewrite() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(HISTORY_FILE);
        let reset = ts("2026-10-05T03:00:00Z");
        let t0 = ts("2026-10-01T12:00:00Z");
        let mut history = UsageHistory::new(path.clone());
        history
            .record(Provider::Claude, &reading(t0, vec![week(10.0, reset)]), t0)
            .unwrap();
        // The file cannot be appended to: a folder stands in its place.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let failed = history.record(
            Provider::Claude,
            &reading(t0 + HOUR, vec![week(12.0, reset)]),
            t0,
        );
        assert!(failed.is_err());
        std::fs::remove_dir(&path).unwrap();
        // Nothing new to add, but the whole history is written again.
        history
            .record(
                Provider::Claude,
                &reading(t0 + HOUR, vec![week(12.0, reset)]),
                t0,
            )
            .unwrap();
        let (saved, damaged) = parse_lines(&std::fs::read(&path).unwrap());
        assert!(!damaged);
        assert_eq!(
            saved.iter().map(|s| s.used_percent).collect::<Vec<_>>(),
            vec![10.0, 12.0]
        );
    }

    #[test]
    fn history_tolerates_damaged_and_partial_lines() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(HISTORY_FILE);
        let reset = ts("2026-10-05T03:00:00Z");
        let t0 = ts("2026-10-01T12:00:00Z");
        let mut bytes = to_lines(&[sample(t0, 10.0, reset)]).unwrap();
        bytes.extend_from_slice(b"not json\n{\"provider\":\"claude\",\"used_percent\":150}\n");
        bytes.extend_from_slice(&to_lines(&[sample(t0 + HOUR, 12.0, reset)]).unwrap());
        bytes.extend_from_slice(b"{\"provider\":\"cla"); // a write cut short
        std::fs::write(&path, &bytes).unwrap();
        let mut history = UsageHistory::new(path.clone());
        assert_eq!(history.samples().len(), 2);
        history
            .record(
                Provider::Claude,
                &reading(t0 + 2 * HOUR, vec![week(14.0, reset)]),
                t0,
            )
            .unwrap();
        // The damaged file was replaced whole, so the new line is intact.
        let (saved, damaged) = parse_lines(&std::fs::read(&path).unwrap());
        assert!(!damaged);
        assert_eq!(
            saved.iter().map(|s| s.used_percent).collect::<Vec<_>>(),
            vec![10.0, 12.0, 14.0]
        );
        // A missing file is an empty history.
        let mut empty = UsageHistory::new(root.path().join("missing.jsonl"));
        assert!(empty.samples().is_empty());
    }

    #[test]
    fn pace_is_hidden_early_at_zero_at_the_limit_and_after_the_reset() {
        let reset = ts("2026-10-05T03:00:00Z");
        let start = reset - 7 * DAY;
        // 3% of a week is about 5 hours.
        let early = start + 5 * HOUR;
        assert_eq!(
            pace(&week(5.0, reset), Provider::Claude, &[], early, early),
            None
        );
        let later = start + 6 * HOUR;
        assert!(pace(&week(5.0, reset), Provider::Claude, &[], later, later).is_some());
        let mid = start + 3 * DAY;
        assert_eq!(
            pace(&week(0.0, reset), Provider::Claude, &[], mid, mid),
            None
        );
        assert_eq!(
            pace(&week(100.0, reset), Provider::Claude, &[], mid, mid),
            None
        );
        // The reset passed (a stale reading looked at later).
        assert_eq!(
            pace(&week(40.0, reset), Provider::Claude, &[], mid, reset + 60),
            None
        );
        // No reset time, or no known length.
        let mut unknown = week(40.0, reset);
        unknown.resets_at = None;
        assert_eq!(pace(&unknown, Provider::Claude, &[], mid, mid), None);
        let mut other = week(40.0, reset);
        other.duration_minutes = None;
        other.window = "Other".into();
        assert_eq!(pace(&other, Provider::Claude, &[], mid, mid), None);
    }

    #[test]
    fn pace_uses_the_window_average_without_history() {
        let reset = ts("2026-10-05T03:00:00Z");
        let start = reset - 7 * DAY;
        // 30% used after 3 of 7 days: 70% by the reset.
        let as_of = start + 3 * DAY;
        let on_pace = pace(&week(30.0, reset), Provider::Claude, &[], as_of, as_of).unwrap();
        assert_eq!(on_pace.basis, AccountUsagePaceBasis::WindowAverage);
        assert_eq!(on_pace.projected_percent_at_reset, 70.0);
        assert_eq!(on_pace.run_out_at, None);
        // Three of seven days have passed: even use would be at 42.9%.
        assert_eq!(on_pace.expected_percent, 42.9);
        // 60% after 3 days: 20% a day, so the last 40% lasts two more days.
        let fast = pace(&week(60.0, reset), Provider::Claude, &[], as_of, as_of).unwrap();
        assert_eq!(fast.projected_percent_at_reset, 100.0);
        assert_eq!(fast.run_out_at, Some(as_of + 2 * DAY));
        // A session with no stated length is five hours long.
        let session = AccountUsageWindow {
            bucket_key: "limit:session:session:session".into(),
            window_key: "session".into(),
            window: "Session".into(),
            duration_minutes: None,
            ..week(50.0, as_of + 2 * HOUR)
        };
        let session_pace = pace(&session, Provider::Claude, &[], as_of, as_of).unwrap();
        // 50% in 3 hours: the rest takes 3 more hours, after the reset.
        assert_eq!(session_pace.projected_percent_at_reset, 83.3);
        assert_eq!(session_pace.run_out_at, None);
        // Three of five hours.
        assert_eq!(session_pace.expected_percent, 60.0);
    }

    #[test]
    fn pace_prefers_the_recent_rate_from_history() {
        let reset = ts("2026-10-05T03:00:00Z");
        let start = reset - 7 * DAY;
        let as_of = start + 4 * DAY;
        let history = [
            // Before the last 24 hours: ignored for the recent rate.
            sample(as_of - 30 * HOUR, 10.0, reset),
            sample(as_of - 20 * HOUR, 20.0, reset),
            sample(as_of - 10 * HOUR, 30.0, reset),
            // From another provider: ignored.
            Sample {
                provider: Provider::Codex,
                ..sample(as_of - 23 * HOUR, 0.0, reset)
            },
        ];
        // 40% now, 20% twenty hours ago: 1 point an hour, 72 hours left.
        let recent = pace(&week(40.0, reset), Provider::Claude, &history, as_of, as_of).unwrap();
        assert_eq!(recent.basis, AccountUsagePaceBasis::Recent);
        assert_eq!(recent.run_out_at, Some(as_of + 60 * HOUR));
        assert_eq!(recent.projected_percent_at_reset, 100.0);
        // The even-use point depends only on time, not on the basis.
        assert_eq!(recent.expected_percent, round1(4.0 / 7.0 * 100.0));
        // Flat over the last day: stays where it is.
        let flat_history = [sample(as_of - 20 * HOUR, 40.0, reset)];
        let flat = pace(
            &week(40.0, reset),
            Provider::Claude,
            &flat_history,
            as_of,
            as_of,
        )
        .unwrap();
        assert_eq!(flat.basis, AccountUsagePaceBasis::Recent);
        assert_eq!(flat.projected_percent_at_reset, 40.0);
        // Readings spanning under two hours, a drop, or a previous window:
        // the window average.
        for history in [
            vec![sample(as_of - HOUR, 30.0, reset)],
            vec![sample(as_of - 5 * HOUR, 45.0, reset)],
            vec![sample(as_of - 5 * HOUR, 5.0, reset - 7 * DAY)],
        ] {
            let average =
                pace(&week(40.0, reset), Provider::Claude, &history, as_of, as_of).unwrap();
            assert_eq!(average.basis, AccountUsagePaceBasis::WindowAverage);
            assert_eq!(average.projected_percent_at_reset, 70.0);
        }
    }

    fn la() -> TimeZone {
        TimeZone::get("America/Los_Angeles").unwrap()
    }

    fn days(list: &[AccountUsageDay]) -> Vec<(&str, Option<f64>, bool)> {
        list.iter()
            .map(|day| (day.date.as_str(), day.used_points, day.partial))
            .collect()
    }

    #[test]
    fn daily_use_counts_each_local_day_and_carries_across_midnight() {
        // The week started Mon Sep 28 8 PM PDT and resets Mon Oct 5 8 PM PDT.
        let reset = ts("2026-10-06T03:00:00Z");
        let at = |local: &str| {
            local
                .parse::<jiff::civil::DateTime>()
                .unwrap()
                .to_zoned(la())
                .unwrap()
                .timestamp()
                .as_second()
        };
        let history = [
            sample(at("2026-09-28T22:00"), 4.0, reset),
            sample(at("2026-09-29T09:00"), 6.0, reset),
            sample(at("2026-09-29T23:30"), 15.0, reset),
            // Wednesday has no reading.
            sample(at("2026-10-01T10:00"), 25.0, reset),
            sample(at("2026-10-01T18:00"), 30.0, reset),
            sample(at("2026-10-02T08:00"), 32.0, reset),
        ];
        let as_of = at("2026-10-02T12:00");
        let list = daily(&week(36.0, reset), Provider::Claude, &history, as_of, &la()).unwrap();
        assert_eq!(
            days(&list),
            vec![
                // From the window start (0%) to 4%.
                ("2026-09-28", Some(4.0), false),
                // 2 points overnight, then 9 during the day.
                ("2026-09-29", Some(11.0), false),
                ("2026-09-30", None, false),
                // Only its own readings: the overnight rise is unknown.
                ("2026-10-01", Some(5.0), true),
                // 2 overnight plus 4 since, through this reading.
                ("2026-10-02", Some(6.0), false),
            ]
        );
        // Without any history today has only the reading itself, with nothing
        // earlier to compare against: every day is unknown.
        let alone = daily(&week(36.0, reset), Provider::Claude, &[], as_of, &la()).unwrap();
        assert_eq!(alone.len(), 5);
        assert!(
            alone
                .iter()
                .all(|day| day.used_points.is_none() && !day.partial)
        );
        // Only the all-models week gets days.
        let mut session = week(36.0, reset);
        session.duration_minutes = Some(300);
        assert_eq!(
            daily(&session, Provider::Claude, &history, as_of, &la()),
            None
        );
        let mut model = week(36.0, reset);
        model.scope = AccountUsageScope::Model;
        assert_eq!(
            daily(&model, Provider::Claude, &history, as_of, &la()),
            None
        );
    }

    #[test]
    fn daily_use_follows_local_days_across_a_clock_change() {
        // US clocks fall back on Sun Nov 1 2026 at 2 AM: that day has 25 hours.
        let reset = ts("2026-11-05T03:00:00Z");
        let at = |utc: &str| ts(utc);
        let history = [
            sample(at("2026-10-31T23:00:00Z"), 10.0, reset), // Sat 4 PM PDT
            sample(at("2026-11-01T07:30:00Z"), 12.0, reset), // Sun 12:30 AM PDT
            sample(at("2026-11-02T07:30:00Z"), 20.0, reset), // Sun 11:30 PM PST
            sample(at("2026-11-02T08:30:00Z"), 21.0, reset), // Mon 12:30 AM PST
        ];
        let as_of = at("2026-11-02T09:00:00Z");
        let list = daily(&week(22.0, reset), Provider::Claude, &history, as_of, &la()).unwrap();
        let tail: Vec<_> = days(&list)
            .into_iter()
            .skip_while(|d| d.0 != "2026-10-31")
            .collect();
        assert_eq!(
            tail,
            vec![
                ("2026-10-31", None, false),
                ("2026-11-01", Some(10.0), false),
                ("2026-11-02", Some(2.0), false),
            ]
        );
    }

    fn summary(points: &[AccountUsagePoint]) -> Vec<(i64, f64)> {
        points.iter().map(|p| (p.at, p.used_percent)).collect()
    }

    #[test]
    fn series_keeps_this_window_and_ends_with_the_reading() {
        let reset = ts("2026-10-05T03:00:00Z");
        let start = reset - 7 * DAY;
        let as_of = start + 3 * DAY;
        let history = [
            // The previous window: excluded.
            sample(start - DAY, 80.0, reset - 7 * DAY),
            sample(start + HOUR, 1.0, reset),
            sample(start + DAY, 12.0, reset),
            // Another provider: excluded.
            Sample {
                provider: Provider::Codex,
                ..sample(start + DAY + HOUR, 50.0, reset)
            },
            sample(start + 2 * DAY, 20.0, reset),
            // After the reading (a later sample than a stale reading): excluded.
            sample(as_of + HOUR, 40.0, reset),
        ];
        let list = series(&week(30.0, reset), Provider::Claude, &history, as_of).unwrap();
        assert_eq!(
            summary(&list),
            vec![
                (start + HOUR, 1.0),
                (start + DAY, 12.0),
                (start + 2 * DAY, 20.0),
                (as_of, 30.0),
            ]
        );
        // Without history: only the reading itself.
        let alone = series(&week(30.0, reset), Provider::Claude, &[], as_of).unwrap();
        assert_eq!(summary(&alone), vec![(as_of, 30.0)]);
        // Only the all-models week, and only while the reading is inside it.
        let mut session = week(30.0, reset);
        session.duration_minutes = Some(300);
        assert_eq!(series(&session, Provider::Claude, &history, as_of), None);
        let mut model = week(30.0, reset);
        model.scope = AccountUsageScope::Model;
        assert_eq!(series(&model, Provider::Claude, &history, as_of), None);
        let mut unknown = week(30.0, reset);
        unknown.resets_at = None;
        assert_eq!(series(&unknown, Provider::Claude, &history, as_of), None);
        assert_eq!(
            series(&week(30.0, reset), Provider::Claude, &[], reset + 1),
            None
        );
    }

    #[test]
    fn series_is_thinned_evenly_and_keeps_the_ends() {
        let reset = ts("2026-10-05T03:00:00Z");
        let start = reset - 7 * DAY;
        // 999 saved readings, one every 10 minutes, plus the reading itself.
        let history: Vec<Sample> = (0..999)
            .map(|i| sample(start + 600 * (i + 1), i as f64 / 20.0, reset))
            .collect();
        let as_of = start + 600 * 1000;
        let list = series(&week(60.0, reset), Provider::Claude, &history, as_of).unwrap();
        assert_eq!(list.len(), MAX_SERIES_POINTS);
        assert_eq!(list[0].at, start + 600);
        assert_eq!(summary(&list[MAX_SERIES_POINTS - 1..]), vec![(as_of, 60.0)]);
        assert!(list.windows(2).all(|pair| pair[0].at < pair[1].at));
        // Evenly spread: no gap is more than twice the average.
        let average = (as_of - (start + 600)) / (MAX_SERIES_POINTS as i64 - 1);
        assert!(
            list.windows(2)
                .all(|pair| pair[1].at - pair[0].at <= 2 * average)
        );
        // At the limit nothing is dropped.
        let exact: Vec<Sample> = history[..MAX_SERIES_POINTS - 1].to_vec();
        let as_of = start + 600 * MAX_SERIES_POINTS as i64;
        let kept = series(&week(10.0, reset), Provider::Claude, &exact, as_of).unwrap();
        assert_eq!(kept.len(), MAX_SERIES_POINTS);
    }

    #[test]
    fn annotate_uses_the_reading_time_and_skips_failed_readings() {
        let reset = ts("2026-10-05T03:00:00Z");
        let start = reset - 7 * DAY;
        let read_at = start + 3 * DAY;
        let mut usage = reading(read_at, vec![week(30.0, reset)]);
        usage.state = AccountUsageState::Stale;
        // Looked at a day later, the stale reading keeps its own time.
        annotate(&mut usage, Provider::Claude, &[], read_at + DAY, &la());
        let window = &usage.windows[0];
        assert_eq!(
            window.pace.as_ref().unwrap().projected_percent_at_reset,
            70.0
        );
        assert!(window.daily.is_some());
        assert_eq!(
            summary(window.series.as_ref().unwrap()),
            vec![(read_at, 30.0)]
        );
        let mut failed = reading(read_at, vec![week(30.0, reset)]);
        failed.state = AccountUsageState::Failed;
        annotate(&mut failed, Provider::Claude, &[], read_at, &la());
        assert_eq!(failed.windows[0].pace, None);
        assert_eq!(failed.windows[0].series, None);
    }
}
