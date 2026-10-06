//! Sub-session relations: one guarded, content-free fact per created child,
//! applied by exact identity, never replaced by a disagreeing proof, and read
//! beside a page or a context set without changing which sessions they hold.

use rusqlite::Connection;
use tempfile::TempDir;
use xt_store::{
    Host, SessionMeta, SessionSource, Store,
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationAbstention, CreationBootstrap, CreationDisposition,
        CreationEvidence, CreationWitness, LOCATOR_TAIL_QUERY, MAX_CREATION_DEPTH,
        MAX_CREATION_PROOFS, SessionCreationProof,
    },
    session_list::{self, SessionFilter},
};

const PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const CHILD: &str = "019a0000-0000-7000-8000-0000000000bb";
const OTHER: &str = "019a0000-0000-7000-8000-0000000000cc";
const UNINDEXED: &str = "019a0000-0000-7000-8000-0000000000dd";
const T: i64 = 1_788_782_400_000;

fn id(native: &str) -> String {
    format!("codex-{native}")
}

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("creation.sqlite")).unwrap()
}

fn sql(directory: &TempDir) -> Connection {
    Connection::open(directory.path().join("creation.sqlite")).unwrap()
}

fn session(store: &mut Store, native: &str, started: i64, title: Option<&str>) {
    let mut meta = SessionMeta::new(id(native), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    meta.started_at_ms = Some(started);
    meta.title = title.map(str::to_owned);
    meta.cwd = Some(format!("/repo/{native}"));
    // A saved title is retained only under a content-keeping write.
    store.upsert_session(&meta, title.is_some()).unwrap();
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

fn record(store: &mut Store, proofs: &[SessionCreationProof]) -> Vec<CreationDisposition> {
    store
        .record_session_creations(proofs, T)
        .unwrap()
        .dispositions
}

fn abstained(reason: CreationAbstention) -> CreationDisposition {
    CreationDisposition::Abstained(reason)
}

/// `(child, parent)` for every listed row that shows a parent.
fn shown(store: &Store) -> Vec<(String, String)> {
    store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap()
        .into_iter()
        .filter_map(|row| Some((row.id, row.parent?.session_id)))
        .collect()
}

fn reviewer_origin(directory: &TempDir, child: &str, parent: &str) {
    sql(directory)
        .execute(
            "INSERT INTO human_session_origins(session_id,host,native_session_id,parent_host,
         parent_native_session_id,method,evidence_id,launch_id,rule_version)
         VALUES (?1,'codex',?2,'codex',?3,'native_reviewer_header',
         'rollout_opening_session_meta','native_reviewer_header',1)",
            (id(child), child, parent),
        )
        .unwrap();
}

#[test]
fn reviewer_header_labels_and_links_only_an_exact_unconflicted_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, Some("Parent saved title"));
    session(&mut store, CHILD, 2, None);
    reviewer_origin(&directory, CHILD, PARENT);
    let child = store
        .sessions_page("", None, None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == id(CHILD))
        .unwrap();
    assert!(child.automated_review);
    assert_eq!(child.title, None);
    assert_eq!(child.parent.as_ref().unwrap().session_id, id(PARENT));
    assert_eq!(
        child.parent.as_ref().unwrap().evidence,
        session_list::ParentEvidence::NativeReviewer
    );
    let context = session_list::context(&sql(&directory), &[&id(CHILD)]).unwrap();
    assert!(context[0].automated_review);
    assert_eq!(context[0].parent, child.parent);

    sql(&directory)
        .execute(
            "UPDATE human_session_origins SET conflicted=1,
        conflict_reason='evidence_mismatch' WHERE session_id=?1",
            [id(CHILD)],
        )
        .unwrap();
    let child = store
        .sessions_page("", None, None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == id(CHILD))
        .unwrap();
    assert!(!child.automated_review);
    assert!(child.parent.is_none());
}

