//! A verified sub-session's parent, as the Sessions page and the Dashboard
//! lane context carry it: an exact, indexed parent identity beside rows the
//! reads already return, and nothing else. Membership, order, cursors,
//! search, every measurement, span and output are byte-for-byte what they
//! were before the relation existed, and a parent that is not indexed is
//! never a link.
use jiff::{SignedDuration, Timestamp, tz::TimeZone};
use serde_json::{Value, json};
use xt_metrics::{BreakLength, MetricsDb, TypingRate};
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
        DashboardMetrics, MetricClock, SessionChildCheck, SessionPage, SessionParentContext,
        SessionParentLink, SessionQuery, SubSessionEvidence, session_page, session_row,
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
        BreakLength::default(),
    )
    .unwrap()
}

/// Compare every agent/work field; assert the changed parent, known-child,
/// referenced-parent and Human fields separately.
fn without_parent_and_human(value: &impl serde::Serialize) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for key in [
                    "parent",
                    "known_child",
                    "child_check",
                    "referenced_parents",
                    "human_messages",
                    "human_hours_est",
                    // "Your hours" and the leverage over them read the same
                    // Human classification the parent link changes.
                    "human_hours",
                    "leverage",
                ] {
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

/// The report with only the returned sessions' context: a parent the lanes
/// do not name gets a context-only entry, asserted separately.
fn returned_only(report: &DashboardMetrics) -> DashboardMetrics {
    let mut report = report.clone();
    let returned: std::collections::BTreeSet<String> = report
        .lanes
        .iter()
        .map(|span| span.session_id.clone())
        .collect();
    report
        .lane_sessions
        .retain(|session| returned.contains(&session.session_id));
    report
}

/// A page's context-only entry for a parent it does not list.
fn parent_context(
    native: &str,
    known_child: bool,
    parent: Option<SessionParentLink>,
) -> SessionParentContext {
    // No display check is run here: a session that is no known child is
    // still checking.
    SessionParentContext {
        session_id: id(native),
        host: "codex".into(),
        known_child,
        parent,
        child_check: Some(if known_child {
            SessionChildCheck::Child
        } else {
            SessionChildCheck::Checking
        }),
    }
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
            .all(|row| row.parent.is_none() && row.known_child == Some(false))
    );
    assert!(
        before_pages
            .iter()
            .all(|page| page.referenced_parents.is_empty())
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
    // The parent is on another page: linked, never moved or injected. The
    // child's page carries context only for it, and the parent's own page,
    // which lists it, carries none.
    assert_eq!(after_pages[0].rows[0].id, id(CHILD));
    assert!(!after_pages[0].rows.iter().any(|row| row.id == id(PARENT)));
    assert!(after_pages[1].rows.iter().any(|row| row.id == id(PARENT)));
    assert_eq!(find(PARENT).parent, None);
    assert_eq!(
        after_pages[0].referenced_parents,
        [parent_context(PARENT, false, None)]
    );
    assert!(after_pages[1].referenced_parents.is_empty());
    // Each row's own stored bit, read in the page's snapshot.
    assert_eq!(find(CHILD).known_child, Some(true));
    assert_eq!(find(PARENT).known_child, Some(false));
    // An unindexed parent is not a link, and gains no context entry; the
    // known child keeps its bit.
    assert_eq!(find(ORPHAN).parent, None);
    assert_eq!(find(ORPHAN).known_child, Some(true));
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
    // A filtered-out parent: context only, never a row.
    assert_eq!(
        child[0].referenced_parents,
        [parent_context(PARENT, false, None)]
    );
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
        without_parent_and_human(&returned_only(&after_dashboard)),
        without_parent_and_human(&before_dashboard)
    );
    let lane = |native: &str| {
        after_dashboard
            .lane_sessions
            .iter()
            .find(|session| session.session_id == id(native))
    };
    assert_eq!(lane(CHILD).unwrap().parent, Some(parent_link(PARENT)));
    assert_eq!(lane(CHILD).unwrap().known_child, Some(true));
    // A known child whose parent is not indexed: the bit without a parent.
    assert_eq!(lane(ORPHAN).unwrap().parent, None);
    assert_eq!(lane(ORPHAN).unwrap().known_child, Some(true));
    // The parent has no span: it gets context only, with nothing measured,
    // and still no lane.
    let parent = lane(PARENT).unwrap();
    assert_eq!(
        (
            parent.cost.clone(),
            parent.parent.clone(),
            parent.known_child
        ),
        (None, None, Some(false))
    );
    assert!(
        !after_dashboard
            .lanes
            .iter()
            .any(|span| span.session_id == id(PARENT))
    );
    assert!(
        before_dashboard
            .lane_sessions
            .iter()
            .all(|session| session.known_child == Some(false))
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
    assert_eq!(found[0].rows[0].known_child, Some(true));
    assert_eq!(
        found[0].referenced_parents,
        [parent_context(PARENT, false, None)]
    );

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
    assert_eq!(lane.known_child, Some(true));

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

const GRANDPARENT: &str = "019a0000-0000-7000-8000-0000000000a1";
const MIDDLE: &str = "019a0000-0000-7000-8000-0000000000a2";
const LEAF: &str = "019a0000-0000-7000-8000-0000000000a3";

/// A returned session's direct parent with no span gets one context-only
/// entry, read in the same snapshot: the bit and the parent it resolved, and
/// nothing measured. That entry's own parent is not followed, and every
/// lane, span, measurement and the returned sessions' own context is what it
/// was before the relations existed.
#[test]
fn an_unreturned_direct_parent_gets_context_only_and_no_further_ancestor() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("xtrace.db");
    let old = |hours: i64| (now() - SignedDuration::from_hours(hours)).to_string();
    {
        let mut store = Store::open(&db).unwrap();
        // Two parents with work only before the lane window, and an indexed
        // grandparent with none in it either.
        codex(
            &mut store,
            GRANDPARENT,
            0,
            &[record("grand-h", &old(24 * 5), true, 0)],
        );
        codex(
            &mut store,
            PARENT,
            1,
            &[record("parent-h", &old(24 * 5), true, 0)],
        );
        codex(
            &mut store,
            MIDDLE,
            2,
            &[record("middle-h", &old(24 * 5), true, 0)],
        );
        for (native, start, minutes) in [(CHILD, 10, 0), (LEAF, 11, 20)] {
            codex(
                &mut store,
                native,
                start,
                &[
                    record(&format!("{native}-h"), &at(minutes), true, 0),
                    record(&format!("{native}-a"), &at(minutes + 3), false, 7),
                ],
            );
        }
    }
    let metrics = MetricsDb::open(&db).unwrap();
    let before = dashboard(&metrics);
    let mut store = Store::open(&db).unwrap();
    store
        .record_session_creations(
            &[
                // PARENT is a known child whose own parent is not indexed.
                spawn(PARENT, UNINDEXED),
                spawn(CHILD, PARENT),
                // MIDDLE is a known child of an indexed grandparent.
                spawn(MIDDLE, GRANDPARENT),
                spawn(LEAF, MIDDLE),
            ],
            1,
        )
        .unwrap();
    let after = dashboard(&metrics);
    assert_eq!(after.lanes, before.lanes);
    assert_eq!(
        without_parent_and_human(&returned_only(&after)),
        without_parent_and_human(&before)
    );
    let entry = |native: &str| {
        after
            .lane_sessions
            .iter()
            .find(|session| session.session_id == id(native))
    };
    // The returned sessions and their two unreturned direct parents, in
    // identifier order; never the grandparent.
    let mut expected = vec![id(PARENT), id(MIDDLE), id(LEAF), id(CHILD)];
    expected.sort();
    let ids: Vec<String> = after
        .lane_sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect();
    assert_eq!(ids, expected);
    // The returned children: known, each with its verified parent.
    assert_eq!(entry(CHILD).unwrap().parent, Some(parent_link(PARENT)));
    assert_eq!(entry(LEAF).unwrap().parent, Some(parent_link(MIDDLE)));
    // An unreturned parent that is a known child with no verified parent.
    let parent = entry(PARENT).unwrap();
    assert_eq!(
        (
            parent.known_child,
            parent.parent.clone(),
            parent.cost.clone()
        ),
        (Some(true), None, None)
    );
    // An unreturned parent with its own verified parent, which is not read.
    let middle = entry(MIDDLE).unwrap();
    assert_eq!(
        (
            middle.known_child,
            middle.parent.clone(),
            middle.cost.clone()
        ),
        (Some(true), Some(parent_link(GRANDPARENT)), None)
    );
    assert!(entry(GRANDPARENT).is_none());
    assert!(
        !after
            .lanes
            .iter()
            .any(|span| [id(PARENT), id(MIDDLE), id(GRANDPARENT)].contains(&span.session_id))
    );

    // Sessions reads the same context for a parent its filtered page does not
    // list: the direct parent's own bit and resolved parent, never further.
    let child = pages(&metrics, &CHILD[CHILD.len() - 6..]);
    assert_eq!(child[0].rows.len(), 1);
    assert_eq!(child[0].rows[0].known_child, Some(true));
    assert_eq!(
        child[0].referenced_parents,
        [parent_context(PARENT, true, None)]
    );
    let leaf = pages(&metrics, &LEAF[LEAF.len() - 6..]);
    assert_eq!(leaf[0].rows.len(), 1);
    assert_eq!(
        leaf[0].referenced_parents,
        [parent_context(MIDDLE, true, Some(parent_link(GRANDPARENT)))]
    );
    // Listed together, every parent is a row and none is context.
    let every = pages(&metrics, "");
    assert_eq!(every.len(), 1);
    assert!(every[0].referenced_parents.is_empty());
    let known: Vec<(String, Option<bool>)> = every[0]
        .rows
        .iter()
        .map(|row| (row.id.clone(), row.known_child))
        .collect();
    for (native, bit) in [
        (GRANDPARENT, false),
        (PARENT, true),
        (MIDDLE, true),
        (CHILD, true),
        (LEAF, true),
    ] {
        assert!(known.contains(&(id(native), Some(bit))), "{native}");
    }
}

// The grandchild's identifier sorts before its parent's, against the walk's
// level order, so an identifier-ordered read would put it first.
const OLD_CHILD: &str = "019a0000-0000-7000-8000-0000000000f9";
const OLD_GRANDCHILD: &str = "019a0000-0000-7000-8000-000000000001";

#[test]
fn a_listed_parent_carries_its_older_sub_sessions_with_their_whole_cost() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("xtrace.db");
    let old = |hours: i64| (now() - SignedDuration::from_hours(hours)).to_string();
    {
        let mut store = Store::open(&db).unwrap();
        // The parent and one child are active in the lane window.
        for (native, start, minutes) in [(PARENT, 1, 0), (CHILD, 2, 30)] {
            codex(
                &mut store,
                native,
                start,
                &[
                    record(&format!("{native}-h"), &at(minutes), true, 0),
                    record(&format!("{native}-a"), &at(minutes + 3), false, 7),
                ],
            );
        }
        // A child and its own child that ran only days before the window.
        for (native, start) in [(OLD_CHILD, 3), (OLD_GRANDCHILD, 4)] {
            codex(
                &mut store,
                native,
                start,
                &[
                    record(&format!("{native}-h"), &old(24 * 5), true, 0),
                    record(&format!("{native}-a"), &old(24 * 5 - 1), false, 5),
                ],
            );
        }
        // An old session linked to a parent that is not indexed is no one's.
        codex(
            &mut store,
            ORPHAN,
            5,
            &[record("orphan-a", &old(24 * 5), false, 5)],
        );
        store
            .record_session_creations(
                &[
                    spawn(CHILD, PARENT),
                    spawn(OLD_CHILD, PARENT),
                    spawn(OLD_GRANDCHILD, OLD_CHILD),
                    spawn(ORPHAN, UNINDEXED),
                ],
                1,
            )
            .unwrap();
    }
    let metrics = MetricsDb::open(&db).unwrap();
    let report = dashboard(&metrics);
    // The lanes are still only the sessions active in the window.
    let lanes: std::collections::BTreeSet<&str> = report
        .lanes
        .iter()
        .map(|span| span.session_id.as_str())
        .collect();
    assert_eq!(
        lanes,
        [id(PARENT), id(CHILD)].iter().map(String::as_str).collect()
    );
    // Nearer levels first — the old child, then its own child, though its
    // identifier sorts first — each with its verified parent and its whole
    // cost; the orphan is not carried.
    let older: Vec<_> = report
        .lane_sub_sessions
        .iter()
        .map(|s| {
            (
                s.session_id.clone(),
                s.parent.as_ref().map(|p| p.session_id.clone()),
                s.cost.as_ref().map(|c| c.selected_observations),
            )
        })
        .collect();
    assert_eq!(
        older,
        [
            (id(OLD_CHILD), Some(id(PARENT)), Some(1)),
            (id(OLD_GRANDCHILD), Some(id(OLD_CHILD)), Some(1)),
        ]
    );
    // Nothing was left out, so no listed session counts any.
    assert!(
        report
            .lane_sessions
            .iter()
            .all(|s| s.sub_sessions_not_shown.is_none())
    );
}
