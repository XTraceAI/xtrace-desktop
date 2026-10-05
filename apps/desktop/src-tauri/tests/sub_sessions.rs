//! A verified sub-session's parent, as the Sessions page and the Dashboard
//! lane context carry it: an exact, indexed parent identity beside rows the
//! reads already return, and nothing else. Membership, order, cursors,
//! search, every measurement, span and output are byte-for-byte what they
//! were before the relation existed, and a parent that is not indexed is
//! never a link.
use jiff::{SignedDuration, Timestamp, tz::TimeZone};
use serde_json::{Value, json};
use xt_metrics::{MetricsDb, TypingRate};
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    creation::{
        CLI_ARTIFACT_CREATE_VERSION, CODEX_THREAD_SPAWN_VERSION, CliArtifactCreationProof,
        CreationDisposition, CreationEvidence, CreationWitness, SessionCreationProof,
    },
};
use xtrace_desktop::{
    dashboard::{assemble, fixture_catalog},
    dto::{
        DashboardMetrics, MetricClock, SessionPage, SessionParentLink, SessionQuery,
        SubSessionEvidence, session_page, session_row,
    },
};

const PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const CHILD: &str = "019a0000-0000-7000-8000-0000000000bb";
const ORPHAN: &str = "019a0000-0000-7000-8000-0000000000cc";
const UNINDEXED: &str = "019a0000-0000-7000-8000-0000000000dd";

#[test]
fn native_reviewer_parent_has_its_own_dto_evidence() {
    let link: SessionParentLink = xt_store::session_list::SessionParent {
        session_id: id(PARENT),
        host: "codex".into(),
        title: None,
        evidence: xt_store::session_list::ParentEvidence::NativeReviewer,
    }
    .into();
    assert_eq!(link.evidence, SubSessionEvidence::NativeReviewer);
    assert_eq!(
        serde_json::to_value(link).unwrap()["evidence"],
        "native_reviewer"
    );
}

fn id(native: &str) -> String {
    format!("codex-{native}")
}

/// The pinned instant every read is taken at, so two reads of the same
/// state are byte-for-byte comparable.
fn now() -> Timestamp {
    "2026-09-08T00:00:00Z".parse().unwrap()
}

/// `minutes` after the start of the day before [`now`].
fn at(minutes: i64) -> String {
    (now() - SignedDuration::from_hours(24) + SignedDuration::from_mins(minutes)).to_string()
}

fn record(uuid: &str, ts: &str, human: bool, output: u64) -> CanonicalRecord {
    let role = if human { "user" } else { "assistant" };
    let mut message = json!({"role":role,"model":"test-model",
        "content":[{"type":"text","text":"Synthetic"}]});
    if !human {
        message["id"] = json!(format!("response-{uuid}"));
        message["usage"] = json!({"input_tokens":10,"output_tokens":output,
            "cache_read_input_tokens":0,"cache_creation_input_tokens":0});
    }
    serde_json::from_value(json!({"uuid":uuid,"type":role,"timestamp":ts,
        "requestId":format!("request-{uuid}"),"message":message}))
    .unwrap()
}

fn codex(store: &mut Store, native: &str, started: i64, records: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id(native), "codex", SessionSource::ReadersCli);
    session.native_session_id = Some(native.into());
    session.started_at_ms = Some(started);
    session.cwd = Some(format!("/repo/{native}"));
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(&id(native), records, false).unwrap();
}

fn spawn(child: &str, parent: &str) -> SessionCreationProof {
    SessionCreationProof {
        child_session_id: id(child),
        child_host: Host::Codex,
        child_native_session_id: child.into(),
        parent_host: Host::Codex,
        parent_native_session_id: parent.into(),
        evidence_kind: CreationEvidence::CodexThreadSpawn,
        evidence_version: CODEX_THREAD_SPAWN_VERSION,
        witness: CreationWitness::RolloutOpeningSessionMeta,
    }
}