#[test]
fn reviewer_header_rejects_missing_ambiguous_and_wrong_evidence() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, CHILD, 2, None);
    reviewer_origin(&directory, CHILD, PARENT);
    let read = |store: &Store| {
        store
            .sessions_page("", None, None)
            .unwrap()
            .into_iter()
            .find(|row| row.id == id(CHILD))
            .unwrap()
    };
    assert!(!read(&store).automated_review);
    session(&mut store, PARENT, 1, None);
    assert!(read(&store).automated_review);
    let mut duplicate =
        SessionMeta::new("codex-duplicate-parent", "codex", SessionSource::ReadersCli);
    duplicate.native_session_id = Some(PARENT.into());
    store.upsert_session(&duplicate, false).unwrap();
    assert!(!read(&store).automated_review);
    sql(&directory)
        .execute(
            "DELETE FROM sessions WHERE session_id='codex-duplicate-parent'",
            [],
        )
        .unwrap();
    for (column, value) in [
        ("method", "exact_initial_text"),
        ("evidence_id", "other_witness"),
        ("launch_id", "other_launch"),
    ] {
        let query = format!("UPDATE human_session_origins SET {column}=?1 WHERE session_id=?2");
        sql(&directory).execute(&query, (value, id(CHILD))).unwrap();
        assert!(!read(&store).automated_review);
        let restore = match column {
            "method" | "launch_id" => "native_reviewer_header",
            _ => "rollout_opening_session_meta",
        };
        sql(&directory)
            .execute(&query, (restore, id(CHILD)))
            .unwrap();
    }
    sql(&directory)
        .execute(
            "UPDATE human_session_origins SET native_session_id=?1
        WHERE session_id=?2",
            (OTHER, id(CHILD)),
        )
        .unwrap();
    assert!(!read(&store).automated_review);
}

#[test]
fn accepted_creation_wins_and_conflicted_creation_withholds_reviewer_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    session(&mut store, OTHER, 1, None);
    session(&mut store, CHILD, 2, None);
    reviewer_origin(&directory, CHILD, PARENT);
    record(&mut store, &[spawn(CHILD, OTHER)]);
    let child = store
        .sessions_page("", None, None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == id(CHILD))
        .unwrap();
    assert_eq!(child.parent.unwrap().session_id, id(OTHER));
    sql(&directory)
        .execute(
            "UPDATE session_creation_relations SET state='conflicted'
        WHERE child_session_id=?1",
            [id(CHILD)],
        )
        .unwrap();
    let child = store
        .sessions_page("", None, None)
        .unwrap()
        .into_iter()
        .find(|row| row.id == id(CHILD))
        .unwrap();
    assert!(child.parent.is_none());
    assert!(!child.automated_review);
}

#[test]
fn an_exact_spawn_shows_its_indexed_parent_and_replays_idempotently() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    store
        .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
        .unwrap();
    session(&mut store, PARENT, 1, Some("Parent saved title"));
    session(&mut store, CHILD, 2, None);
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [CreationDisposition::Recorded]
    );
    let rows = store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap();
    let child = rows.iter().find(|row| row.id == id(CHILD)).unwrap();
    let parent = child.parent.as_ref().unwrap();
    assert_eq!(parent.session_id, id(PARENT));
    assert_eq!(parent.host, "codex");
    assert_eq!(parent.title.as_deref(), Some("Parent saved title"));
    assert_eq!(parent.evidence, CreationEvidence::CodexThreadSpawn.into());
    assert!(
        rows.iter()
            .find(|row| row.id == id(PARENT))
            .unwrap()
            .parent
            .is_none()
    );

    // Replay, in one batch and across a reopen, changes nothing.
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT), spawn(CHILD, PARENT)]),
        [
            CreationDisposition::AlreadyRecorded,
            CreationDisposition::AlreadyRecorded
        ]
    );
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [CreationDisposition::AlreadyRecorded]
    );
    assert_eq!(
        store.session_creation(&id(CHILD)).unwrap(),
        Some((spawn(CHILD, PARENT), false))
    );
    assert_eq!(shown(&store), [(id(CHILD), id(PARENT))]);
}

