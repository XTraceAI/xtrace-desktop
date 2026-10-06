//! The context's known-child bit: stored positive evidence that another
//! session created this one, from an accepted creation relation or an
//! unconflicted saved Human session origin bound to the session's exact
//! canonical ID, host and native ID. It is read independently of the parent,
//! a conflicted or mismatched fact alone never sets it, and for these two
//! facts it agrees with the Human input view's own treatment of the same
//! session's user messages. Child facts, which set the bit without that
//! view reading them, are covered in `child_facts.rs`.

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, SessionSource, Store,
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationDisposition, CreationEvidence, CreationWitness,
        SessionCreationProof,
    },
    human_input::{OriginManifest, SessionOrigin},
    session_list::{self, SessionContext},
};

const PARENT: &str = "019a0000-0000-7000-8000-0000000000aa";
const CHILD: &str = "019a0000-0000-7000-8000-0000000000bb";
const OTHER: &str = "019a0000-0000-7000-8000-0000000000cc";
const UNINDEXED: &str = "019a0000-0000-7000-8000-0000000000dd";
const PLAIN: &str = "019a0000-0000-7000-8000-0000000000ee";

fn id(native: &str) -> String {
    format!("codex-{native}")
}

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("known-child.sqlite")).unwrap()
}

fn sql(directory: &TempDir) -> Connection {
    Connection::open(directory.path().join("known-child.sqlite")).unwrap()
}

/// An untitled session with no recorded start and one user message, so the
/// Human input view has a message to decide about.
fn session(store: &mut Store, native: &str) {
    let mut meta = SessionMeta::new(id(native), "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some(native.into());
    store.upsert_session(&meta, false).unwrap();
    let input: CanonicalRecord = serde_json::from_value(json!({
        "uuid": format!("input-{native}"), "type": "user",
        "timestamp": "2026-09-01T00:00:00Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic"}]}
    }))
    .unwrap();
    store.upsert_records(&id(native), &[input], false).unwrap();
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

fn origin(child: &str, parent: &str) -> SessionOrigin {
    SessionOrigin {
        session_id: id(child),
        host: Host::Codex,
        native_session_id: child.into(),
        parent_host: Host::Codex,
        parent_native_session_id: parent.into(),
        method: "explicit_session_id".into(),
        evidence_id: "audit-1".into(),
        launch_id: "call-1".into(),
    }
}

fn save_origin(store: &mut Store, origin: SessionOrigin) -> usize {
    let report = store
        .import_human_session_origins(&OriginManifest {
            version: 1,
            sessions: vec![origin],
        })
        .unwrap();
    report.applied + report.unchanged
}

fn context(directory: &TempDir, native: &str) -> SessionContext {
    session_list::context(&sql(directory), &[&id(native)])
        .unwrap()
        .remove(0)
}

/// The bit, and the Human input view's verdict on the session's one user
/// message, which no other exclusion touches here. The two must agree: the
/// bit repeats that view's creation and origin conditions.
fn known(store: &Store, directory: &TempDir, native: &str) -> bool {
    let bit = context(directory, native).known_child;
    let excluded = store.records(&id(native)).unwrap()[0].human_excluded;
    assert_eq!(bit, excluded, "{native}: bit and Human input view disagree");
    bit
}

#[test]
fn an_accepted_creation_marks_a_known_child_whatever_its_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for native in [PARENT, CHILD, OTHER, PLAIN] {
        session(&mut store, native);
    }
    assert_eq!(
        store
            .record_session_creations(&[spawn(CHILD, PARENT), spawn(OTHER, UNINDEXED)], 1)
            .unwrap()
            .dispositions,
        [CreationDisposition::Recorded, CreationDisposition::Recorded]
    );
    // An indexed parent: known, and linked.
    assert!(known(&store, &directory, CHILD));
    assert_eq!(
        context(&directory, CHILD).parent.unwrap().session_id,
        id(PARENT)
    );
    // A missing parent: known, with no parent.
    assert!(known(&store, &directory, OTHER));
    assert_eq!(context(&directory, OTHER).parent, None);
    // An ambiguous parent: two user sessions hold its native identity.
    session(&mut store, UNINDEXED);
    let mut twin = SessionMeta::new("plugin-twin", "codex", SessionSource::Plugin);
    twin.native_session_id = Some(UNINDEXED.into());
    store.upsert_session(&twin, false).unwrap();
    assert!(known(&store, &directory, OTHER));
    assert_eq!(context(&directory, OTHER).parent, None);
    // Neither parent nor an ordinary untitled session is a known child.
    assert!(!known(&store, &directory, PARENT));
    assert!(!known(&store, &directory, PLAIN));
    let plain = context(&directory, PLAIN);
    assert_eq!((plain.title, plain.parent), (None, None));
}

