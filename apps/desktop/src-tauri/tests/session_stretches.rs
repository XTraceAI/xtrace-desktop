//! One session's M-09 stretches through the app's own command path.
//!
//! The fold is `xt_metrics::MetricsDb::session_stretches`, reviewed on its own;
//! these cases pin the seam the detail timeline depends on. The window is the
//! one every windowed read selects, the answer is the core's answer with not a
//! field added, dropped or recomputed, and its three states cross as three
//! states — an identifier nothing owns, a session M-09 could not measure, and a
//! measured session whose list may honestly be empty.
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use std::path::Path;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::{
    dto::{
        MetricRepeatDensity, MetricRepeatGroup, MetricRepeatThresholds, MetricSessionStretch,
        MetricSessionStretches, MetricToolBlock, MetricUnknownRepeats,
    },
    state::{AppState, StartupOptions},
};

/// Read once per test, so every fixture sits relative to one clock reading and
/// no calendar date can drift out of the window the case is about.
fn anchor() -> Timestamp {
    Timestamp::now()
}

fn at(anchor: Timestamp, days: i64, minute: i64) -> String {
    (anchor - SignedDuration::from_hours(days * 24) + SignedDuration::from_mins(minute)).to_string()
}

fn human(uuid: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(json!({
        "uuid": uuid, "type": "user", "timestamp": ts,
        "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic ask"}]}
    }))
    .unwrap()
}

/// An assistant record whose call sits at block 1, after a line of text, so a
/// locator that ignored the index would name the wrong block.
fn calls(uuid: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(json!({
        "uuid": uuid, "type": "assistant", "timestamp": ts,
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Reading it."},
            {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"path": "synthetic"}}
        ]}
    }))
    .unwrap()
}

fn seed(store: &mut Store, id: &str, surface: Option<&str>, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(id, rows, false).unwrap();
}

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    root
}

fn store(root: &Path) -> Store {
    Store::open(root.join("data/xtrace.db")).unwrap()
}

