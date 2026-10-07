//! Claude usage from the user's own Claude Code installation, with no
//! credential access: the `cachedUsageUtilization` entry Claude Code writes to
//! its config file after `/usage`, the rendered `/usage` panel as a fallback,
//! the trust-dialog answer for the dedicated probe folder, the automatic-read
//! schedule, and the small last-reading file in the app's data folder.
use crate::account_usage::{parse_claude, valid_report};
use crate::account_usage_dto::{
    AccountProviderUsage, AccountUsageIssue, AccountUsageScope, AccountUsageState,
    AccountUsageWindow,
};
use jiff::{Timestamp, civil, tz::TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

/// The probe folder's own name. The session index skips Claude projects whose
/// encoded folder name ends with `-` followed by this name.
pub(super) const PROBE_DIR_NAME: &str = "xtrace-claude-usage-probe";
const LAST_READING_FILE: &str = "claude-usage-last.json";
/// Claude Code's config can hold many projects; anything larger is not read.
const MAX_CONFIG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LAST_READING_BYTES: u64 = 64 * 1024;

/// How often Claude is read while the app runs, and how old a reading Claude
/// Code cached on its own may be to be used without starting a probe.
pub(super) const CLAUDE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Where the probe runs and what it reads. Built only for the live app.
#[derive(Clone, Debug)]
pub struct ClaudeUsagePaths {
    pub(super) probe_dir: PathBuf,
    pub(super) last_reading: PathBuf,
    /// The reading history of both providers (`account-usage-history.jsonl`).
    pub(super) history: PathBuf,
    /// Claude Code's global config (`~/.claude.json`), read only.
    pub(super) config_file: PathBuf,
    /// Claude Code's transcript folders (`~/.claude/projects`).
    pub(super) projects_dir: PathBuf,
}

impl ClaudeUsagePaths {
    pub fn new(app_data_dir: &Path, home: &Path, claude_config_dir: Option<PathBuf>) -> Self {
        let claude_config_dir = claude_config_dir.filter(|dir| dir.is_absolute());
        let (config_file, projects_dir) = match claude_config_dir {
            Some(dir) => (dir.join(".claude.json"), dir.join("projects")),
            None => (
                home.join(".claude.json"),
                home.join(".claude").join("projects"),
            ),
        };
        Self {
            probe_dir: app_data_dir.join(PROBE_DIR_NAME),
            last_reading: app_data_dir.join(LAST_READING_FILE),
            history: app_data_dir.join(crate::account_usage_history::HISTORY_FILE),
            config_file,
            projects_dir,
        }
    }
}

/// Delay before the next automatic read after `failures` consecutive failed
/// reads: 10, 20, 40, then at most 60 minutes. Zero failures is the interval.
pub(super) fn retry_delay(failures: u32) -> Duration {
    match failures {
        0 | 1 => CLAUDE_INTERVAL,
        2 => Duration::from_secs(20 * 60),
        3 => Duration::from_secs(40 * 60),
        _ => Duration::from_secs(60 * 60),
    }
}

// ---------------------------------------------------------------------------
// cachedUsageUtilization in Claude Code's config

#[derive(Debug, PartialEq)]
pub(super) struct CachedUsage {
    pub(super) fetched_at_ms: i64,
    pub(super) windows: Vec<AccountUsageWindow>,
}

impl CachedUsage {
    pub(super) fn into_usage(self) -> AccountProviderUsage {
        AccountProviderUsage {
            state: AccountUsageState::Available,
            issue: None,
            checked_at: Some(self.fetched_at_ms.div_euclid(1000)),
            stale_at: None,
            windows: self.windows,
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    #[serde(rename = "cachedUsageUtilization")]
    cached: Option<Value>,
}

/// Reads Claude Code's config file without changing it. `None` when the file,
/// the entry, or a valid reading is absent (a file mid-write reads as absent).
pub(super) fn read_cached_usage(config_file: &Path) -> Option<CachedUsage> {
    let file = std::fs::File::open(config_file).ok()?;
    if file.metadata().ok()?.len() > MAX_CONFIG_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES).read_to_end(&mut bytes).ok()?;
    let config: ConfigFile = serde_json::from_slice(&bytes).ok()?;
    parse_cached_usage(&config.cached?).ok()
}

/// Only the session window, the all-models week and model weeks whose name is
/// clear are kept; every other key is dropped rather than given a guessed name.
pub(super) fn parse_cached_usage(cached: &Value) -> Result<CachedUsage, AccountUsageIssue> {
    let fetched_at_ms = cached
        .get("fetchedAtMs")
        .and_then(Value::as_i64)
        .filter(|ms| *ms > 0)
        .ok_or(AccountUsageIssue::InvalidResponse)?;
    let utilization = cached
        .get("utilization")
        .and_then(Value::as_object)
        .ok_or(AccountUsageIssue::InvalidResponse)?;
    let mut kept = Map::new();
    for key in [
        "five_hour",
        "seven_day",
        "seven_day_opus",
        "seven_day_sonnet",
        "limits",
    ] {
        if let Some(value) = utilization.get(key).filter(|value| !value.is_null()) {
            kept.insert(key.to_owned(), value.clone());
        }
    }
    let mut windows = parse_claude(&Value::Object(kept))?;
    for window in &mut windows {
        match window.window_key.as_str() {
            "seven_day_opus" => window.name = "Opus".into(),
            "seven_day_sonnet" => window.name = "Sonnet".into(),
            _ => {}
        }
    }
    Ok(CachedUsage {
        fetched_at_ms,
        windows,
    })
}

// ---------------------------------------------------------------------------
// The rendered /usage panel (fallback)

enum Header {
    Session,
    Week,
    ModelWeek(String),
}

fn header(line: &str) -> Option<(Header, usize)> {
    if let Some(at) = line.find("Current session") {
        return Some((Header::Session, at + "Current session".len()));
    }
    let after = line.find("Current week")? + "Current week".len();
    let rest = &line[after..];
    let open = after + (rest.len() - rest.trim_start().len());
    if line[open..].starts_with('(') {
        let close = open + line[open..].find(')')?;
        let name: String = line[open + 1..close]
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        let end = close + 1;
        let name = name.trim();
        if name.eq_ignore_ascii_case("all models") {
            return Some((Header::Week, end));
        }
        if name.is_empty() {
            return None;
        }
        return Some((Header::ModelWeek(name.to_owned()), end));
    }
    Some((Header::Week, after))
}

fn percent_used(text: &str) -> Option<f64> {
    for (marker, used) in [("% used", true), ("% left", false), ("% remaining", false)] {
        let Some(at) = text.find(marker) else {
            continue;
        };
        let digits: String = text[..at]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let value: f64 = digits.parse().ok()?;
        if !(0.0..=100.0).contains(&value) {
            return None;
        }
        return Some(if used { value } else { 100.0 - value });
    }
    None
}

/// Windows shown in a captured `/usage` panel. The "What's contributing to
/// your limits usage?" section and everything below it is never read.
pub(super) fn parse_usage_screen(text: &str, now: Timestamp) -> Vec<AccountUsageWindow> {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line
            .to_ascii_lowercase()
            .contains("contributing to your limits")
        {
            break;
        }
        lines.push(line);
    }
    let mut windows: Vec<AccountUsageWindow> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some((kind, end)) = header(line) else {
            continue;
        };
        // The header's own line after the name, then following lines until
        // the next header: "██ 19% used" and "Resets 1:10pm (Zone)".
        let mut body = vec![&line[end..]];
        for next in lines.iter().skip(index + 1).take(4) {
            if header(next).is_some() {
                break;
            }
            body.push(next);
        }
        let mut used = None;
        let mut resets = None;
        for part in &body {
            if used.is_none() {
                used = percent_used(part);
            }
            if resets.is_none()
                && let Some(at) = part.find("Resets")
            {
                let text = part[at + "Resets".len()..]
                    .trim_end_matches(|c: char| c.is_whitespace() || c == '│')
                    .trim();
                resets = Some(text.to_owned());
            }
        }
        let Some(used_percent) = used else {
            continue;
        };
        let mut resets_at = resets.and_then(|text| parse_reset(&text, now));
        if matches!(kind, Header::Session) {
            // A session reset outside the five-hour window was misread.
            resets_at = resets_at.filter(|at| {
                (-RESET_TOLERANCE_SECS..=SESSION_RESET_MAX_SECS).contains(&(at - now.as_second()))
            });
        }
        let (bucket, window_key, scope, name, window, minutes) = match kind {
            Header::Session => (
                "five_hour".to_owned(),
                "five_hour",
                AccountUsageScope::AllModels,
                "Claude".to_owned(),
                "Session",
                Some(300),
            ),
            Header::Week => (
                "seven_day".to_owned(),
                "seven_day",
                AccountUsageScope::AllModels,
                "Claude".to_owned(),
                "Week",
                Some(10080),
            ),
            Header::ModelWeek(name) => (
                format!("screen:{}", name.to_ascii_lowercase()),
                "weekly_scoped",
                AccountUsageScope::Model,
                name,
                "Week",
                Some(10080),
            ),
        };
        if windows.iter().any(|w| w.bucket_key == bucket) {
            continue;
        }
        windows.push(AccountUsageWindow {
            bucket_key: bucket,
            window_key: window_key.into(),
            scope,
            name,
            window: window.into(),
            used_percent,
            duration_minutes: minutes,
            resets_at,
            pace: None,
            daily: None,
            series: None,
        });
    }
    windows
}

