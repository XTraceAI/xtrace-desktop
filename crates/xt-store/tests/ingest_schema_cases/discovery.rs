use crate::support::*;
use xt_fixtures::TempDb;
use xt_store::{
    Host, SessionSource,
    ingest::{DiscoveredSession, RecordSourceObservation, SessionSourceObservation},
};

#[test]
fn f20_discovery_without_capture_stays_observable() {
    let fixture = fixture("F20");
    let schema = &fixture.snapshots()["schema"];
    let inputs: Vec<DiscoveredSession> =
        serde_json::from_value(schema["discovery"].clone()).unwrap();
    let mut db = TempDb::empty().unwrap();
    for input in &inputs {
        db.store_mut().observe_discovered_session(input).unwrap();
    }
    let mut expected = inputs.clone();
    expected.sort_by(|a, b| a.native_session_id.cmp(&b.native_session_id));
    let loaded = db.store().discovered_sessions(Host::Cursor).unwrap();
    assert_eq!(loaded, expected);
    assert!(
        db.store()
            .discovered_sessions(Host::Codex)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        loaded.iter().filter(|r| r.started_at_ms.is_none()).count() as u64,
        schema["expected"]["unknown_start"].as_u64().unwrap()
    );
    assert_eq!(
        loaded.iter().filter(|r| !r.discovery_complete).count() as u64,
        schema["expected"]["incomplete"].as_u64().unwrap()
    );
    let sql = connection(db.path());
    for (table, key) in [
        ("sessions", "sessions"),
        ("records", "records"),
        ("capture_receipts", "receipts"),
        ("discovered_sessions", "discoveries"),
    ] {
        assert_eq!(
            scalar(&sql, &format!("SELECT count(*) FROM {table}")),
            schema["expected"][key].as_i64().unwrap()
        );
    }
    assert_eq!(scalar(&sql, "SELECT count(*) FROM session_sources"), 0);
}

#[test]
fn discovery_preserves_identity_and_latest_completeness() {
    let mut db = TempDb::empty().unwrap();
    let mut input = DiscoveredSession {
        host: Host::Cursor,
        native_session_id: "native".into(),
        conversation_id: None,
        surface: None,
        started_at_ms: None,
        last_observed_at: 10,
        discovery_complete: false,
    };
    db.store_mut().observe_discovered_session(&input).unwrap();
    input.conversation_id = Some("canonical".into());
    input.surface = Some("future.desktop".into());
    input.started_at_ms = Some(0);
    input.last_observed_at = 20;
    input.discovery_complete = true;
    db.store_mut().observe_discovered_session(&input).unwrap();
    let mut stale = input.clone();
    stale.last_observed_at = 15;
    stale.discovery_complete = false;
    stale.surface = None;
    db.store_mut().observe_discovered_session(&stale).unwrap();
    assert_eq!(
        db.store().discovered_sessions(Host::Cursor).unwrap(),
        [input.clone()]
    );
    let mut conflict = input.clone();
    conflict.conversation_id = Some("different".into());
    conflict.last_observed_at = 30;
    assert!(
        db.store_mut()
            .observe_discovered_session(&conflict)
            .is_err()
    );
    assert_eq!(
        db.store().discovered_sessions(Host::Cursor).unwrap(),
        [input.clone()]
    );
    input.last_observed_at = 30;
    input.discovery_complete = false;
    db.store_mut().observe_discovered_session(&input).unwrap();
    assert_eq!(
        db.store().discovered_sessions(Host::Cursor).unwrap(),
        [input]
    );
}