fn state(root: &Path) -> AppState {
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

/// One stretch, `days` back: a human ask, then a call four minutes later.
fn stretch(store: &mut Store, id: &str, now: Timestamp, days: i64, tag: &str) {
    seed(
        store,
        id,
        Some("raw-surface"),
        &[
            human(&format!("{tag}-ask"), &at(now, days, 0)),
            calls(&format!("{tag}-call"), &at(now, days, 4)),
        ],
    );
}

#[test]
fn a_measured_session_states_its_stretch_with_the_core_duration_and_locator() {
    let now = anchor();
    let root = root();
    stretch(&mut store(root.path()), "session-one", now, 1, "one");
    let state = state(root.path());
    let MetricSessionStretches::Measured { stretches, .. } =
        state.session_stretches("session-one", 7).unwrap()
    else {
        panic!("a session with a human ask and a call is measured");
    };
    assert_eq!(stretches.len(), 1);
    let only = &stretches[0];
    assert_eq!(only.start_uuid, "one-ask");
    assert_eq!(only.end_uuid, "one-call");
    // M-09's own duration on the stored projection, carried as it is.
    assert_eq!(only.duration_ms, 4 * 60 * 1000);
    // A position, not an identity: the record and the block it sits at.
    assert_eq!(
        only.first_tool,
        Some(MetricToolBlock {
            record_uuid: "one-call".into(),
            block_index: 1,
        })
    );
}

#[test]
fn m09_is_the_cores_answer_unchanged_and_m20_sits_beside_it_unchanged() {
    // The conversion refuses any drift, so this holds by construction; it is
    // stated here because the timeline relies on it — a segment on the detail
    // screen must be one of the segments M-09 measured, not a lookalike, and
    // what it says about repeats must be M-20's own answer about that segment.
    let now = anchor();
    let root = root();
    stretch(&mut store(root.path()), "session-one", now, 1, "one");
    let from_app = serde_json::to_value(
        state(root.path())
            .session_stretches("session-one", 7)
            .unwrap(),
    )
    .unwrap();
    let metrics = xt_metrics::MetricsDb::open(root.path().join("data/xtrace.db")).unwrap();
    let window = xt_metrics::Window::new(
        (now - SignedDuration::from_hours(7 * 24)).as_millisecond(),
        Timestamp::now().as_millisecond(),
    )
    .unwrap();
    let m09 =
        serde_json::to_value(metrics.session_stretches(window, "session-one").unwrap()).unwrap();
    let m20 = serde_json::to_value(
        metrics
            .session_repeats(
                window,
                "session-one",
                xt_metrics::RepeatThresholds::default(),
            )
            .unwrap(),
    )
    .unwrap();

    assert_eq!(from_app["state"], "measured");
    assert_eq!(from_app["repeat_thresholds"], m20["thresholds"]);
    let app = from_app["stretches"].as_array().unwrap();
    let (m09, m20) = (
        m09["stretches"].as_array().unwrap(),
        m20["stretches"].as_array().unwrap(),
    );
    assert_eq!((app.len(), m09.len(), m20.len()), (1, 1, 1));
    for field in [
        "start_uuid",
        "end_uuid",
        "start",
        "end",
        "duration_ms",
        "first_tool",
    ] {
        assert_eq!(app[0][field], m09[0][field], "{field}");
    }
    for field in ["active_duration_ms", "repeats", "circling"] {
        assert_eq!(app[0][field], m20[0][field], "{field}");
    }
    // Nothing else crosses: the M-09 fields, the M-20 fields, and no third.
    assert_eq!(app[0].as_object().unwrap().len(), 9);
}

/// An assistant record whose content is exactly these calls, `(tool, input)`,
/// with `None` for a call that stated no input at all.
fn agent(uuid: &str, ts: &str, calls: &[(&str, Option<serde_json::Value>)]) -> CanonicalRecord {
    let content: Vec<_> = calls
        .iter()
        .enumerate()
        .map(|(index, (name, input))| match input {
            Some(input) => json!({"type": "tool_use", "id": format!("toolu_{index}"),
                "name": name, "input": input}),
            None => json!({"type": "tool_use", "id": format!("toolu_{index}"), "name": name}),
        })
        .collect();
    serde_json::from_value(json!({
        "uuid": uuid, "type": "assistant", "timestamp": ts,
        "message": {"role": "assistant", "content": content}
    }))
    .unwrap()
}

fn seconds(anchor: Timestamp, days: i64, second: i64) -> String {
    (anchor - SignedDuration::from_hours(days * 24) + SignedDuration::from_secs(second)).to_string()
}

/// One stretch one day back: an ask, a record making `calls` at `calls_at`
/// seconds, and a closing read at `end_at` seconds.
fn repeat_stretch(
    id: &str,
    calls: &[(&str, Option<serde_json::Value>)],
    calls_at: i64,
    end_at: i64,
) -> (tempfile::TempDir, AppState) {
    let now = anchor();
    let root = root();
    seed(
        &mut store(root.path()),
        id,
        Some("raw-surface"),
        &[
            human("ask", &seconds(now, 1, 0)),
            agent("work", &seconds(now, 1, calls_at), calls),
            agent(
                "done",
                &seconds(now, 1, end_at),
                &[("Read", Some(json!({"file_path": "/synthetic/done"})))],
            ),
        ],
    );
    let state = state(root.path());
    (root, state)
}

fn only_stretch(state: &AppState, id: &str) -> MetricSessionStretch {
    let MetricSessionStretches::Measured { mut stretches, .. } =
        state.session_stretches(id, 7).unwrap()
    else {
        panic!("a session with a human ask and calls is measured");
    };
    assert_eq!(stretches.len(), 1, "{stretches:?}");
    stretches.remove(0)
}

fn edits(count: usize) -> Vec<(&'static str, Option<serde_json::Value>)> {
    (0..count)
        .map(|_| ("Edit", Some(json!({"file_path": "/synthetic/a.rs"}))))
        .collect()
}

#[test]
fn circling_is_the_cores_judgement_at_exactly_both_default_thresholds() {
    // Exactly four active minutes and exactly five repeats: circling.
    let (_root, state) = repeat_stretch("exact", &edits(6), 120, 240);
    let exact = only_stretch(&state, "exact");
    assert_eq!(
        (exact.duration_ms, exact.active_duration_ms),
        (240_000, 240_000)
    );
    assert_eq!(
        exact.repeats,
        MetricRepeatDensity::Measured {
            repeats: 5,
            // Tool name, count and where the earliest of them sits — nothing
            // about which path they edited.
            worst: Some(MetricRepeatGroup {
                tool_name: "Edit".into(),
                count: 6,
                representative: MetricToolBlock {
                    record_uuid: "work".into(),
                    block_index: 0,
                },
            }),
        }
    );
    assert_eq!(exact.circling, Some(true));

    // One repeat fewer, or one active second fewer, is measured and not circling.
    let (_root, state) = repeat_stretch("fewer", &edits(5), 120, 240);
    let fewer = only_stretch(&state, "fewer");
    assert!(matches!(
        fewer.repeats,
        MetricRepeatDensity::Measured { repeats: 4, .. }
    ));
    assert_eq!(fewer.circling, Some(false));
    let (_root, state) = repeat_stretch("shorter", &edits(6), 120, 239);
    let shorter = only_stretch(&state, "shorter");
    assert_eq!(shorter.active_duration_ms, 239_000);
    assert_eq!(shorter.circling, Some(false));
}

#[test]
fn a_stretch_whose_calls_all_differ_measures_zero_and_is_not_circling() {
    let calls = [
        ("Edit", Some(json!({"file_path": "/synthetic/a.rs"}))),
        ("Edit", Some(json!({"file_path": "/synthetic/b.rs"}))),
        ("Bash", Some(json!({"command": "synthetic"}))),
    ];
    let (_root, state) = repeat_stretch("zero", &calls, 60, 600);
    let zero = only_stretch(&state, "zero");
    assert_eq!(
        zero.repeats,
        MetricRepeatDensity::Measured {
            repeats: 0,
            worst: None
        }
    );
    // A measured zero is a measurement: not circling, not unknown.
    assert_eq!(zero.circling, Some(false));
}

#[test]
fn a_call_with_no_comparison_evidence_leaves_repeats_and_circling_unknown() {
    // The same answer a call written before this build compared calls gets:
    // its key is missing. The stretch keeps its place, its durations and its
    // first call; only the repeat count and the judgement are unknown.
    let mut calls = edits(8);
    calls.push(("Edit", None));
    let (_root, state) = repeat_stretch("unknown", &calls, 60, 600);
    let unknown = only_stretch(&state, "unknown");
    assert_eq!(
        unknown.repeats,
        MetricRepeatDensity::Unknown {
            reason: MetricUnknownRepeats::MissingKey
        }
    );
    // Never "not circling": whether it is was not established.
    assert_eq!(unknown.circling, None);
    assert_eq!(unknown.duration_ms, 600_000);
    assert_eq!(unknown.active_duration_ms, 600_000);
    assert_eq!(
        unknown.first_tool,
        Some(MetricToolBlock {
            record_uuid: "work".into(),
            block_index: 0,
        })
    );
}

#[test]
fn active_time_is_carried_separately_from_elapsed_time() {
    // Twenty repeated edits, then an idle hour before the closing read. M-09's
    // elapsed duration includes the wait; M-05's fold does not, so the stretch
    // is not circling on active time although it lasted over an hour.
    let (_root, state) = repeat_stretch("idle", &edits(20), 60, 60 + 3_600);
    let idle = only_stretch(&state, "idle");
    assert_eq!(idle.duration_ms, 3_660_000);
    assert_eq!(idle.active_duration_ms, 60_000);
    assert!(matches!(
        idle.repeats,
        MetricRepeatDensity::Measured { repeats: 19, .. }
    ));
    assert_eq!(idle.circling, Some(false));
}

#[test]
fn nothing_a_call_said_and_no_comparison_key_reaches_the_response() {
    let calls = [
        ("Bash", Some(json!({"command": "synthetic-secret-command"}))),
        ("Bash", Some(json!({"command": "synthetic-secret-command"}))),
        (
            "Edit",
            Some(json!({"file_path": "/synthetic/secret/path.rs"})),
        ),
        ("Grep", Some(json!({"pattern": "synthetic-secret-pattern"}))),
    ];
    let (_root, state) = repeat_stretch("private", &calls, 60, 600);
    let wire = serde_json::to_string(&state.session_stretches("private", 7).unwrap()).unwrap();
    for said in [
        "synthetic-secret",
        "/synthetic/secret",
        "command",
        "file_path",
        "pattern",
    ] {
        assert!(!wire.contains(said), "{said} crossed: {wire}");
    }
    // The comparison key is 64 lowercase hex characters; no run of hex that
    // long appears anywhere in the answer, and neither does its name.
    let longest_hex = wire
        .split(|c: char| !c.is_ascii_hexdigit())
        .map(str::len)
        .max()
        .unwrap_or(0);
    assert!(longest_hex < 32, "{wire}");
    assert!(!wire.contains("key") && !wire.contains("digest"), "{wire}");
}

#[test]
fn an_identifier_no_session_owns_is_missing_not_an_empty_measurement() {
    let now = anchor();
    let root = root();
    stretch(&mut store(root.path()), "session-one", now, 1, "one");
    let state = state(root.path());
    // Exact, as every other read of one session is: not a prefix, not a
    // whitespace variant, not a case variant.
    for id in [
        "no-such-session",
        "session-on",
        " session-one",
        "SESSION-ONE",
        "",
    ] {
        assert_eq!(
            state.session_stretches(id, 7).unwrap(),
            MetricSessionStretches::Missing,
            "{id:?}"
        );
    }
}

#[test]
fn a_measured_session_with_nothing_in_the_window_is_an_honest_empty_list() {
    // Indexed, measured, and simply without a stretch in this window — which
    // is a different fact from missing and from unmeasured.
    let now = anchor();
    let root = root();
    stretch(&mut store(root.path()), "session-old", now, 20, "old");
    let state = state(root.path());
    assert_eq!(
        state.session_stretches("session-old", 7).unwrap(),
        MetricSessionStretches::Measured {
            stretches: vec![],
            repeat_thresholds: MetricRepeatThresholds {
                active_ms: 240_000,
                repeats: 5,
            },
        }
    );
}

#[test]
fn the_selected_window_is_the_window_the_stretches_come_from() {
    // Twenty days back: outside the shortest window by thirteen days, inside
    // the longest by ten.
    let now = anchor();
    let root = root();
    stretch(&mut store(root.path()), "session-old", now, 20, "old");
    let state = state(root.path());
    let counted = |days| match state.session_stretches("session-old", days).unwrap() {
        MetricSessionStretches::Measured { stretches, .. } => stretches.len(),
        other => panic!("{other:?}"),
    };
    assert_eq!((counted(7), counted(30)), (0, 1));
    // An unsupported window is refused rather than quietly answered.
    assert!(state.session_stretches("session-old", 99).is_err());
}

#[test]
fn an_excluded_surface_is_named_as_the_reason_a_session_is_unmeasured() {
    // Three qualifying sessions on one raw surface, every record at one
    // instant: the surface's timestamp health excludes all of them. The
    // reason names the surface and its counts — it does not claim that this
    // session's own timestamps are all absent.
    let now = anchor();
    let root = root();
    {
        let mut store = store(root.path());
        let instant = at(now, 1, 0);
        for index in 0..3 {
            let rows: Vec<_> = (0..6)
                .map(|block| {
                    let uuid = format!("batched-{index}-{block}");
                    if block == 0 {
                        human(&uuid, &instant)
                    } else {
                        calls(&uuid, &instant)
                    }
                })
                .collect();
            seed(
                &mut store,
                &format!("batched-{index}"),
                Some("raw-batched"),
                &rows,
            );
        }
    }
    let state = state(root.path());
    let MetricSessionStretches::Unmeasured {
        excluded_surface: Some(surface),
    } = state.session_stretches("batched-0", 7).unwrap()
    else {
        panic!("a session on an excluded surface is unmeasured, with the surface named");
    };
    assert_eq!(surface.host, "claude");
    assert_eq!(surface.surface.as_deref(), Some("raw-batched"));
    assert_eq!(surface.qualifying_sessions, 3);
    assert_eq!(surface.degenerate_sessions, 3);
}

#[test]
fn reading_stretches_writes_nothing() {
    let now = anchor();
    let root = root();
    stretch(&mut store(root.path()), "session-one", now, 1, "one");
    let before = store(root.path()).counts().unwrap();
    let state = state(root.path());
    for days in [7, 14, 30] {
        state.session_stretches("session-one", days).unwrap();
    }
    drop(state);
    assert_eq!(store(root.path()).counts().unwrap(), before);
}