#[test]
fn an_unindexed_or_ambiguous_parent_is_kept_but_never_shown() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, CHILD, 2, None);
    assert_eq!(
        record(&mut store, &[spawn(CHILD, UNINDEXED)]),
        [CreationDisposition::Recorded]
    );
    assert!(shown(&store).is_empty(), "no parent is indexed yet");

    // Indexed later, the parent appears without any replay.
    session(&mut store, UNINDEXED, 1, None);
    assert_eq!(shown(&store), [(id(CHILD), id(UNINDEXED))]);

    // A second user session holding the parent's native identity makes the
    // parent ambiguous: nothing is shown rather than either.
    let mut twin = SessionMeta::new("plugin-twin", "codex", SessionSource::Plugin);
    twin.native_session_id = Some(UNINDEXED.into());
    store.upsert_session(&twin, false).unwrap();
    assert!(shown(&store).is_empty());
    // A judge session holding it is not a user session and does not count.
    sql(&directory)
        .execute(
            "UPDATE sessions SET kind='judge' WHERE session_id='plugin-twin'",
            [],
        )
        .unwrap();
    assert_eq!(shown(&store), [(id(CHILD), id(UNINDEXED))]);
}

#[test]
fn a_child_must_be_exactly_one_indexed_user_session() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [abstained(CreationAbstention::MissingChild)]
    );
    session(&mut store, CHILD, 2, None);
    // The canonical session exists, but the proof names another native child.
    let mut wrong = spawn(CHILD, PARENT);
    wrong.child_native_session_id = OTHER.into();
    // The child named as its own parent.
    let own = spawn(CHILD, CHILD);
    let mut upper = spawn(CHILD, PARENT);
    upper.parent_native_session_id = CHILD.to_uppercase();
    for (proof, reason) in [
        (wrong, CreationAbstention::ChildIdentityMismatch),
        (own, CreationAbstention::SelfLink),
        (upper, CreationAbstention::SelfLink),
    ] {
        assert_eq!(record(&mut store, &[proof]), [abstained(reason)]);
    }
    // Two user sessions hold the child's native identity.
    let mut twin = SessionMeta::new("plugin-child", "codex", SessionSource::Plugin);
    twin.native_session_id = Some(CHILD.into());
    store.upsert_session(&twin, false).unwrap();
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [abstained(CreationAbstention::AmbiguousChild)]
    );
    sql(&directory)
        .execute(
            "UPDATE sessions SET kind='judge' WHERE session_id IN ('plugin-child',?1)",
            [id(CHILD)],
        )
        .unwrap();
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [abstained(CreationAbstention::NotUserSession)]
    );
    assert!(store.session_creation(&id(CHILD)).unwrap().is_none());
}

#[test]
fn a_disagreeing_parent_is_durably_withheld_and_never_replaces_the_first() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for (native, start) in [(PARENT, 1), (OTHER, 2), (CHILD, 3)] {
        session(&mut store, native, start, None);
    }
    // In one batch: neither applies.
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT), spawn(CHILD, OTHER)]),
        [
            abstained(CreationAbstention::AmbiguousInBatch),
            abstained(CreationAbstention::AmbiguousInBatch)
        ]
    );
    assert!(store.session_creation(&id(CHILD)).unwrap().is_none());

    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [CreationDisposition::Recorded]
    );
    assert_eq!(
        record(&mut store, &[spawn(CHILD, OTHER)]),
        [CreationDisposition::Conflicted]
    );
    assert!(shown(&store).is_empty());
    // The first proof stays; only its state changed, and the original proof
    // replayed is still withheld.
    assert_eq!(
        store.session_creation(&id(CHILD)).unwrap(),
        Some((spawn(CHILD, PARENT), true))
    );
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [CreationDisposition::Conflicted]
    );
    assert!(shown(&store).is_empty());
}