/// Every page of a query, following its cursor, at the pinned instant.
fn pages(metrics: &MetricsDb, search: &str) -> Vec<SessionPage> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = session_page(
            metrics,
            7,
            now().as_millisecond(),
            TimeZone::UTC,
            MetricClock::Fixture,
            SessionQuery {
                search,
                after: after.as_deref(),
                ..Default::default()
            },
        )
        .unwrap();
        after = page.next.clone();
        all.push(page);
        if after.is_none() {
            return all;
        }
    }
}

fn dashboard(metrics: &MetricsDb) -> DashboardMetrics {
    assemble(
        metrics,
        7,
        now().as_millisecond(),
        TimeZone::UTC,
        MetricClock::Fixture,
        &fixture_catalog(None).unwrap(),
        TypingRate::default(),
    )
    .unwrap()
}

/// Compare every agent/work field; assert the changed parent and Human fields separately.
fn without_parent_and_human(value: &impl serde::Serialize) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for key in ["parent", "human_messages", "human_hours_est", "ratio"] {
                    object.remove(key);
                }
                object.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(value).unwrap();
    strip(&mut value);
    value
}

fn parent_link(native: &str) -> SessionParentLink {
    SessionParentLink {
        session_id: id(native),
        host: "codex".into(),
        title: None,
        evidence: SubSessionEvidence::NativeSpawn,
    }
}

#[test]
fn a_parent_link_excludes_child_human_inputs_and_preserves_agent_work() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("xtrace.db");
    let old = |hours: i64| (now() - SignedDuration::from_hours(hours)).to_string();
    {
        let mut store = Store::open(&db).unwrap();
        // The parent started first and worked only before the lane window;
        // fifty sessions after it push it to the second page.
        codex(
            &mut store,
            PARENT,
            0,
            &[
                record("parent-h", &old(24 * 5), true, 0),
                record("parent-a", &old(24 * 5 - 1), false, 40),
            ],
        );
        for index in 0..50 {
            let filler = format!("019a0000-0000-7000-8000-1{index:011}");
            codex(&mut store, &filler, 10 + index, &[]);
        }
        codex(
            &mut store,
            CHILD,
            1_000,
            &[
                record("child-h", &at(0), true, 0),
                record("child-a", &at(3), false, 7),
            ],
        );
        codex(
            &mut store,
            ORPHAN,
            999,
            &[
                record("orphan-h", &at(10), true, 0),
                record("orphan-a", &at(12), false, 5),
            ],
        );
    }
    let metrics = MetricsDb::open(&db).unwrap();
    let child_search = &CHILD[CHILD.len() - 6..];
    let parent_search = &PARENT[PARENT.len() - 6..];
    let before_pages = pages(&metrics, "");
    let before_child = pages(&metrics, child_search);
    let before_parent = pages(&metrics, parent_search);
    let before_dashboard = dashboard(&metrics);
    assert!(
        before_pages
            .iter()
            .flat_map(|page| &page.rows)
            .all(|row| row.parent.is_none())
    );
    assert!(
        before_dashboard
            .lane_sessions
            .iter()
            .all(|lane| lane.parent.is_none())
    );

    let mut store = Store::open(&db).unwrap();
    assert_eq!(
        store
            .record_session_creations(&[spawn(CHILD, PARENT), spawn(ORPHAN, UNINDEXED)], 1)
            .unwrap()
            .dispositions,
        [CreationDisposition::Recorded, CreationDisposition::Recorded]
    );

    // Sessions: identical pages, cursors and measurements but for the child's
    // parent link.
    let after_pages = pages(&metrics, "");
    assert_eq!(
        without_parent_and_human(&after_pages),
        without_parent_and_human(&before_pages)
    );
    let rows: Vec<_> = after_pages.iter().flat_map(|page| &page.rows).collect();
    let find = |native: &str| *rows.iter().find(|row| row.id == id(native)).unwrap();
    assert_eq!(find(CHILD).parent, Some(parent_link(PARENT)));
    assert_eq!(
        serde_json::to_value(find(CHILD)).unwrap()["metrics"]["human_messages"],
        json!(0)
    );
    assert_eq!(
        serde_json::to_value(find(ORPHAN)).unwrap()["metrics"]["human_messages"],
        json!(0)
    );
    assert_eq!(
        serde_json::to_value(find(PARENT)).unwrap()["metrics"]["human_messages"],
        json!(1)
    );
    // The parent is on another page: linked, never moved or injected.
    assert_eq!(after_pages[0].rows[0].id, id(CHILD));
    assert!(!after_pages[0].rows.iter().any(|row| row.id == id(PARENT)));
    assert!(after_pages[1].rows.iter().any(|row| row.id == id(PARENT)));
    assert_eq!(find(PARENT).parent, None);
    // An unindexed parent is not a link.
    assert_eq!(find(ORPHAN).parent, None);
    // The detail row is the listed row, parent included.
    assert_eq!(
        &session_row(&metrics, 7, now().as_millisecond(), &id(CHILD))
            .unwrap()
            .unwrap(),
        find(CHILD)
    );

    // Search membership is unchanged in both directions.
    let child = pages(&metrics, child_search);
    assert_eq!(
        without_parent_and_human(&child),
        without_parent_and_human(&before_child)
    );
    assert_eq!(child[0].rows.len(), 1);
    assert_eq!(child[0].rows[0].parent, Some(parent_link(PARENT)));
    let parent = pages(&metrics, parent_search);
    assert_eq!(
        serde_json::to_value(&parent).unwrap(),
        serde_json::to_value(&before_parent).unwrap()
    );

    // Dashboard: every measurement, lane, span, cap and output unchanged; the
    // child's lane context links a parent that has no lane of its own.
    let after_dashboard = dashboard(&metrics);
    assert_eq!(
        after_dashboard.tiles.human_messages.value,
        before_dashboard.tiles.human_messages.value.map(|n| n - 2.0)
    );
    assert_eq!(
        without_parent_and_human(&after_dashboard),
        without_parent_and_human(&before_dashboard)
    );
    let lane = |native: &str| {
        after_dashboard
            .lane_sessions
            .iter()
            .find(|session| session.session_id == id(native))
    };
    assert_eq!(lane(CHILD).unwrap().parent, Some(parent_link(PARENT)));
    assert_eq!(lane(ORPHAN).unwrap().parent, None);
    assert!(lane(PARENT).is_none());
    assert!(
        !after_dashboard
            .lanes
            .iter()
            .any(|span| span.session_id == id(PARENT))
    );

    // A conflicting later proof withholds the link everywhere, with nothing
    // else changed.
    assert_eq!(
        store
            .record_session_creations(&[spawn(CHILD, ORPHAN)], 2)
            .unwrap()
            .dispositions,
        [CreationDisposition::Conflicted]
    );
    assert_eq!(
        without_parent_and_human(&pages(&metrics, "")),
        without_parent_and_human(&before_pages)
    );
    assert_eq!(
        without_parent_and_human(&dashboard(&metrics)),
        without_parent_and_human(&before_dashboard)
    );
    assert_eq!(
        dashboard(&metrics).tiles.human_messages.value,
        before_dashboard.tiles.human_messages.value.map(|n| n - 1.0)
    );
}