fn month_number(name: &str) -> Option<i8> {
    let lower = name.to_ascii_lowercase();
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    months
        .iter()
        .position(|month| lower.starts_with(month))
        .map(|index| index as i8 + 1)
}

fn clock(text: &str) -> Option<(i8, i8)> {
    let lower = text.trim().to_ascii_lowercase();
    let (digits, pm) = if let Some(rest) = lower.strip_suffix("pm") {
        (rest, Some(true))
    } else if let Some(rest) = lower.strip_suffix("am") {
        (rest, Some(false))
    } else {
        (lower.as_str(), None)
    };
    let digits = digits.trim();
    let (hour, minute) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<i8>().ok()?, m.parse::<i8>().ok()?),
        None => (digits.parse::<i8>().ok()?, 0),
    };
    if !(0..60).contains(&minute) {
        return None;
    }
    let hour = match pm {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            (hour % 12) + if pm { 12 } else { 0 }
        }
        None if (0..24).contains(&hour) => hour,
        None => return None,
    };
    Some((hour, minute))
}

/// How far in the past a shown reset may be and still be the one shown.
const RESET_TOLERANCE_SECS: i64 = 15 * 60;
/// A session window is five hours long, so its reset is never further away.
const SESSION_RESET_MAX_SECS: i64 = 5 * 60 * 60 + RESET_TOLERANCE_SECS;