/// Only a proof that changes what is stored counts as a change: a new
/// relation, or an accepted one first withheld. A replay, an abstention, a
/// relation already conflicted and a batch ambiguity count nothing, so a
/// caller publishes a change only when a read would show one.
#[test]
fn only_a_new_relation_or_a_first_conflict_counts_as_a_change() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for (native, start) in [(PARENT, 1), (OTHER, 2), (CHILD, 3)] {
        session(&mut store, native, start, None);
    }
    let changed = |store: &mut Store, proofs: &[SessionCreationProof]| {
        let report = store.record_session_creations(proofs, T).unwrap();
        (report.dispositions, report.changed)
    };
    assert_eq!(
        changed(&mut store, &[spawn(CHILD, PARENT), spawn(CHILD, OTHER)]).1,
        0
    );
    assert_eq!(
        changed(&mut store, &[spawn(CHILD, CHILD)]),
        (vec![abstained(CreationAbstention::SelfLink)], 0)
    );
    assert_eq!(
        changed(&mut store, &[spawn(CHILD, PARENT)]),
        (vec![CreationDisposition::Recorded], 1)
    );
    assert_eq!(
        changed(&mut store, &[spawn(CHILD, PARENT)]),
        (vec![CreationDisposition::AlreadyRecorded], 0)
    );
    assert_eq!(
        changed(&mut store, &[spawn(CHILD, OTHER)]),
        (vec![CreationDisposition::Conflicted], 1)
    );
    for proof in [spawn(CHILD, OTHER), spawn(CHILD, PARENT)] {
        assert_eq!(
            changed(&mut store, &[proof]),
            (vec![CreationDisposition::Conflicted], 0)
        );
    }
    drop(store);
    let mut store = open(&directory);
    assert_eq!(
        changed(&mut store, &[spawn(CHILD, PARENT)]),
        (vec![CreationDisposition::Conflicted], 0)
    );
}

#[test]
fn a_cycle_or_an_overlong_chain_is_refused_even_through_an_unindexed_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    session(&mut store, CHILD, 2, None);
    // PARENT was spawned by UNINDEXED before UNINDEXED was indexed, and CHILD
    // by PARENT. Once indexed, UNINDEXED claims CHILD as its parent: the
    // stored edges are followed by native identity and close the cycle.
    let mut unindexed_child = spawn(UNINDEXED, CHILD);
    unindexed_child.child_session_id = id(UNINDEXED);
    assert_eq!(
        record(&mut store, &[spawn(PARENT, UNINDEXED)]),
        [CreationDisposition::Recorded]
    );
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT)]),
        [CreationDisposition::Recorded]
    );
    session(&mut store, UNINDEXED, 0, None);
    assert_eq!(
        record(&mut store, &[unindexed_child]),
        [abstained(CreationAbstention::Cycle)]
    );
    // A direct two-session cycle.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    session(&mut store, CHILD, 2, None);
    assert_eq!(
        record(&mut store, &[spawn(CHILD, PARENT), spawn(PARENT, CHILD)]),
        [
            CreationDisposition::Recorded,
            abstained(CreationAbstention::Cycle)
        ]
    );
    assert_eq!(shown(&store), [(id(CHILD), id(PARENT))]);

    // A chain longer than a check follows is not known to be acyclic.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    let natives: Vec<String> = (0..=MAX_CREATION_DEPTH + 1)
        .map(|index| format!("019a0000-0000-7000-8000-{index:012}"))
        .collect();
    for (index, native) in natives.iter().enumerate() {
        session(&mut store, native, index as i64, None);
    }
    let chain: Vec<SessionCreationProof> = natives
        .windows(2)
        .take(MAX_CREATION_DEPTH)
        .map(|pair| spawn(&pair[1], &pair[0]))
        .collect();
    assert!(
        record(&mut store, &chain)
            .iter()
            .all(|disposition| *disposition == CreationDisposition::Recorded)
    );
    let last = &natives[MAX_CREATION_DEPTH + 1];
    assert_eq!(
        record(&mut store, &[spawn(last, &natives[MAX_CREATION_DEPTH])]),
        [abstained(CreationAbstention::Cycle)]
    );
}

