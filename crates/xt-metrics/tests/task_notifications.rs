//! Claude Code task notifications — the `type: "user"` lines Claude Code
//! writes itself when a background agent, command or monitor finishes — are
//! automatic inputs, not a person's messages, everywhere the index measures
//! one: M-02's human messages and characters, the hours estimate and the
//! span bubble's last prompt. The structural `origin` marker is the evidence;
//! a message that merely starts with the same tag stays a person's. Inputs
//! indexed before the marker was read are corrected by reading them again.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{
    AutomaticText, MetricsDb, PromptText, SpanAutomatic, SpanDetail, SpanPrompt, TypingRate, Window,
};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, batch::IngestBatch, retention::RetentionMode,
};

const SESSION: &str = "claude-synthetic";

fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn at(hour: u32, minute: u32) -> String {
    format!("2026-09-07T{hour:02}:{minute:02}:00Z")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn input(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn reply(uuid: &str, time: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":time,"requestId":format!("req-{uuid}"),
        "message":{"id":format!("msg-{uuid}"),"role":"assistant","model":"fixture-model-v1",
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
    )
}
fn notification(summary: Option<&str>) -> String {
    let summary = summary
        .map(|text| format!("<summary>{text}</summary>"))
        .unwrap_or_default();
    format!(
        "<task-notification><task-id>bul9vnv11</task-id><tool-use-id>toolu_synthetic</tool-use-id>\
         <status>completed</status>{summary}<result>Synthetic result</result></task-notification>"
    )
}

/// Six records: the person's request (9:00), a reply, an agent finishing
/// (9:05), another reply, a second notification (9:08) and a user message
/// that only quotes the tag without Claude Code's marker (9:09). `marked`
/// names the inputs whose native lines carry `origin.kind`.
fn rows() -> Vec<CanonicalRecord> {
    vec![
        input("p1", &at(9, 0), "map the prompt classification sources"),
        reply("a1", &at(9, 1)),
        input(
            "n1",
            &at(9, 5),
            &notification(Some("Agent \"Map prompt classification sources\" finished")),
        ),
        reply("a2", &at(9, 6)),
        input(
            "n2",
            &at(9, 8),
            &notification(Some(
                "Background command \"pnpm check\" completed (exit code 0)",
            )),
        ),
        input("q1", &at(9, 9), &notification(Some("pasted by a person"))),
    ]
}
const MARKED: [&str; 2] = ["n1", "n2"];

/// Read the session as the native Claude reader's writer does: the markers
/// travel beside the records when `markers` is set, and not at all when it
/// is not — what a build before this change stored.
fn read(store: &mut xt_store::Store, rows: &[CanonicalRecord], markers: bool, keep: bool) {
    let mut meta = SessionMeta::new(SESSION, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(SESSION.into());
    meta.surface = Some("cli".into());
    store
        .set_retention_mode(if keep {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    let marks = rows
        .iter()
        .map(|row| MARKED.contains(&row.uuid.as_deref().unwrap()))
        .collect::<Vec<_>>();
    let mut batch = IngestBatch::new(&meta, rows, keep);
    if markers {
        batch.task_notifications = &marks;
    }
    let saved = store.apply_ingest_batch(&batch).unwrap();
    assert!(
        saved
            .records
            .iter()
            .all(|row| row.disposition.is_accepted())
    );
}

fn database(markers: bool, keep: bool) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    read(db.store_mut(), &rows(), markers, keep);
    db
}

/// (raw is_human, effective is_human, human_is_eligible, confirmed) per input.
fn classified(db: &TempDb) -> Vec<(String, i64, i64, i64, i64)> {
    let connection = Connection::open(db.path()).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT uuid,raw_is_human,is_human,human_is_eligible,confirmed_automated_input
             FROM v_records WHERE type='user' ORDER BY ts",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn window() -> Window {
    Window::new(ms("2026-09-07T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}

#[test]
fn the_origin_marker_makes_an_input_automatic_and_nothing_else_does() {
    let db = database(true, false);
    assert_eq!(
        classified(&db),
        [
            ("p1".into(), 1, 1, 1, 0),
            // Ingestion's own classification is untouched; the proof is read
            // as an override, as a confirmation is.
            ("n1".into(), 1, 0, 0, 1),
            ("n2".into(), 1, 0, 0, 1),
            // The same tag without Claude Code's marker is a person's message:
            // the text is never the evidence.
            ("q1".into(), 1, 1, 1, 0),
        ]
    );
    let counts = MetricsDb::open(db.path())
        .unwrap()
        .counts(window(), TypingRate::default())
        .unwrap();
    assert_eq!(counts.human_messages, Some(2));
}

#[test]
fn an_index_built_before_the_marker_is_corrected_by_reading_it_again() {
    // An older build stored every notification as the person's message.
    let mut db = database(false, false);
    let before: Vec<_> = classified(&db);
    assert!(before.iter().all(|row| row.2 == 1));
    let records = |db: &TempDb| {
        let connection = Connection::open(db.path()).unwrap();
        let mut statement = connection
            .prepare("SELECT uuid,is_human,has_conflict,text_len,first_seen_at FROM records ORDER BY uuid")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let stored = records(&db);
    // The replay this build's migration asks for reads the same lines again,
    // now with their markers. Twice, to show it settles.
    for _ in 0..2 {
        read(db.store_mut(), &rows(), true, false);
        assert_eq!(classified(&db), classified(&database(true, false)));
        // Not one stored record changed or became conflicted.
        assert_eq!(records(&db), stored);
    }
    let proofs: i64 = Connection::open(db.path())
        .unwrap()
        .query_row("SELECT count(*) FROM task_notification_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(proofs, 2);
}

#[test]
fn the_hours_estimate_counts_only_the_persons_characters() {
    let hours = |db: &TempDb| {
        MetricsDb::open(db.path())
            .unwrap()
            .human_time(window(), TypingRate::new(60).unwrap(), TimeZone::UTC)
            .unwrap()
            .human_minutes_est
            .unwrap()
    };
    let person = [
        "map the prompt classification sources",
        &notification(Some("pasted by a person")),
    ]
    .iter()
    .map(|text| text.chars().count())
    .sum::<usize>() as f64;
    let notifications = [
        notification(Some("Agent \"Map prompt classification sources\" finished")),
        notification(Some(
            "Background command \"pnpm check\" completed (exit code 0)",
        )),
    ]
    .iter()
    .map(|text| text.chars().count())
    .sum::<usize>() as f64;
    assert_eq!(
        hours(&database(false, false)),
        (person + notifications) / 60.0
    );
    assert_eq!(hours(&database(true, false)), person / 60.0);
}

fn detail(db: &TempDb, start: &str, end: &str) -> (SpanPrompt, SpanAutomatic) {
    match MetricsDb::open(db.path())
        .unwrap()
        .span_detail(
            SESSION,
            ms(start),
            ms(end),
            &xt_metrics::PriceCatalog::bundled().unwrap(),
        )
        .unwrap()
    {
        SpanDetail::Indexed {
            prompt, automatic, ..
        } => (prompt, automatic),
        SpanDetail::Missing => panic!("the session is indexed"),
    }
}

#[test]
fn the_bubble_shows_the_latest_notification_in_the_span_beside_the_persons_message() {
    let db = database(true, true);
    // 9:00–9:08: the latest notification is the command; the last message a
    // person typed is still the request, never a notification.
    let (prompt, automatic) = detail(&db, &at(9, 0), &at(9, 8));
    assert!(matches!(
        prompt,
        SpanPrompt::Found { ref record_uuid, text: PromptText::Stored { ref text, .. }, .. }
            if record_uuid == "p1" && text == "map the prompt classification sources"
    ));
    assert!(matches!(
        automatic,
        SpanAutomatic::Found { at_ms, ref record_uuid, text: AutomaticText::Stored { ref text, truncated: false } }
            if at_ms == ms(&at(9, 8)) && record_uuid == "n2"
                && text == "Background command \"pnpm check\" completed (exit code 0)"
    ));
    // 9:00–9:06 ends before the second: the agent's is the latest.
    let (_, automatic) = detail(&db, &at(9, 0), &at(9, 6));
    assert!(
        matches!(automatic, SpanAutomatic::Found { ref record_uuid, .. } if record_uuid == "n1")
    );
    // A notification before the span is not the span's.
    assert_eq!(
        detail(&db, &at(9, 9), &at(9, 9)).1,
        SpanAutomatic::NoNotification
    );
    // Without the marker there is no notification, and the quoted tag is the
    // person's last message.
    let unmarked = database(false, true);
    let (prompt, automatic) = detail(&unmarked, &at(9, 0), &at(9, 8));
    assert_eq!(automatic, SpanAutomatic::NoNotification);
    assert!(matches!(prompt, SpanPrompt::Found { ref record_uuid, .. } if record_uuid == "n2"));
}

#[test]
fn a_notification_without_a_summary_or_without_kept_words_says_so() {
    let mut db = TempDb::empty().unwrap();
    let mut rows = rows();
    rows[4] = input("n2", &at(9, 8), &notification(None));
    read(db.store_mut(), &rows, true, true);
    assert!(matches!(
        detail(&db, &at(9, 0), &at(9, 8)).1,
        SpanAutomatic::Found {
            text: AutomaticText::NoSummary,
            ..
        }
    ));
    // Metadata-only retention kept the record's content, but the index keeps
    // the summary's short preview, so no source has to be read.
    let mut kept = database(true, false);
    assert!(matches!(
        detail(&kept, &at(9, 0), &at(9, 8)).1,
        SpanAutomatic::Found { ref record_uuid, text: AutomaticText::Stored { ref text, truncated: false }, .. }
            if record_uuid == "n2" && text == "Background command \"pnpm check\" completed (exit code 0)"
    ));
    // Once stored text is deleted no words are kept at all: the caller reads
    // them from the session's source by the saved identity.
    kept.store_mut()
        .purge_content(&xt_store::retention::ContentRegistry::default())
        .unwrap();
    assert!(matches!(
        detail(&kept, &at(9, 0), &at(9, 8)).1,
        SpanAutomatic::Found { ref record_uuid, text: AutomaticText::NotStored, .. }
            if record_uuid == "n2"
    ));
    // A notification without a summary keeps no preview either.
    let mut db = TempDb::empty().unwrap();
    read(db.store_mut(), &rows, true, false);
    assert!(matches!(
        detail(&db, &at(9, 0), &at(9, 8)).1,
        SpanAutomatic::Found {
            text: AutomaticText::NotStored,
            ..
        }
    ));
}

/// The previews the index keeps whatever the retention mode: the person's
/// messages and each proven notification's summary, never the notification's
/// text as a person's message, and nothing for an unmarked replay that the
/// marker later corrects beyond what the current classification allows.
#[test]
fn previews_are_kept_for_the_persons_messages_and_the_notifications_summaries() {
    let previews = |db: &TempDb| -> Vec<(String, String, String)> {
        let connection = Connection::open(db.path()).unwrap();
        let mut statement = connection
            .prepare("SELECT record_uuid,kind,text FROM record_previews ORDER BY record_uuid")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let db = database(true, false);
    assert_eq!(
        previews(&db),
        [
            (
                "n1".into(),
                "automatic".into(),
                "Agent \"Map prompt classification sources\" finished".into()
            ),
            (
                "n2".into(),
                "automatic".into(),
                "Background command \"pnpm check\" completed (exit code 0)".into()
            ),
            (
                "p1".into(),
                "person".into(),
                "map the prompt classification sources".into()
            ),
            (
                "q1".into(),
                "person".into(),
                notification(Some("pasted by a person"))
            ),
        ]
    );
    let (prompt, _) = detail(&db, &at(9, 0), &at(9, 8));
    assert!(matches!(
        prompt,
        SpanPrompt::Found { ref record_uuid, text: PromptText::Stored { ref text, truncated: false }, .. }
            if record_uuid == "p1" && text == "map the prompt classification sources"
    ));
    // An index read without the marker kept the notifications as person
    // previews; reading again with the marker replaces them with their
    // summaries, and the person's last message is never a notification.
    let mut old = database(false, false);
    assert_eq!(
        previews(&old)
            .iter()
            .filter(|row| row.1 == "person")
            .count(),
        4
    );
    read(old.store_mut(), &rows(), true, false);
    assert_eq!(previews(&old), previews(&db));
    let (prompt, automatic) = detail(&old, &at(9, 0), &at(9, 8));
    assert!(matches!(prompt, SpanPrompt::Found { ref record_uuid, .. } if record_uuid == "p1"));
    assert!(matches!(
        automatic,
        SpanAutomatic::Found { ref record_uuid, text: AutomaticText::Stored { .. }, .. } if record_uuid == "n2"
    ));
}