/// "1:10pm (America/Los_Angeles)" or "Oct 5 at 8pm (America/Los_Angeles)",
/// as the next such instant relative to `now`, the moment the panel showed
/// it. Unreadable text has no reset time.
pub(super) fn parse_reset(text: &str, now: Timestamp) -> Option<i64> {
    let (when, zone) = match (text.rfind('('), text.rfind(')')) {
        (Some(open), Some(close)) if open < close => (
            text[..open].trim(),
            TimeZone::get(text[open + 1..close].trim()).ok()?,
        ),
        _ => (text.trim(), TimeZone::system()),
    };
    let now_zoned = now.to_zoned(zone.clone());
    let (date, time) = match when.split_once(" at ").or_else(|| when.split_once(", ")) {
        Some((date, time)) => (Some(date.trim()), time.trim()),
        None => (None, when),
    };
    let (hour, minute) = clock(time)?;
    match date {
        Some(date) => {
            let mut parts = date.split_whitespace();
            let month = month_number(parts.next()?)?;
            let day: i8 = parts.next()?.trim_end_matches(',').parse().ok()?;
            let mut year = now_zoned.year();
            loop {
                let candidate = civil::Date::new(year, month, day)
                    .ok()?
                    .at(hour, minute, 0, 0)
                    .to_zoned(zone.clone())
                    .ok()?;
                // A reset a day or more in the past belongs to next year.
                if candidate.timestamp().as_second() >= now.as_second() - 86_400
                    || year > now_zoned.year()
                {
                    return Some(candidate.timestamp().as_second());
                }
                year += 1;
            }
        }
        None => {
            let today = now_zoned
                .date()
                .at(hour, minute, 0, 0)
                .to_zoned(zone.clone())
                .ok()?;
            // A time a few minutes past is today's reset, just passed (the
            // panel was drawn a moment before `now`); only one well past
            // means tomorrow.
            let passed = today.timestamp().as_second() <= now.as_second() - RESET_TOLERANCE_SECS;
            let next = if passed {
                today.tomorrow().ok()?
            } else {
                today
            };
            Some(next.timestamp().as_second())
        }
    }
}