#[test]
fn malformed_or_misfitting_proofs_reject_the_whole_call() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    session(&mut store, CHILD, 2, None);
    let mut blank = spawn(CHILD, PARENT);
    blank.parent_native_session_id = " ".into();
    let mut control = spawn(CHILD, PARENT);
    control.parent_native_session_id = format!("{PARENT}\n");
    let mut long = spawn(CHILD, PARENT);
    long.parent_native_session_id = "a".repeat(257);
    let mut version = spawn(CHILD, PARENT);
    version.evidence_version = 0;
    let mut claude = spawn(CHILD, PARENT);
    claude.parent_host = Host::Claude;
    for bad in [blank, control, long, version, claude] {
        assert!(
            store
                .record_session_creations(&[spawn(CHILD, PARENT), bad], T)
                .is_err()
        );
    }
    let many = vec![spawn(CHILD, PARENT); MAX_CREATION_PROOFS + 1];
    assert!(store.record_session_creations(&many, T).is_err());
    assert!(store.session_creation(&id(CHILD)).unwrap().is_none());
}

#[test]
fn relations_are_immutable_and_hold_no_content() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    session(&mut store, CHILD, 2, None);
    record(&mut store, &[spawn(CHILD, PARENT)]);
    let connection = sql(&directory);
    for statement in [
        "UPDATE session_creation_relations SET parent_native_session_id='x'",
        "UPDATE session_creation_relations SET state='conflicted',evidence_version=evidence_version+1",
        "DELETE FROM session_creation_relations",
    ] {
        assert!(connection.execute(statement, []).is_err(), "{statement}");
    }
    connection
        .execute(
            "UPDATE session_creation_relations SET state='conflicted'",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute("UPDATE session_creation_relations SET state='accepted'", [])
            .is_err()
    );
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('session_creation_relations') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "child_session_id",
            "child_host",
            "child_native_session_id",
            "parent_host",
            "parent_native_session_id",
            "evidence_kind",
            "evidence_version",
            "witness",
            "state",
            "recorded_at",
            "parent_session_id",
            "first_record_uuid",
            "launch_call_id",
            "launch_operation_index",
            "process_session_id",
            "completion_call_id",
            "output_read_call_id",
            "provider_result_uuid",
            "completion_operation_index",
            "segment_rollout_id",
            "launch_ordinal",
            "completion_ordinal"
        ]
    );
    // The native spawn kind cannot name another host or witness.
    assert!(
        connection
            .execute(
                "INSERT INTO session_creation_relations(child_session_id,child_host,
                     child_native_session_id,parent_host,parent_native_session_id,
                     evidence_kind,evidence_version,witness,state,recorded_at)
                 VALUES (?1,'codex',?2,'claude','p','codex_thread_spawn',1,
                  'rollout_opening_session_meta','accepted',0)",
                [id(OTHER), OTHER.to_owned()],
            )
            .is_err()
    );
}