#[test]
fn source_observations_accumulate_without_manufacturing_receipts() {
    let (mut db, case) = receipt_db(false);
    let session = &case.receipt.session_id;
    for (first, last) in [(20, 30), (10, 25), (22, 40)] {
        db.store_mut()
            .observe_session_source(&SessionSourceObservation {
                session_id: session.clone(),
                source: SessionSource::Transcript,
                first_seen_at: first,
                last_seen_at: last,
            })
            .unwrap();
    }
    assert_eq!(
        db.store().session_sources(session).unwrap(),
        [SessionSourceObservation {
            session_id: session.clone(),
            source: SessionSource::Transcript,
            first_seen_at: 10,
            last_seen_at: 40
        }]
    );
    let uuid = &case.coverage[0].record_uuid;
    for (presence, conflict) in [(1, 0), (4, 2), (1, 0)] {
        db.store_mut()
            .observe_record_source(&RecordSourceObservation {
                uuid: uuid.clone(),
                source: SessionSource::ReadersCli,
                field_presence: presence,
                conflict_flags: conflict,
            })
            .unwrap();
    }
    assert_eq!(
        db.store().record_sources(uuid).unwrap(),
        [RecordSourceObservation {
            uuid: uuid.clone(),
            source: SessionSource::ReadersCli,
            field_presence: 5,
            conflict_flags: 2
        }]
    );
    assert!(
        db.store_mut()
            .observe_record_source(&RecordSourceObservation {
                uuid: uuid.clone(),
                source: SessionSource::Plugin,
                field_presence: -1,
                conflict_flags: 0
            })
            .is_err()
    );
    assert!(
        db.store_mut()
            .observe_record_source(&RecordSourceObservation {
                uuid: "missing".into(),
                source: SessionSource::Plugin,
                field_presence: 1,
                conflict_flags: 0
            })
            .is_err()
    );
    assert!(
        db.store_mut()
            .observe_session_source(&SessionSourceObservation {
                session_id: "missing".into(),
                source: SessionSource::Plugin,
                first_seen_at: 0,
                last_seen_at: 1
            })
            .is_err()
    );
    assert!(
        db.store_mut()
            .observe_session_source(&SessionSourceObservation {
                session_id: session.clone(),
                source: SessionSource::Plugin,
                first_seen_at: 2,
                last_seen_at: 1
            })
            .is_err()
    );
    assert!(db.store().capture_receipts(session).unwrap().is_empty());
}

#[test]
fn supplemental_schema_constraints_and_record_counts_survive_replay() {
    let (mut db, case) = receipt_db(false);
    let sql = connection(db.path());
    for table in [
        "hosts",
        "settings",
        "source_cursors",
        "native_checkpoints",
        "pull_requests",
        "pr_links",
    ] {
        assert_eq!(scalar(&sql, &format!("SELECT count(*) FROM {table}")), 0);
    }
    sql.execute("INSERT INTO settings VALUES('retention','false')", [])
        .unwrap();
    assert!(
        sql.execute("INSERT INTO settings VALUES('invalid','not-json')", [])
            .is_err()
    );
    assert!(
        sql.execute(
            "INSERT INTO source_cursors VALUES('plugin','native',-1,0)",
            []
        )
        .is_err()
    );
    assert!(
        sql.execute("INSERT INTO hosts(host,installed) VALUES('cursor',2)", [])
            .is_err()
    );
    assert!(
        sql.execute("INSERT INTO pr_links VALUES('missing',1,'exact',0,1)", [])
            .is_err()
    );
    for statement in [
        "UPDATE sessions SET git_branches='{}'",
        "UPDATE sessions SET kind='unknown'",
        "UPDATE records SET is_human=2",
    ] {
        assert!(sql.execute(statement, []).is_err());
    }
    db.store_mut()
        .upsert_records(&case.receipt.session_id, &case.records, false)
        .unwrap();
    assert_eq!(scalar(&sql, "SELECT record_count FROM sessions"), 2);
    let mut new = case.records[0].clone();
    new.uuid = Some("new-record".into());
    let mut invalid = new.clone();
    invalid.uuid = Some("invalid-record".into());
    invalid.message.usage.as_mut().unwrap().input_tokens = Some(-1);
    assert!(
        db.store_mut()
            .upsert_records(&case.receipt.session_id, &[new.clone(), invalid], false)
            .is_err()
    );
    assert_eq!(scalar(&sql, "SELECT record_count FROM sessions"), 2);
    db.store_mut()
        .upsert_records(&case.receipt.session_id, &[new], false)
        .unwrap();
    assert_eq!(scalar(&sql, "SELECT record_count FROM sessions"), 3);
    assert_eq!(scalar(&sql, "SELECT count(*) FROM records"), 3);
}