// ---------------------------------------------------------------------------
// The workspace trust dialog in the probe folder

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TrustAction {
    /// No trust dialog on screen.
    Absent,
    /// The dialog is on screen but its selection marker is not readable yet.
    Wait,
    Send(&'static [u8]),
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Moves the `❯` marker onto "Yes, I trust this folder" and confirms only once
/// it is there. "No, exit" is selected by default, so a bare Enter would quit.
pub(super) fn trust_action(screen: &str) -> TrustAction {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(yes_row) = lines
        .iter()
        .position(|line| squash(line).contains("yes,itrustthisfolder"))
    else {
        let all = squash(screen);
        return if all.contains("quicksafetycheck") || all.contains("doyoutrustthefiles") {
            TrustAction::Wait
        } else {
            TrustAction::Absent
        };
    };
    let blank = |row: usize| lines[row].trim().is_empty();
    let mut first = yes_row;
    while first > 0 && !blank(first - 1) {
        first -= 1;
    }
    let mut last = yes_row;
    while last + 1 < lines.len() && !blank(last + 1) {
        last += 1;
    }
    let Some(marker) = (first..=last).find(|row| lines[*row].contains('❯')) else {
        return TrustAction::Wait;
    };
    TrustAction::Send(if marker == yes_row {
        b"\r"
    } else if marker < yes_row {
        b"\x1b[B"
    } else {
        b"\x1b[A"
    })
}

// ---------------------------------------------------------------------------
// The last good reading, kept across launches

#[derive(Serialize, Deserialize)]
struct LastReading {
    version: u32,
    claude: AccountProviderUsage,
}

/// The last good reading saved by an earlier run, or `None` when the file is
/// missing, unreadable, or not a valid reading.
pub(super) fn load_last_reading(path: &Path) -> Option<AccountProviderUsage> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_LAST_READING_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_LAST_READING_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let saved: LastReading = serde_json::from_slice(&bytes).ok()?;
    let claude = saved.claude;
    (saved.version == 1
        && claude.state == AccountUsageState::Available
        && claude.issue.is_none()
        && claude.checked_at.is_some_and(|at| at > 0)
        && !claude.windows.is_empty()
        && valid_report(&claude))
    .then_some(claude)
}

/// Replaces the saved reading atomically. Only the provider DTO is written:
/// windows, percents, reset times and the read time.
pub(super) fn save_last_reading(path: &Path, claude: &AccountProviderUsage) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::create_dir_all(directory)?;
    let bytes = serde_json::to_vec(&LastReading {
        version: 1,
        claude: claude.clone(),
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    std::io::Write::write_all(&mut temporary, &bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn live_shape(fetched_at_ms: i64) -> Value {
        json!({
            "fetchedAtMs": fetched_at_ms,
            "accountUuid": "00000000-0000-0000-0000-000000000000",
            "utilization": {
                "five_hour": {"utilization": 20, "resets_at": "2026-10-01T20:09:59.600697+00:00", "limit_dollars": null},
                "seven_day": {"utilization": 37, "resets_at": "2026-10-06T02:59:59.600727+00:00"},
                "seven_day_oauth_apps": null,
                "seven_day_opus": null,
                "seven_day_sonnet": {"utilization": 12, "resets_at": "2026-10-06T02:59:59Z"},
                "seven_day_cowork": {"utilization": 3, "resets_at": null},
                "seven_day_omelette": {"utilization": 44, "resets_at": null},
                "iguana_necktie": {"utilization": 0, "resets_at": "2026-11-05T07:59:00+00:00"},
                "extra_usage": {"is_enabled": false, "utilization": null},
                "limits": [
                    {"kind": "session", "group": "session", "percent": 20, "resets_at": "2026-10-01T20:09:59.600697+00:00", "scope": null},
                    {"kind": "weekly_all", "group": "weekly", "percent": 37, "resets_at": "2026-10-06T02:59:59.600727+00:00", "scope": null},
                    {"kind": "weekly_scoped", "group": "weekly", "percent": 19, "resets_at": "2026-10-06T02:59:59.601017+00:00", "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}}
                ]
            }
        })
    }

    #[test]
    fn cached_utilization_keeps_known_windows_and_drops_unknown_keys() {
        let cached = parse_cached_usage(&live_shape(1_790_881_585_671)).unwrap();
        assert_eq!(cached.fetched_at_ms, 1_790_881_585_671);
        let names: Vec<(&str, &str, f64)> = cached
            .windows
            .iter()
            .map(|w| (w.name.as_str(), w.window.as_str(), w.used_percent))
            .collect();
        assert_eq!(cached.windows.len(), 4, "{names:?}");
        let session = cached
            .windows
            .iter()
            .find(|w| w.window == "Session")
            .unwrap();
        assert_eq!(
            (session.scope.clone(), session.used_percent),
            (AccountUsageScope::AllModels, 20.0)
        );
        assert_eq!(session.resets_at, Some(1_790_885_399));
        let week = cached
            .windows
            .iter()
            .find(|w| w.window == "Week" && w.scope == AccountUsageScope::AllModels)
            .unwrap();
        assert_eq!(week.used_percent, 37.0);
        assert!(
            cached
                .windows
                .iter()
                .any(|w| w.name == "Fable" && w.used_percent == 19.0)
        );
        assert!(
            cached
                .windows
                .iter()
                .any(|w| w.name == "Sonnet" && w.scope == AccountUsageScope::Model)
        );
        assert!(!cached.windows.iter().any(|w| w.used_percent == 44.0
            || w.used_percent == 3.0
            || w.name.contains("omelette")
            || w.name.contains("iguana")));
        let usage = cached.into_usage();
        assert_eq!(usage.checked_at, Some(1_790_881_585));
        assert_eq!(usage.state, AccountUsageState::Available);
    }

    #[test]
    fn cached_utilization_with_only_legacy_keys_and_null_models() {
        let cached = parse_cached_usage(&json!({
            "fetchedAtMs": 5_000,
            "utilization": {
                "five_hour": {"utilization": 0, "resets_at": null},
                "seven_day": {"utilization": 100, "resets_at": "2026-10-06T03:00:00Z"},
                "seven_day_opus": {"utilization": 50, "resets_at": null},
                "seven_day_sonnet": null
            }
        }))
        .unwrap();
        assert_eq!(cached.windows.len(), 3);
        assert!(
            cached
                .windows
                .iter()
                .any(|w| w.name == "Opus" && w.used_percent == 50.0)
        );
        assert!(
            cached
                .windows
                .iter()
                .any(|w| w.window_key == "five_hour" && w.resets_at.is_none())
        );
    }

    #[test]
    fn cached_utilization_rejects_missing_time_or_body() {
        for bad in [
            json!({"utilization": {"seven_day": {"utilization": 1}}}),
            json!({"fetchedAtMs": 0, "utilization": {"seven_day": {"utilization": 1}}}),
            json!({"fetchedAtMs": 1, "utilization": null}),
            json!({"fetchedAtMs": 1, "utilization": {"seven_day": {"utilization": 140}}}),
            json!({"fetchedAtMs": 1, "utilization": {"iguana_necktie": {"utilization": 1}}}),
        ] {
            assert!(parse_cached_usage(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn config_file_is_read_without_change_and_partial_files_read_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        assert_eq!(read_cached_usage(&path), None);
        let config =
            json!({"numStartups": 3, "projects": {}, "cachedUsageUtilization": live_shape(42_000)});
        let text = serde_json::to_string(&config).unwrap();
        std::fs::write(&path, &text).unwrap();
        let cached = read_cached_usage(&path).unwrap();
        assert_eq!(cached.fetched_at_ms, 42_000);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::write(&path, &text[..text.len() / 2]).unwrap();
        assert_eq!(read_cached_usage(&path), None);
        std::fs::write(&path, r#"{"numStartups": 3}"#).unwrap();
        assert_eq!(read_cached_usage(&path), None);
    }

    /// A capture as the probe's screen renders it: ANSI already replayed by
    /// `Screen`, with the insights section below the limits.
    const CAPTURE: &str = "\
 ▐▛███▜▌   Claude Code v2.1.284
╭──────────────────────────────────────────────────────────────╮
│  Status   Config   Usage                                       │
│                                                                │
│  Current session                                               │
│  █████████▌                                         19% used   │
│  Resets 1:10pm (America/Los_Angeles)                           │
│                                                                │
│  Current week (all models)                                     │
│  ██████████████████▌                                37% used   │
│  Resets Oct 5 at 8pm (America/Los_Angeles)                     │
│                                                                │
│  Current week (Fable)                                          │
│  █████████▌                                         19% used   │
│  Resets Oct 5 at 8pm (America/Los_Angeles)                     │
│                                                                │
│  What's contributing to your limits usage?                     │
│  Current session  Bash 88% used                                │
│  Current week (Secret tool) 77% used                           │
╰──────────────────────────────────────────────────────────────╯";

    #[test]
    fn screen_capture_parses_limits_and_ignores_insights() {
        let now: Timestamp = "2026-10-01T19:06:25Z".parse().unwrap();
        let windows = parse_usage_screen(CAPTURE, now);
        assert_eq!(windows.len(), 3, "{windows:?}");
        let session = &windows[0];
        assert_eq!(
            (session.window.as_str(), session.used_percent),
            ("Session", 19.0)
        );
        // 1:10pm Pacific on Oct 1 is 20:10 UTC.
        assert_eq!(
            session.resets_at,
            Some(
                "2026-10-01T20:10:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        let week = &windows[1];
        assert_eq!(
            (week.scope.clone(), week.used_percent),
            (AccountUsageScope::AllModels, 37.0)
        );
        assert_eq!(
            week.resets_at,
            Some(
                "2026-10-06T03:00:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        let model = &windows[2];
        assert_eq!(
            (model.scope.clone(), model.name.as_str(), model.used_percent),
            (AccountUsageScope::Model, "Fable", 19.0)
        );
        assert!(
            !windows
                .iter()
                .any(|w| w.used_percent == 88.0 || w.used_percent == 77.0)
        );
    }

    #[test]
    fn screen_capture_through_the_terminal_replay() {
        // The same panel drawn with cursor moves, line erases and colors.
        let mut screen = crate::account_usage_claude_screen::Screen::new(40, 120);
        screen.feed(b"\x1b[?25l\x1b[2J\x1b[H> /usage\x1b[1B\r\x1b[2K");
        screen.feed(b"\x1b[3;3H\x1b[1mCurrent session\x1b[22m\x1b[4;3H\x1b[38;5;75m\xe2\x96\x88\xe2\x96\x88\x1b[39m\x1b[4;40H19% used");
        screen.feed(b"\x1b[5;3HResets 1:10pm (America/Los_Angeles)");
        screen.feed(b"\x1b[7;3HCurrent week (all models)\r\n\x1b[2C\xe2\x96\x88 37% used\r\n\x1b[2CResets Oct 5 at 8pm (America/Los_Angeles)");
        screen.feed(
            b"\x1b[11;3HWhat's contributing to your limits usage?\r\n  Current session 90% used",
        );
        let now: Timestamp = "2026-10-01T19:06:25Z".parse().unwrap();
        let windows = parse_usage_screen(&screen.text(), now);
        assert_eq!(windows.len(), 2, "{}", screen.text());
        assert_eq!(windows[0].used_percent, 19.0);
        assert_eq!(windows[1].used_percent, 37.0);
    }

    #[test]
    fn session_reset_is_read_relative_to_the_capture_and_within_five_hours() {
        let panel = "Current session\n██ 30% used\nResets 1:10pm (America/Los_Angeles)";
        // Captured two minutes after 1:10pm Pacific: today's reset, not tomorrow's.
        let just_after: Timestamp = "2026-10-01T20:12:00Z".parse().unwrap();
        let windows = parse_usage_screen(panel, just_after);
        assert_eq!(
            windows[0].resets_at,
            Some(
                "2026-10-01T20:10:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        // Captured an hour after: "tomorrow 1:10pm" is beyond a five-hour
        // session, so no reset time is claimed.
        let hour_after: Timestamp = "2026-10-01T21:10:00Z".parse().unwrap();
        let windows = parse_usage_screen(panel, hour_after);
        assert_eq!(windows[0].used_percent, 30.0);
        assert_eq!(windows[0].resets_at, None);
    }

    #[test]
    fn reset_text_forms() {
        let now: Timestamp = "2026-12-30T12:00:00Z".parse().unwrap();
        // A date earlier in the year than now is next year's.
        assert_eq!(
            parse_reset("Jan 2 at 9am (UTC)", now),
            Some(
                "2027-01-02T09:00:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        // One that passed moments ago is still today's.
        assert_eq!(
            parse_reset("11:50am (UTC)", now),
            Some(
                "2026-12-30T11:50:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        // A clock time well past today is tomorrow's.
        assert_eq!(
            parse_reset("11:30am (UTC)", now),
            Some(
                "2026-12-31T11:30:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        assert_eq!(
            parse_reset("Dec 30, 12:30pm (UTC)", now),
            Some(
                "2026-12-30T12:30:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_second()
            )
        );
        assert_eq!(parse_reset("soon (Not/AZone)", now), None);
        assert_eq!(parse_reset("13pm (UTC)", now), None);
    }

    #[test]
    fn trust_dialog_moves_to_yes_before_confirming() {
        let default = "\
 Quick safety check: Is this a project you created or one you trust?

 ❯ 1. No, exit
   2. Yes, I trust this folder

 Enter to confirm · Esc to cancel";
        assert_eq!(trust_action(default), TrustAction::Send(b"\x1b[B"));
        let moved = default
            .replace("❯ 1. No", "  1. No")
            .replace("  2. Yes", "❯ 2. Yes");
        assert_eq!(trust_action(&moved), TrustAction::Send(b"\r"));
        let unmarked = default.replace('❯', " ");
        assert_eq!(trust_action(&unmarked), TrustAction::Wait);
        assert_eq!(
            trust_action(" Quick safety check: Is this a project"),
            TrustAction::Wait
        );
        assert_eq!(
            trust_action("╭───╮\n│ > │\n╰───╯\n  ? for shortcuts"),
            TrustAction::Absent
        );
        // A marker in an unrelated block never confirms the dialog.
        let elsewhere = "❯ /usage\n\n   1. No, exit\n   2. Yes, I trust this folder";
        assert_eq!(trust_action(elsewhere), TrustAction::Wait);
    }

    #[test]
    fn retry_delays_double_to_an_hour() {
        let minutes: Vec<u64> = (0..6).map(|n| retry_delay(n).as_secs() / 60).collect();
        assert_eq!(minutes, vec![10, 10, 20, 40, 60, 60]);
    }

    fn reading() -> AccountProviderUsage {
        parse_cached_usage(&live_shape(1_790_881_585_671))
            .unwrap()
            .into_usage()
    }

    #[test]
    fn last_reading_round_trip_and_corrupted_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(LAST_READING_FILE);
        assert_eq!(load_last_reading(&path), None);
        save_last_reading(&path, &reading()).unwrap();
        assert_eq!(load_last_reading(&path), Some(reading()));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("accountUuid") && !saved.contains("00000000-0000"));
        // Overwrite in place.
        let mut second = reading();
        second.windows.truncate(1);
        save_last_reading(&path, &second).unwrap();
        assert_eq!(load_last_reading(&path), Some(second));
        for corrupt in [
            "".to_owned(),
            "{not json".to_owned(),
            saved.replace("\"version\":1", "\"version\":2"),
            saved.replace("37.0", "137.0"),
            saved.replace("\"state\":\"available\"", "\"state\":\"failed\""),
            "x".repeat(70_000),
        ] {
            std::fs::write(&path, corrupt).unwrap();
            assert_eq!(load_last_reading(&path), None);
        }
    }

    #[test]
    fn paths_use_claude_config_dir_only_when_absolute() {
        let home = Path::new("/Users/someone");
        let data = Path::new("/data");
        let default = ClaudeUsagePaths::new(data, home, None);
        assert_eq!(default.config_file, home.join(".claude.json"));
        assert_eq!(default.projects_dir, home.join(".claude/projects"));
        assert_eq!(default.probe_dir, data.join(PROBE_DIR_NAME));
        let custom = ClaudeUsagePaths::new(data, home, Some("/cfg".into()));
        assert_eq!(custom.config_file, Path::new("/cfg/.claude.json"));
        assert_eq!(custom.projects_dir, Path::new("/cfg/projects"));
        let relative = ClaudeUsagePaths::new(data, home, Some("cfg".into()));
        assert_eq!(relative.config_file, home.join(".claude.json"));
    }
}