/// The page and a context read carry a parent only for rows they already
/// hold: search, host filters, the page bound and the cursor decide the rows
/// exactly as before, and a parent filtered out or on another page is never
/// added.
#[test]
fn a_parent_never_changes_which_rows_a_page_or_context_holds() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    // Sixty sessions: the child starts last so it opens the first page; its
    // parent starts first so it falls on the second.
    session(&mut store, PARENT, 0, None);
    for index in 1..59 {
        session(
            &mut store,
            &format!("019a0000-0000-7000-8000-1{index:011}"),
            index,
            None,
        );
    }
    session(&mut store, CHILD, 100, None);
    let before: Vec<Vec<String>> = pages(&store, "");
    record(&mut store, &[spawn(CHILD, PARENT)]);
    assert_eq!(pages(&store, ""), before, "membership, order and cursor");
    let first = store
        .sessions_page_filtered(&SessionFilter::default(), None)
        .unwrap();
    assert_eq!(first[0].id, id(CHILD));
    assert_eq!(first[0].parent.as_ref().unwrap().session_id, id(PARENT));
    assert!(!first.iter().take(50).any(|row| row.id == id(PARENT)));

    // A search matching only the child keeps it alone, with its parent link.
    let found = store
        .sessions_page_filtered(
            &SessionFilter {
                search: &CHILD[CHILD.len() - 6..],
                ..SessionFilter::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(
        found.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
        [id(CHILD)]
    );
    assert_eq!(found[0].parent.as_ref().unwrap().session_id, id(PARENT));
    // A search matching only the parent never brings in the child.
    let found = store
        .sessions_page_filtered(
            &SessionFilter {
                search: &PARENT[PARENT.len() - 6..],
                ..SessionFilter::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(
        found.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
        [id(PARENT)]
    );

    // The context read beside Dashboard lanes: exactly the named sessions.
    let connection = sql(&directory);
    let context = session_list::context(&connection, &[&id(CHILD)]).unwrap();
    assert_eq!(context.len(), 1);
    assert_eq!(
        context[0].parent.as_ref().unwrap().session_id,
        id(PARENT),
        "a parent outside the named set is linked, not added"
    );
    let parents = session_list::parents(&connection, &[&id(CHILD), &id(PARENT)]).unwrap();
    assert_eq!(parents.len(), 1);
    let too_many: Vec<String> = (0..=session_list::MAX_CONTEXT)
        .map(|index| format!("s{index}"))
        .collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert!(session_list::parents(&connection, &too_many).is_err());
}

/// One pull request's linked sessions are chosen before the page bound, and a
/// member that is a sub-session still names its parent, which is linked to
/// another pull request and so is never added to the member pages.
#[test]
fn exact_pull_request_members_keep_their_parent_without_adding_it() {
    use xt_store::pr_link::{PrConfidence, PrIdentity, PrLinkObservation};
    use xt_store::session_list::PrMembership;
    let link = |store: &mut Store, native: &str, number: u64, confidence| {
        store
            .record_pr_link(&PrLinkObservation {
                session_id: id(native),
                pull_request: PrIdentity::from_parts("example/atlas", number).unwrap(),
                confidence,
                first_seen_at: 1,
                last_seen_at: 1,
            })
            .unwrap();
    };
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    // The parent opens the unfiltered list and links only to #8; 57 members of
    // #7 and 27 non-members interleave; the child is the last member.
    session(&mut store, PARENT, 1_000, None);
    link(&mut store, PARENT, 8, PrConfidence::Exact);
    for index in 1..=84_u32 {
        let native = format!("019a0000-0000-7000-8000-2{index:011}");
        session(&mut store, &native, i64::from(index), None);
        if index % 3 != 0 || index > 81 {
            link(&mut store, &native, 7, PrConfidence::Exact);
        } else {
            link(&mut store, &native, 8, PrConfidence::Inferred);
        }
    }
    session(&mut store, CHILD, 0, None);
    link(&mut store, CHILD, 7, PrConfidence::Sha);
    record(&mut store, &[spawn(CHILD, PARENT)]);

    let identity = PrIdentity::from_parts("Example/Atlas", 7).unwrap();
    for confirmed_only in [false, true] {
        let filter = SessionFilter {
            pull_request: Some(PrMembership {
                identity: &identity,
                confirmed_only,
            }),
            ..SessionFilter::default()
        };
        let mut first = store.sessions_page_filtered(&filter, None).unwrap();
        assert_eq!(first.len(), 51, "a 51st member marks a next page");
        first.truncate(50);
        let rest = store
            .sessions_page_filtered(&filter, Some(&first[49].cursor))
            .unwrap();
        let rows: Vec<_> = first.iter().chain(&rest).collect();
        assert_eq!(rows.len(), 58);
        assert!(!rows.iter().any(|row| row.id == id(PARENT)));
        let child = rows.last().unwrap();
        assert_eq!(child.id, id(CHILD));
        assert_eq!(
            child.parent.as_ref().unwrap().session_id,
            id(PARENT),
            "the parent is named, not added"
        );
        assert!(rows[..57].iter().all(|row| row.parent.is_none()));
    }
}

/// Every page's row identities and its cursor, following the cursor to the
/// end.
fn pages(store: &Store, search: &str) -> Vec<Vec<String>> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let mut rows = store
            .sessions_page_filtered(
                &SessionFilter {
                    search,
                    ..SessionFilter::default()
                },
                after.as_ref(),
            )
            .unwrap();
        let more = rows.len() > 50;
        rows.truncate(50);
        after = rows.last().map(|row| row.cursor.clone());
        all.push(rows.into_iter().map(|row| row.id).collect());
        if !more {
            return all;
        }
    }
}

#[test]
fn bootstrap_progress_and_locator_pages_only_move_forward() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    let kind = CreationEvidence::CodexThreadSpawn;
    assert_eq!(
        store.session_creation_bootstrap(kind, 1).unwrap(),
        CreationBootstrap::default()
    );
    for key in ["codex:/b", "codex:/a", "codex:/c", "cursor:/a", "codex;x"] {
        store
            .record_native_source_locator(
                &xt_store::batch::SourceCursor {
                    source: SessionSource::ReadersCli,
                    cursor_key: key.into(),
                    position: 0,
                    updated_at: T,
                },
                true,
            )
            .unwrap();
    }
    let keys = |after: Option<&str>, limit| {
        store
            .source_locator_keys_after(SessionSource::ReadersCli, "codex:", after, limit)
            .unwrap()
    };
    assert_eq!(keys(None, 10), ["codex:/a", "codex:/b", "codex:/c"]);
    assert_eq!(keys(Some("codex:/a"), 1), ["codex:/b"]);
    assert!(keys(Some("codex:/c"), 10).is_empty());

    let progress = |after: &str, complete| CreationBootstrap {
        after_locator: Some(after.into()),
        complete,
    };
    store
        .advance_session_creation_bootstrap(kind, 1, &progress("codex:/b", false))
        .unwrap();
    store
        .advance_session_creation_bootstrap(kind, 1, &progress("codex:/a", false))
        .unwrap();
    assert_eq!(
        store.session_creation_bootstrap(kind, 1).unwrap(),
        progress("codex:/b", false)
    );
    store
        .advance_session_creation_bootstrap(kind, 1, &progress("codex:/c", true))
        .unwrap();
    store
        .advance_session_creation_bootstrap(kind, 1, &progress("codex:/c", false))
        .unwrap();
    assert_eq!(
        store.session_creation_bootstrap(kind, 1).unwrap(),
        progress("codex:/c", true)
    );
    // Another version starts its own pass.
    assert_eq!(
        store.session_creation_bootstrap(kind, 2).unwrap(),
        CreationBootstrap::default()
    );
}

