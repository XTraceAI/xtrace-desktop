//! What each standalone reader accepts from an unusual selected response, and
//! that the combined report read fails exactly when one of the reports it
//! combines fails. The responses are written straight into the selection
//! view, so values no writer stores can be read.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::json;
use xt_fixtures::TempDb;
use xt_metrics::{Error, MetricsDb, PriceCatalog, UsageGap, Window};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};
mod report_support;

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
/// One response row in the view's column order. `ts_ms` follows `ts` unless
/// `ts` is not a time.
fn response(session: &str, model: &str, ts: &str, counters: [&str; 4], split: &str) -> String {
    let ts_ms = ts
        .parse::<Timestamp>()
        .map_or(ms("2026-09-07T12:00:00Z"), |t| t.as_millisecond());
    let [input, output, read, write] = counters;
    format!(
        "({session},'claude',{model},'cli','{ts}',{ts_ms},{input},{output},{read},{write},{split},NULL,'standard')"
    )
}
/// A database whose only selected responses are `rows`, and whose session
/// `s` has one human event in the window, so it is in coverage's cohort.
fn database(rows: &[String]) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    let session = SessionMeta::new("s", "claude", SessionSource::Fixture);
    db.store_mut().upsert_session(&session, false).unwrap();
    let event: CanonicalRecord = serde_json::from_value(json!({"uuid":"h","type":"user","timestamp":"2026-09-07T11:00:00Z","message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}})).unwrap();
    db.store_mut().upsert_records("s", &[event], false).unwrap();
    Connection::open(db.path())
        .unwrap()
        .execute_batch(&format!(
            "DROP VIEW v_response_usage; CREATE VIEW v_response_usage(session_id,host,model,surface,ts,ts_ms,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,service_tier) AS VALUES {}",
            rows.join(",")
        ))
        .unwrap();
    db
}
fn good() -> String {
    response(
        "'s'",
        "'m'",
        "2026-09-07T12:00:00Z",
        ["1", "1", "0", "0"],
        "NULL",
    )
}
/// Which of the current window's standalone reports succeed:
/// (activity with M-19, tokens, favorite model, cost, coverage).
type Outcomes = (bool, bool, bool, bool, bool);
fn outcomes(db: &TempDb) -> (Outcomes, report_support::Standalone) {
    let metrics = MetricsDb::open(db.path()).unwrap();
    let catalog = PriceCatalog::bundled().unwrap();
    let read = report_support::matches_standalone(
        &metrics,
        window(),
        window().end_ms(),
        &[],
        TimeZone::UTC,
        &catalog,
    );
    let current = &read.current;
    (
        (
            current.activity.is_ok(),
            current.tokens.is_ok(),
            current.favorite.is_ok(),
            current.cost.is_ok(),
            read.coverage.is_ok(),
        ),
        read,
    )
}

#[test]
fn report_failures_are_exactly_the_standalone_failures() {
    let end = "2026-09-08T00:00:00.5Z";
    for (name, row, expected) in [
        (
            // Coverage only asks whether a counter is recorded.
            "negative cache read",
            response(
                "'s'",
                "'m'",
                "2026-09-07T12:00:01Z",
                ["1", "1", "-1", "0"],
                "NULL",
            ),
            (false, false, true, false, true),
        ),
        (
            "negative output",
            response(
                "'s'",
                "'m'",
                "2026-09-07T12:00:01Z",
                ["1", "-1", "0", "0"],
                "NULL",
            ),
            (false, false, false, false, true),
        ),
        (
            // Only the pricing reads decode the cache split.
            "negative cache split",
            response(
                "'s'",
                "'m'",
                "2026-09-07T12:00:01Z",
                ["1", "1", "0", "0"],
                "-1",
            ),
            (false, true, true, false, true),
        ),
        (
            // Past the exact end but inside the candidate second: the pricing
            // reads decode it before the window applies; the others skip it.
            "negative counter after the exact end",
            response("'s'", "'m'", end, ["-1", "1", "0", "0"], "NULL"),
            (false, true, true, false, true),
        ),
        (
            "malformed timestamp",
            response("'s'", "'m'", "not a time", ["1", "1", "0", "0"], "NULL"),
            (false, false, false, false, false),
        ),
        (
            // The favorite model skips an absent output before its model.
            "integer model without output",
            response(
                "'s'",
                "5",
                "2026-09-07T12:00:01Z",
                ["1", "NULL", "0", "0"],
                "NULL",
            ),
            (false, false, true, false, false),
        ),
        (
            // Coverage skips a session outside its event cohort first.
            "integer model outside the cohort",
            response(
                "'x'",
                "5",
                "2026-09-07T12:00:01Z",
                ["1", "NULL", "0", "0"],
                "NULL",
            ),
            (false, false, true, false, true),
        ),
        (
            // The cost and favorite reports do not read the session.
            "missing session",
            response(
                "NULL",
                "'m'",
                "2026-09-07T12:00:01Z",
                ["1", "1", "0", "0"],
                "NULL",
            ),
            (false, false, true, true, false),
        ),
    ] {
        let db = database(&[good(), row]);
        let (actual, _) = outcomes(&db);
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn report_overflow_unknown_and_zero_match_standalone() {
    // Output overflows the token and favorite sums only; nothing is priced.
    // Two of these still fit an unsigned 64-bit sum; three do not.
    let max = i64::MAX.to_string();
    let big = |second: u32| {
        response(
            "'s'",
            "'m'",
            &format!("2026-09-07T12:00:{second:02}Z"),
            ["1", &max, "0", "0"],
            "NULL",
        )
    };
    let (actual, read) = outcomes(&database(&[big(1), big(2), big(3)]));
    assert_eq!(actual, (true, false, false, true, true));
    assert!(matches!(read.current.tokens, Err(Error::CounterOverflow)));
    assert!(matches!(read.current.favorite, Err(Error::CounterOverflow)));

    // Absent counters stay unknown, a recorded zero is measured.
    let absent = response("'s'", "'m'", "2026-09-07T12:00:01Z", ["NULL"; 4], "NULL");
    let zero = response("'s'", "'m'", "2026-09-07T12:00:02Z", ["0"; 4], "NULL");
    let (actual, read) = outcomes(&database(&[absent, zero]));
    assert_eq!(actual, (true, true, true, true, true));
    let tokens = read.current.tokens.unwrap();
    assert_eq!(tokens.total.measured_responses, 1);
    assert_eq!(tokens.total.counters.input_tokens, None);
    assert_eq!(
        read.coverage.unwrap().usage.total.gaps,
        [UsageGap::IncompleteCounters]
    );
    let favorite = read.current.favorite.unwrap();
    assert_eq!(
        (favorite.model.as_deref(), favorite.output_tokens),
        (Some("m"), Some(0))
    );
}

#[test]
fn report_fails_on_the_previous_windows_responses_too() {
    let previous = response(
        "'s'",
        "'m'",
        "2026-08-30T12:00:00Z",
        ["-1", "1", "0", "0"],
        "NULL",
    );
    let (actual, read) = outcomes(&database(&[good(), previous]));
    assert_eq!(actual, (true, true, true, true, true));
    assert!(read.previous.tokens.is_err());
    assert!(read.previous.cost.is_err());
    assert!(read.previous.favorite.is_ok());
}
