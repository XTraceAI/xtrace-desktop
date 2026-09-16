use jiff::{Timestamp, tz::TimeZone};
use xt_metrics::Window;
fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}

#[test]
fn window_previous_is_equal_length_adjacent_and_checked() {
    let window = Window::new(1_000, 7_000).unwrap();
    assert_eq!(
        window.previous().unwrap(),
        Window::new(-5_000, 1_000).unwrap()
    );
    for (start, end) in [(0, 0), (1, 0), (i64::MIN, i64::MAX)] {
        assert!(Window::new(start, end).is_err());
    }
    let lower = Timestamp::MIN.as_millisecond();
    assert!(Window::new(lower, lower + 1).unwrap().previous().is_err());
}
#[test]
fn window_local_days_clip_partial_days_and_do_not_overlap() {
    let w = Window::new(ms("2026-09-01T13:00:00Z"), ms("2026-09-03T15:00:00Z")).unwrap();
    let days = w
        .local_days(TimeZone::get("America/Los_Angeles").unwrap())
        .unwrap();
    assert_eq!(
        days.iter().map(|d| d.date.to_string()).collect::<Vec<_>>(),
        ["2026-09-01", "2026-09-02", "2026-09-03"]
    );
    assert_eq!(days[0].window.start_ms(), w.start_ms());
    assert_eq!(days[0].window.end_ms(), ms("2026-09-02T07:00:00Z"));
    assert_eq!(days[2].window.end_ms(), w.end_ms());
    assert!(
        days.windows(2)
            .all(|p| p[0].window.end_ms() == p[1].window.start_ms())
    );
}
#[test]
fn window_calendar_days_handle_dst_and_skipped_dates() {
    for (start, end, hours) in [
        ("2026-03-08T08:00:00Z", "2026-03-09T07:00:00Z", 23),
        ("2026-11-01T07:00:00Z", "2026-11-02T08:00:00Z", 25),
    ] {
        let w = Window::new(ms(start), ms(end)).unwrap();
        let days = w
            .local_days(TimeZone::get("America/Los_Angeles").unwrap())
            .unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(
            days[0].window.end_ms() - days[0].window.start_ms(),
            hours * 3_600_000
        );
    }
    let w = Window::new(ms("2011-12-29T10:00:00Z"), ms("2011-12-31T10:00:00Z")).unwrap();
    let days = w
        .local_days(TimeZone::get("Pacific/Apia").unwrap())
        .unwrap();
    assert_eq!(
        days.iter().map(|d| d.date.to_string()).collect::<Vec<_>>(),
        ["2011-12-29", "2011-12-31"]
    );
}

#[test]
fn window_final_partial_day_does_not_require_a_representable_tomorrow() {
    let end = Timestamp::MAX.as_second() * 1_000;
    let w = Window::new(end - 1, end).unwrap();
    assert_eq!(w.local_days(TimeZone::UTC).unwrap()[0].window, w);
}