/// An index a schema-10 build wrote gains the empty relation store, and an
/// unstarted bootstrap, without touching a session; reopening is a no-op.
#[test]
fn upgrading_an_existing_index_adds_an_empty_relation_store() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    session(&mut store, PARENT, 1, None);
    session(&mut store, CHILD, 2, None);
    drop(store);
    let connection = sql(&directory);
    connection
        .execute_batch(
            "DELETE FROM schema_version WHERE version>=11;
             DROP TABLE guardian_turn_inputs;
             DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS tool_sent_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS session_child_checks; DROP TABLE IF EXISTS session_child_facts; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins;
             DROP TABLE session_creation_relations;
             DROP TABLE session_creation_bootstrap; DROP TABLE cli_artifact_launch_owners; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
             DROP INDEX sessions_host_native;
             DROP INDEX source_cursors_tail;",
        )
        .unwrap();
    let sessions = |connection: &Connection| -> Vec<String> {
        connection
            .prepare("SELECT json_array(session_id,host,native_session_id,kind) FROM sessions ORDER BY 1")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let before = sessions(&connection);
    for _ in 0..2 {
        let store = open(&directory);
        assert_eq!(store.schema_version().unwrap(), 22);
        assert_eq!(sessions(&connection), before);
        assert!(store.session_creation(&id(CHILD)).unwrap().is_none());
        assert_eq!(
            store
                .session_creation_bootstrap(
                    CreationEvidence::CodexThreadSpawn,
                    CODEX_THREAD_SPAWN_VERSION
                )
                .unwrap(),
            CreationBootstrap::default()
        );
    }
}

