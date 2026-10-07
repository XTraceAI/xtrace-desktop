//! The saved break length through the application state: bounds, and the one
//! length the Dashboard's human time and leverage read. Every session and
//! record is synthetic, placed relative to the live clock.
use jiff::{Timestamp, tz::TimeZone};
use serde_json::json;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::state::{AppState, StartupOptions};

const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

fn live(root: &std::path::Path) -> AppState {
    std::fs::create_dir_all(root.join("home")).unwrap();
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

fn event(id: &str, ms: i64, human: bool) -> CanonicalRecord {
    let role = if human { "user" } else { "assistant" };
    serde_json::from_value(json!({"uuid":id,"type":role,
        "timestamp":Timestamp::from_millisecond(ms).unwrap().to_string(),
        "message":{"role":role,"model":"test-claude",
            "content":[{"type":"text","text":"x"}]}}))
    .unwrap()
}

/// Three of your messages two days ago, 40 and then 50 minutes apart, each
/// answered within ten minutes.
fn seed(root: &std::path::Path) {
    let start = Timestamp::now().as_millisecond() - 2 * DAY_MS;
    let mut store = Store::open(root.join("data/xtrace.db")).unwrap();
    let mut records = Vec::new();
    for (index, minutes) in [0, 40, 90].into_iter().enumerate() {
        let at = start + minutes * MINUTE_MS;
        records.push(event(&format!("you-{index}"), at, true));
        records.push(event(&format!("agent-{index}"), at + 10 * MINUTE_MS, false));
    }
    let mut session = SessionMeta::new("work", "claude", SessionSource::Fixture);
    session.surface = Some("cli".into());
    store.upsert_session(&session, false).unwrap();
    store.upsert_records("work", &records, false).unwrap();
}

#[test]
fn the_break_length_is_bounded_and_defaults_to_an_hour() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    assert_eq!(state.human_break().unwrap(), 60);
    for minutes in [0, 4, 241, u32::MAX] {
        assert_eq!(
            state.set_human_break(minutes).unwrap_err().to_string(),
            "break length must be a whole number from 5 to 240 minutes",
            "{minutes}"
        );
        assert_eq!(state.human_break().unwrap(), 60, "{minutes} left no trace");
    }
    state.shutdown();
}

#[test]
fn a_saved_break_length_changes_what_the_dashboard_computes() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    seed(root.path());

    // 60 minutes: 0 → 40 → 90 is one stretch of 90 minutes.
    let hour = state.metrics_dashboard(7).unwrap();
    let human = &hour.human_hours.current;
    assert_eq!(human.break_minutes, 60);
    assert_eq!(human.active_ms, Some(90 * MINUTE_MS as u64));
    assert_eq!(human.messages, Some(3));
    let leverage = hour.tiles.leverage.value.unwrap();
    let agent_h = hour.tiles.agent_hours.value.unwrap();
    assert!(
        (leverage - agent_h / 1.5).abs() < 1e-9,
        "{leverage} vs {agent_h}"
    );

    // 45 minutes: the 50-minute gap is a break; 40 minutes remain.
    assert_eq!(state.set_human_break(45).unwrap(), 45);
    let shorter = state.metrics_dashboard(7).unwrap();
    assert_eq!(shorter.human_hours.current.break_minutes, 45);
    assert_eq!(
        shorter.human_hours.current.active_ms,
        Some(40 * MINUTE_MS as u64)
    );
    assert!((shorter.tiles.leverage.value.unwrap() - agent_h / (40.0 / 60.0)).abs() < 1e-9);

    // 30 minutes: every message stands alone, so leverage has no value and
    // says why, not that you sent nothing.
    assert_eq!(state.set_human_break(30).unwrap(), 30);
    let alone = state.metrics_dashboard(7).unwrap();
    assert_eq!(alone.human_hours.current.active_ms, Some(0));
    assert_eq!(alone.human_hours.current.messages, Some(3));
    assert_eq!(alone.tiles.leverage.value, None);
    assert!(
        alone
            .tiles
            .leverage
            .reason
            .as_deref()
            .unwrap()
            .starts_with("Each message you sent stood alone")
    );
    // Agent hours never read the break length.
    assert_eq!(alone.tiles.agent_hours.value, Some(agent_h));
    state.shutdown();
}

#[test]
fn seven_days_of_your_hours_are_eight_whole_local_days() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    seed(root.path());
    let report = state.metrics_dashboard(7).unwrap();
    let human = &report.human_hours.current;
    assert_eq!(human.by_day.len(), report.days.len());
    // The live clock is almost never exactly on a local midnight; when it
    // is, the range touches seven days and the rows still match `days`.
    if report.days[0].start_ms
        != Timestamp::from_millisecond(report.days[0].start_ms)
            .unwrap()
            .to_zoned(TimeZone::system())
            .start_of_day()
            .unwrap()
            .timestamp()
            .as_millisecond()
    {
        assert_eq!(human.by_day.len(), 8);
    }
    for (row, day) in human.by_day.iter().zip(&report.days) {
        assert_eq!(row.date, day.date);
        // Whole days: each row starts at its local midnight, before or at
        // the reported day's (possibly clipped) start, and ends at or after it.
        assert!(row.start_ms <= day.start_ms && row.end_ms >= day.end_ms);
        let zoned = Timestamp::from_millisecond(row.start_ms)
            .unwrap()
            .to_zoned(TimeZone::system());
        assert_eq!(zoned.start_of_day().unwrap(), zoned, "{}", row.date);
    }
    assert_eq!(human.start_ms, human.by_day[0].start_ms);
    assert_eq!(human.end_ms, human.by_day.last().unwrap().end_ms);
    // The first row is a whole day even though the range starts mid-day.
    assert!(human.by_day[0].start_ms <= report.window.start_ms);
    state.shutdown();
}

#[test]
fn leverage_agent_hours_are_the_whole_day_bars_of_your_hours_days() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    seed(root.path());
    let report = state.metrics_dashboard(7).unwrap();
    let human = &report.human_hours.current;
    let leverage = &report.leverage;
    let bars = &report.pr_effort.current.cohort.by_day;
    // Leverage's days are human time's whole days, bound for bound.
    assert_eq!(leverage.by_day.len(), human.by_day.len());
    for (day, row) in leverage.by_day.iter().zip(&human.by_day) {
        assert_eq!(
            (day.date.as_str(), day.start_ms, day.end_ms),
            (row.date.as_str(), row.start_ms, row.end_ms)
        );
        assert_eq!(day.human_ms, row.active_ms);
    }
    // The total beside human time is those whole days' agent hours added up.
    assert_eq!(
        leverage.agent_ms,
        leverage.by_day.iter().map(|day| day.agent_ms).sum::<u64>()
    );
    // Each whole day holds at least the range's bar for that date; a day the
    // range covers whole, the last one included, is the same number.
    assert_eq!(bars.len(), leverage.by_day.len());
    for (bar, day) in bars.iter().zip(&leverage.by_day) {
        assert_eq!(bar.date, day.date);
        assert!(day.agent_ms >= bar.agent_ms, "{}", day.date);
        if bar.start_ms == day.start_ms {
            assert_eq!(day.agent_ms, bar.agent_ms, "{}", day.date);
        }
    }
    state.shutdown();
}
