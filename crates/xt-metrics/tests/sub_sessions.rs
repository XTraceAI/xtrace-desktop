//! The verified sub-session walk under listed sessions: every depth, never a
//! named session, and a limit that counts what it leaves out.
use xt_metrics::MetricsDb;
use xt_store::{
    Host, SessionMeta, SessionSource,
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationEvidence, CreationWitness, SessionCreationProof,
    },
};

fn spawn(child: &str, parent: &str) -> SessionCreationProof {
    SessionCreationProof {
        child_session_id: format!("codex-{child}"),
        child_host: Host::Codex,
        child_native_session_id: child.into(),
        parent_host: Host::Codex,
        parent_native_session_id: parent.into(),
        evidence_kind: CreationEvidence::CodexThreadSpawn,
        evidence_version: CODEX_THREAD_SPAWN_VERSION,
        witness: CreationWitness::RolloutOpeningSessionMeta,
    }
}

#[test]
fn walks_every_level_and_counts_what_the_limit_leaves_out() {
    let mut db = xt_fixtures::TempDb::empty().unwrap();
    // root → a, b; a → a1; b → (nothing); other → o. "listed" is named too.
    for native in ["root", "a", "b", "a1", "other", "o", "listed"] {
        let mut session =
            SessionMeta::new(format!("codex-{native}"), "codex", SessionSource::Fixture);
        session.native_session_id = Some(native.into());
        db.store_mut().upsert_session(&session, false).unwrap();
    }
    db.store_mut()
        .record_session_creations(
            &[
                spawn("a", "root"),
                spawn("b", "root"),
                spawn("a1", "a"),
                spawn("o", "other"),
                spawn("listed", "root"),
            ],
            1,
        )
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let all = metrics
        .sub_sessions(&["codex-root", "codex-listed"], 10)
        .unwrap();
    // Every level, nearer first; never a named session or another tree.
    let mut first: Vec<_> = all.sessions[..2].to_vec();
    first.sort();
    assert_eq!(first, ["codex-a", "codex-b"]);
    assert_eq!(all.sessions[2], "codex-a1");
    assert_eq!(all.sessions.len(), 3);
    assert!(all.not_shown.is_empty());
    // A limit keeps the nearest and counts the rest under the named session
    // whose walk found them.
    let capped = metrics.sub_sessions(&["codex-root"], 1).unwrap();
    assert_eq!(capped.sessions.len(), 1);
    assert_eq!(
        capped.not_shown.into_iter().collect::<Vec<_>>(),
        [("codex-root".to_owned(), 3)]
    );
}