#[test]
fn an_unconflicted_saved_origin_marks_a_known_child_without_a_parent() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for native in [CHILD, OTHER, UNINDEXED] {
        session(&mut store, native);
    }
    assert_eq!(save_origin(&mut store, origin(CHILD, PARENT)), 1);
    // The parent is not indexed: known, with no parent.
    assert!(known(&store, &directory, CHILD));
    assert_eq!(context(&directory, CHILD).parent, None);
    // The parent is ambiguous: known, with no parent.
    assert_eq!(save_origin(&mut store, origin(OTHER, UNINDEXED)), 1);
    let mut twin = SessionMeta::new("plugin-twin", "codex", SessionSource::Plugin);
    twin.native_session_id = Some(UNINDEXED.into());
    store.upsert_session(&twin, false).unwrap();
    assert!(known(&store, &directory, OTHER));
    assert_eq!(context(&directory, OTHER).parent, None);
}

#[test]
fn a_conflicted_or_mismatched_fact_alone_is_not_known() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for native in [PARENT, OTHER, CHILD, PLAIN] {
        session(&mut store, native);
    }
    // A conflicted creation relation alone.
    store
        .record_session_creations(&[spawn(CHILD, PARENT)], 1)
        .unwrap();
    assert_eq!(
        store
            .record_session_creations(&[spawn(CHILD, OTHER)], 2)
            .unwrap()
            .dispositions,
        [CreationDisposition::Conflicted]
    );
    assert!(!known(&store, &directory, CHILD));
    // A conflicted origin alone.
    assert_eq!(save_origin(&mut store, origin(PLAIN, PARENT)), 1);
    assert!(known(&store, &directory, PLAIN));
    let mut disagreeing = origin(PLAIN, PARENT);
    disagreeing.launch_id = "call-2".into();
    store
        .import_human_session_origins(&OriginManifest {
            version: 1,
            sessions: vec![disagreeing],
        })
        .unwrap();
    assert!(!known(&store, &directory, PLAIN));
    // An accepted relation and an unconflicted origin, both no longer bound
    // to the session's native identity.
    assert_eq!(
        store
            .record_session_creations(&[spawn(OTHER, PARENT)], 3)
            .unwrap()
            .dispositions,
        [CreationDisposition::Recorded]
    );
    let mut other = origin(OTHER, PARENT);
    other.parent_native_session_id = UNINDEXED.into();
    assert_eq!(save_origin(&mut store, other), 1);
    assert!(known(&store, &directory, OTHER));
    sql(&directory)
        .execute(
            "UPDATE sessions SET native_session_id='moved' WHERE session_id=?1",
            [id(OTHER)],
        )
        .unwrap();
    assert!(!known(&store, &directory, OTHER));
}

#[test]
fn another_independent_valid_fact_still_marks_a_known_child() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for native in [PARENT, OTHER, CHILD, PLAIN] {
        session(&mut store, native);
    }
    // A conflicted creation relation beside a valid saved origin.
    store
        .record_session_creations(&[spawn(CHILD, PARENT)], 1)
        .unwrap();
    store
        .record_session_creations(&[spawn(CHILD, OTHER)], 2)
        .unwrap();
    assert!(!known(&store, &directory, CHILD));
    assert_eq!(save_origin(&mut store, origin(CHILD, UNINDEXED)), 1);
    assert!(known(&store, &directory, CHILD));
    assert_eq!(context(&directory, CHILD).parent, None);
    // A conflicted origin beside an accepted creation relation.
    assert_eq!(save_origin(&mut store, origin(PLAIN, PARENT)), 1);
    let mut disagreeing = origin(PLAIN, PARENT);
    disagreeing.launch_id = "call-2".into();
    store
        .import_human_session_origins(&OriginManifest {
            version: 1,
            sessions: vec![disagreeing],
        })
        .unwrap();
    assert!(!known(&store, &directory, PLAIN));
    store
        .record_session_creations(&[spawn(PLAIN, UNINDEXED)], 3)
        .unwrap();
    assert!(known(&store, &directory, PLAIN));
    assert_eq!(context(&directory, PLAIN).parent, None);
}