const CLAUDE_CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
const CLAUDE_OTHER: &str = "0c000000-0000-4000-8000-0000000000c2";

fn claude(store: &mut Store, native: &str, started: i64, records: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(native, "claude", SessionSource::Transcript);
    session.native_session_id = Some(native.into());
    session.started_at_ms = Some(started);
    session.surface = Some("sdk-cli".into());
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(native, records, false).unwrap();
}

fn launch(child: &str, first: &str, call: &str) -> CliArtifactCreationProof {
    CliArtifactCreationProof {
        child_session_id: child.into(),
        child_native_session_id: child.into(),
        parent_session_id: id(PARENT),
        parent_native_session_id: PARENT.into(),
        first_record_uuid: first.into(),
        launch_call_id: call.into(),
        launch_operation_index: 0,
        process_session_id: "21036".into(),
        completion_call_id: "call_completion".into(),
        output_read_call_id: "call_read".into(),
        provider_result_uuid: "0d000000-0000-4000-8000-000000000001".into(),
        evidence_version: CLI_ARTIFACT_CREATE_VERSION,
    }
}

/// A Claude session a Codex agent launched in the foreground links its exact
/// Codex parent as an agent launch, and the child keeps every measurement,
/// its human input, search membership and lane as its own.
#[test]
fn a_launched_claude_session_links_its_codex_parent_and_keeps_its_own_work() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("xtrace.db");
    {
        let mut store = Store::open(&db).unwrap();
        codex(
            &mut store,
            PARENT,
            0,
            &[
                record("parent-h", &at(0), true, 0),
                record("parent-a", &at(1), false, 40),
            ],
        );
        claude(
            &mut store,
            CLAUDE_CHILD,
            10,
            &[
                record("claude-h", &at(2), true, 0),
                record("claude-a", &at(5), false, 9),
            ],
        );
        claude(
            &mut store,
            CLAUDE_OTHER,
            20,
            &[
                record("other-h", &at(6), true, 0),
                record("other-a", &at(8), false, 3),
            ],
        );
    }
    let metrics = MetricsDb::open(&db).unwrap();
    let search = &CLAUDE_CHILD[CLAUDE_CHILD.len() - 6..];
    let before_pages = pages(&metrics, "");
    let before_search = pages(&metrics, search);
    let before_dashboard = dashboard(&metrics);

    let mut store = Store::open(&db).unwrap();
    assert_eq!(
        store
            .record_cli_artifact_creations(&[launch(CLAUDE_CHILD, "claude-h", "call_one")], 1)
            .unwrap()
            .dispositions,
        [CreationDisposition::Recorded]
    );
    let link = SessionParentLink {
        session_id: id(PARENT),
        host: "codex".into(),
        title: None,
        evidence: SubSessionEvidence::AgentLaunch,
    };
    assert_eq!(
        serde_json::to_value(&link).unwrap()["evidence"],
        json!("agent_launch")
    );

    let after_pages = pages(&metrics, "");
    assert_eq!(
        without_parent_and_human(&after_pages),
        without_parent_and_human(&before_pages)
    );
    let rows: Vec<_> = after_pages.iter().flat_map(|page| &page.rows).collect();
    let find = |session: &str| *rows.iter().find(|row| row.id == session).unwrap();
    assert_eq!(find(CLAUDE_CHILD).parent, Some(link.clone()));
    assert_eq!(
        serde_json::to_value(find(CLAUDE_CHILD)).unwrap()["metrics"]["human_messages"],
        json!(0)
    );
    assert_eq!(
        serde_json::to_value(find(CLAUDE_OTHER)).unwrap()["metrics"]["human_messages"],
        json!(1)
    );
    assert_eq!(find(CLAUDE_OTHER).parent, None);
    assert_eq!(find(&id(PARENT)).parent, None);
    assert_eq!(
        &session_row(&metrics, 7, now().as_millisecond(), CLAUDE_CHILD)
            .unwrap()
            .unwrap(),
        find(CLAUDE_CHILD)
    );
    let found = pages(&metrics, search);
    assert_eq!(
        without_parent_and_human(&found),
        without_parent_and_human(&before_search)
    );
    assert_eq!(found[0].rows.len(), 1);

    let after_dashboard = dashboard(&metrics);
    assert_eq!(
        after_dashboard.tiles.human_messages.value,
        before_dashboard.tiles.human_messages.value.map(|n| n - 1.0)
    );
    assert_eq!(
        without_parent_and_human(&after_dashboard),
        without_parent_and_human(&before_dashboard)
    );
    let lane = after_dashboard
        .lane_sessions
        .iter()
        .find(|session| session.session_id == CLAUDE_CHILD)
        .unwrap();
    assert_eq!(lane.parent, Some(link));

    // The same launch claimed for another session withholds both, and every
    // report is again exactly what it was before any relation.
    assert_eq!(
        store
            .record_cli_artifact_creations(&[launch(CLAUDE_OTHER, "other-h", "call_one")], 2)
            .unwrap()
            .dispositions,
        [CreationDisposition::Conflicted]
    );
    assert_eq!(
        serde_json::to_value(pages(&metrics, "")).unwrap(),
        serde_json::to_value(&before_pages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(dashboard(&metrics)).unwrap(),
        serde_json::to_value(&before_dashboard).unwrap()
    );
}
