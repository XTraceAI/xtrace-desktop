//! Shared by the metric tests that check `MetricsDb::hands_off_with_days`
//! against the two standalone reports it combines, on each test's own data.
use jiff::tz::TimeZone;
use xt_metrics::{DayHandsOff, HandsOff, MetricsDb, Window};

/// The combined read equals each standalone method over the same window.
pub fn matches_standalone(
    metrics: &MetricsDb,
    window: Window,
    zone: TimeZone,
) -> (HandsOff, Vec<DayHandsOff>) {
    let shared = metrics.hands_off_with_days(window, zone.clone()).unwrap();
    assert_eq!(
        shared,
        (
            metrics.hands_off(window).unwrap(),
            metrics.hands_off_by_day(window, zone).unwrap(),
        )
    );
    shared
}
