//! Shared by the metric tests that check `MetricsDb::window_activity` against
//! the standalone reports it combines, on each test's own fixture.
use jiff::tz::TimeZone;
use xt_metrics::{MetricsDb, PriceCatalog, TypingRate, Window, WindowActivity};

/// The combined read equals each standalone method over the same window.
pub fn matches_standalone(
    metrics: &MetricsDb,
    window: Window,
    zone: TimeZone,
    typing_rate: TypingRate,
    confirmed_only: bool,
    catalog: &PriceCatalog,
) -> WindowActivity {
    let shared = metrics
        .window_activity(window, zone.clone(), typing_rate, confirmed_only, catalog)
        .unwrap();
    assert_eq!(
        shared,
        WindowActivity {
            spans: metrics.active_spans(window).unwrap(),
            human: metrics
                .human_time(window, typing_rate, zone.clone())
                .unwrap(),
            concurrency: metrics.concurrency(window).unwrap(),
            pr_effort: metrics
                .pr_effort(window, zone, confirmed_only, catalog)
                .unwrap(),
        }
    );
    shared
}