/// A thread's root rollouts are found by one indexed seek on the locator's
/// tail, however many locators the index holds; a leading-wildcard pattern
/// over the same keys has to visit every one of them.
#[test]
fn a_root_locator_lookup_seeks_its_tail_instead_of_scanning_every_locator() {
    const LOCATORS: usize = 20_000;
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let n = |index: usize| format!("019a0000-0000-7000-8000-{index:012}");
    let root = |time: &str, native: &str| {
        format!("codex:/h/.codex/sessions/2026/09/07/rollout-2026-09-07T{time}-{native}.jsonl")
    };
    let mut connection = sql(&directory);
    let transaction = connection.transaction().unwrap();
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO source_cursors(source,cursor_key,position,updated_at)
                 VALUES (?1,?2,0,0)",
            )
            .unwrap();
        for index in 0..LOCATORS {
            insert
                .execute(["readers_cli", &root("12-00-00", &n(index))])
                .unwrap();
            // A continuation of every tenth thread, and a Claude transcript
            // whose name happens to end the same way.
            if index % 10 == 0 {
                let continuation = root("12-00-00", &format!("{}_{}", n(index), n(index + 1)));
                insert.execute(["readers_cli", &continuation]).unwrap();
                let transcript = format!("claude:/h/.claude/projects/p/{}.jsonl", n(index));
                insert.execute(["transcript", &transcript]).unwrap();
            }
        }
        // A second root recorded for one thread.
        insert
            .execute(["readers_cli", &root("13-00-00", &n(777))])
            .unwrap();
    }
    transaction.commit().unwrap();
    connection.execute_batch("ANALYZE").unwrap();

    let tail = |native: &str| format!("{native}.jsonl");
    let found = store
        .source_locator_keys_ending(SessionSource::ReadersCli, &tail(&n(777)), 9)
        .unwrap();
    assert_eq!(
        found,
        [root("12-00-00", &n(777)), root("13-00-00", &n(777))]
    );
    // The continuation of thread 10 ends in thread 11's name, never thread
    // 10's; the caller's name check decides what a match is.
    assert_eq!(
        store
            .source_locator_keys_ending(SessionSource::ReadersCli, &tail(&n(10)), 9)
            .unwrap(),
        [root("12-00-00", &n(10))]
    );
    assert_eq!(
        store
            .source_locator_keys_ending(SessionSource::ReadersCli, &tail(&n(777)), 1)
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .source_locator_keys_ending(SessionSource::ReadersCli, &tail(&n(LOCATORS + 1)), 9)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .source_locator_keys_ending(SessionSource::ReadersCli, "x.jsonl", 9)
            .is_err()
    );

    let plan: Vec<String> = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {LOCATOR_TAIL_QUERY}"))
        .unwrap()
        .query_map(rusqlite::params!["readers_cli", tail(&n(777)), 9], |row| {
            row.get(3)
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("SEARCH") && step.contains("source_cursors_tail")),
        "{plan:?}"
    );
    // Virtual-machine steps: a handful for the seek, one or more per key for
    // the pattern the title read uses.
    let steps = |query: &str, key: &str| {
        let mut statement = connection.prepare(query).unwrap();
        let rows = statement
            .query_map(rusqlite::params!["readers_cli", key, 9], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .count();
        assert_eq!(rows, 2);
        statement.get_status(rusqlite::StatementStatus::VmStep)
    };
    let seek = steps(LOCATOR_TAIL_QUERY, &tail(&n(777)));
    let scan = steps(
        "SELECT cursor_key FROM source_cursors WHERE source=?1
           AND cursor_key LIKE ?2 ESCAPE '\\' ORDER BY cursor_key LIMIT ?3",
        &format!("codex:%-{}.jsonl", n(777)),
    );
    assert!(seek < 200, "the seek took {seek} steps");
    assert!(scan > LOCATORS as i32, "the pattern took {scan} steps");
}
