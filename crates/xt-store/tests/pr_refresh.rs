//! Pull-request refresh persistence and status (migration 8). Synthetic
//! identities, typed results and disposable file databases only; no network,
//! Git, subprocess or GitHub client.

use rusqlite::{Connection, types::Value as SqlValue};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use xt_store::{
    Error, SessionMeta, SessionSource, Store,
    pr_link::{
        MAX_ATTEMPTED_AT, MAX_HEAD_REF_LEN, MAX_TITLE_LEN, PrConfidence, PrIdentity,
        PrLinkObservation, PrRefreshError, PrRefreshStatus, PrState, RefreshFailure,
        RefreshOutcome, RefreshSuccess, RefreshWrite,
    },
};

const URL: &str = "https://github.com/octo-org/hello.world/pull/42";
const ERRORS: [PrRefreshError; 9] = [
    PrRefreshError::Unavailable,
    PrRefreshError::Timeout,
    PrRefreshError::Cancelled,
    PrRefreshError::OutputTooLarge,
    PrRefreshError::InvalidResponse,
    PrRefreshError::ExecutionFailed,
    PrRefreshError::NotFound,
    PrRefreshError::Unauthorized,
    PrRefreshError::RateLimited,
];

fn identity(url: &str) -> PrIdentity {
    PrIdentity::from_url(url).unwrap()
}

/// A store whose sessions link `urls` exactly, so each has a canonical row.
fn open(directory: &TempDir, urls: &[&str]) -> (Store, PathBuf) {
    let path = directory.path().join("refresh.sqlite");
    let mut store = Store::open(&path).unwrap();
    for session in ["session-a", "session-b"] {
        store
            .upsert_session(
                &SessionMeta::new(session, "claude", SessionSource::Fixture),
                false,
            )
            .unwrap();
    }
    for url in urls {
        store
            .record_pr_link(&PrLinkObservation {
                session_id: "session-a".into(),
                pull_request: identity(url),
                confidence: PrConfidence::Sha,
                first_seen_at: 10,
                last_seen_at: 20,
            })
            .unwrap();
    }
    (store, path)
}

fn success(url: &str, at: i64, state: PrState) -> RefreshSuccess {
    RefreshSuccess {
        pull_request: identity(url),
        attempted_at: at,
        title: format!("Synthetic title {at}"),
        state,
        merged_at: (state == PrState::Merged).then(|| "2026-09-19T12:34:56Z".into()),
        additions: at,
        deletions: 3,
        head_ref_name: format!("feature/branch-{at}"),
    }
}

fn failure(url: &str, at: i64, error: PrRefreshError) -> RefreshOutcome {
    RefreshOutcome::Failure(RefreshFailure {
        pull_request: identity(url),
        attempted_at: at,
        error,
    })
}

fn rows(path: &Path, sql: &str) -> Vec<Vec<SqlValue>> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|index| row.get(index)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn snapshot(path: &Path) -> (Vec<Vec<SqlValue>>, Vec<Vec<SqlValue>>) {
    (
        rows(path, "SELECT * FROM pull_requests ORDER BY id"),
        rows(path, "SELECT * FROM pr_links ORDER BY session_id, pr_id"),
    )
}

fn invalid(result: xt_store::Result<RefreshWrite>) -> bool {
    matches!(result, Err(Error::InvalidInput(_)))
}

#[test]
fn migration_8_adds_nullable_status_columns_and_preserves_rows() {
    let directory = TempDir::new().unwrap();
    let (store, path) = open(&directory, &[URL]);
    drop(store);
    // What a schema-7 build left behind, with a refresh written outside any
    // typed writer: every value must survive the upgrade unchanged.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "DELETE FROM schema_version WHERE version>=8;
         ALTER TABLE pull_requests DROP COLUMN refresh_error;
         ALTER TABLE pull_requests DROP COLUMN last_attempted_at;
         ALTER TABLE tool_uses DROP COLUMN group_key;
         ALTER TABLE tool_uses DROP COLUMN group_version;
         ALTER TABLE tool_uses DROP COLUMN group_conflict;
         DROP TABLE confirmed_automated_inputs;
         DROP TABLE guardian_turn_inputs;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS tool_sent_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS session_child_checks; DROP TABLE IF EXISTS session_child_facts; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins;
         DROP TABLE session_creation_relations;
         DROP TABLE session_creation_bootstrap; DROP TABLE cli_artifact_launch_owners; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         DROP INDEX sessions_host_native; DROP INDEX source_cursors_tail;
         INSERT INTO pull_requests(repo,number,url,title,state,merged_at,additions,deletions,head_ref_name,refreshed_at)
             VALUES ('octo-org/legacy',7,'https://github.com/octo-org/legacy/pull/7','Legacy','MERGED',
                     '2026-01-02T03:04:05Z',1,2,'legacy-branch',500);",
    )
    .unwrap();
    let before = snapshot(&path);
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 22);
        let (pull_requests, links) = snapshot(&path);
        assert_eq!(links, before.1);
        assert_eq!(pull_requests.len(), before.0.len());
        for (after, before) in pull_requests.iter().zip(&before.0) {
            assert_eq!(after[..before.len()], before[..]);
            assert_eq!(after[before.len()..], [SqlValue::Null, SqlValue::Null]);
        }
        let stub = store.pull_request(&identity(URL)).unwrap().unwrap();
        assert_eq!(stub.refresh_status(), PrRefreshStatus::NeverAttempted);
        // A pre-8 success has no attempt time; it still reads as refreshed.
        let legacy = store
            .pull_request(&identity("https://github.com/octo-org/legacy/pull/7"))
            .unwrap()
            .unwrap();
        assert_eq!(legacy.refresh_status(), PrRefreshStatus::Refreshed);
        assert_eq!(legacy.last_attempted_at, None);
    }
    assert_eq!(
        rows(&path, "SELECT count(*) FROM schema_version"),
        vec![vec![SqlValue::Integer(22)]]
    );
    // The closed vocabulary and status shape are enforced by the schema too.
    for statement in [
        "UPDATE pull_requests SET refresh_error='timeout'",
        "UPDATE pull_requests SET last_attempted_at=900, refresh_error='stderr: boom'",
        "UPDATE pull_requests SET last_attempted_at=900, refresh_error='TIMEOUT'",
        "UPDATE pull_requests SET last_attempted_at='later'",
        "UPDATE pull_requests SET last_attempted_at=-1",
        "UPDATE pull_requests SET last_attempted_at=499 WHERE refreshed_at=500",
    ] {
        assert!(raw.execute(statement, []).is_err(), "{statement}");
    }
    assert_eq!(snapshot(&path).0.len(), 2);

    // The legacy success orders attempts: older is stale, newer applies.
    let mut store = Store::open(&path).unwrap();
    let legacy = "https://github.com/octo-org/legacy/pull/7";
    let kept = store.pull_request(&identity(legacy)).unwrap().unwrap();
    assert_eq!(
        store
            .record_pr_refresh(&failure(legacy, 499, PrRefreshError::Timeout))
            .unwrap(),
        RefreshWrite::Stale
    );
    assert!(invalid(store.record_pr_refresh(&failure(
        legacy,
        500,
        PrRefreshError::Timeout
    ))));
    assert_eq!(
        store
            .record_pr_refresh(&failure(legacy, 501, PrRefreshError::Timeout))
            .unwrap(),
        RefreshWrite::Applied
    );
    let failed = store.pull_request(&identity(legacy)).unwrap().unwrap();
    assert_eq!(
        (failed.last_attempted_at, failed.refresh_error),
        (Some(501), Some(PrRefreshError::Timeout))
    );
    assert_eq!(
        (failed.title, failed.merged_at, failed.refreshed_at),
        (kept.title, kept.merged_at, kept.refreshed_at)
    );
}

#[test]
fn successes_store_open_closed_and_merged_metadata_without_touching_links() {
    for state in [PrState::Open, PrState::Closed, PrState::Merged] {
        let directory = TempDir::new().unwrap();
        let (mut store, path) = open(&directory, &[URL]);
        let links = snapshot(&path).1;
        let refresh = success(URL, 100, state);
        assert_eq!(
            store
                .record_pr_refresh(&RefreshOutcome::Success(refresh.clone()))
                .unwrap(),
            RefreshWrite::Applied
        );
        let stored = store.pull_request(&identity(URL)).unwrap().unwrap();
        assert_eq!(stored.title.as_deref(), Some(refresh.title.as_str()));
        assert_eq!(stored.state, Some(state));
        assert_eq!(stored.merged_at, refresh.merged_at);
        assert_eq!((stored.additions, stored.deletions), (Some(100), Some(3)));
        assert_eq!(
            stored.head_ref_name.as_deref(),
            Some(refresh.head_ref_name.as_str())
        );
        assert_eq!(
            (stored.refreshed_at, stored.last_attempted_at),
            (Some(100), Some(100))
        );
        assert_eq!(stored.refresh_error, None);
        assert_eq!(stored.refresh_status(), PrRefreshStatus::Refreshed);
        // Links, confidence and first/last-seen evidence are never touched.
        assert_eq!(snapshot(&path).1, links, "{state:?}");
        assert_eq!(
            rows(&path, "SELECT count(*) FROM pull_requests"),
            vec![vec![SqlValue::Integer(1)]]
        );
    }
}

#[test]
fn a_newer_success_replaces_every_refresh_owned_field_and_clears_the_error() {
    let directory = TempDir::new().unwrap();
    let (mut store, _) = open(&directory, &[URL]);
    let merged = success(URL, 100, PrState::Merged);
    store
        .record_pr_refresh(&RefreshOutcome::Success(merged))
        .unwrap();
    store
        .record_pr_refresh(&failure(URL, 150, PrRefreshError::RateLimited))
        .unwrap();
    // A later result reports a different state: merged_at is replaced by NULL
    // rather than kept from the earlier success.
    let closed = RefreshSuccess {
        title: "Renamed".into(),
        additions: 0,
        deletions: 0,
        head_ref_name: "other".into(),
        ..success(URL, 200, PrState::Closed)
    };
    assert_eq!(
        store
            .record_pr_refresh(&RefreshOutcome::Success(closed))
            .unwrap(),
        RefreshWrite::Applied
    );
    let stored = store.pull_request(&identity(URL)).unwrap().unwrap();
    assert_eq!(stored.refresh_status(), PrRefreshStatus::Refreshed);
    assert_eq!(
        (
            stored.title.as_deref(),
            stored.state,
            stored.merged_at,
            stored.additions,
            stored.deletions,
            stored.head_ref_name.as_deref(),
            stored.refreshed_at,
            stored.last_attempted_at,
            stored.refresh_error,
        ),
        (
            Some("Renamed"),
            Some(PrState::Closed),
            None,
            Some(0),
            Some(0),
            Some("other"),
            Some(200),
            Some(200),
            None,
        )
    );
}

#[test]
fn every_failure_code_preserves_the_last_successful_metadata() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(
        &directory,
        &[URL, "https://github.com/octo-org/never/pull/1"],
    );
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(URL, 100, PrState::Merged)))
        .unwrap();
    let refreshed = store.pull_request(&identity(URL)).unwrap().unwrap();
    let links = snapshot(&path).1;
    for (offset, error) in (1..).zip(ERRORS) {
        let at = 100 + offset;
        assert_eq!(
            store.record_pr_refresh(&failure(URL, at, error)).unwrap(),
            RefreshWrite::Applied,
            "{error:?}"
        );
        let stored = store.pull_request(&identity(URL)).unwrap().unwrap();
        assert_eq!(
            (stored.last_attempted_at, stored.refresh_error),
            (Some(at), Some(error))
        );
        assert_eq!(
            stored.refresh_status(),
            PrRefreshStatus::FailedAfterRefresh(error)
        );
        let expected = xt_store::pr_link::StoredPullRequest {
            last_attempted_at: Some(at),
            refresh_error: Some(error),
            ..refreshed.clone()
        };
        assert_eq!(stored, expected, "{error:?}");
        assert_eq!(
            rows(
                &path,
                "SELECT refresh_error FROM pull_requests WHERE refreshed_at=100"
            ),
            vec![vec![SqlValue::Text(error.as_str().into())]]
        );
    }
    // A failure before any success leaves every refresh-owned field unknown.
    let never = identity("https://github.com/octo-org/never/pull/1");
    store
        .record_pr_refresh(&failure(never.url().as_str(), 5, PrRefreshError::NotFound))
        .unwrap();
    let stored = store.pull_request(&never).unwrap().unwrap();
    assert_eq!(
        stored.refresh_status(),
        PrRefreshStatus::FailedNeverRefreshed(PrRefreshError::NotFound)
    );
    assert_eq!(
        (
            &stored.title,
            stored.state,
            &stored.merged_at,
            stored.additions,
            stored.deletions,
            &stored.head_ref_name,
            stored.refreshed_at
        ),
        (&None, None, &None, None, None, &None, None)
    );
    assert_eq!(snapshot(&path).1, links);
}

#[test]
fn malformed_results_unknown_identities_and_conflicts_write_nothing() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory, &[URL]);
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(URL, 100, PrState::Open)))
        .unwrap();
    let base = || success(URL, 200, PrState::Open);
    let merged = || success(URL, 200, PrState::Merged);
    let malformed = [
        RefreshSuccess {
            title: String::new(),
            ..base()
        },
        RefreshSuccess {
            title: "   ".into(),
            ..base()
        },
        RefreshSuccess {
            title: "line\nbreak".into(),
            ..base()
        },
        RefreshSuccess {
            title: "bell\u{7}".into(),
            ..base()
        },
        RefreshSuccess {
            title: "next\u{85}line".into(),
            ..base()
        },
        RefreshSuccess {
            title: "t".repeat(MAX_TITLE_LEN + 1),
            ..base()
        },
        RefreshSuccess {
            head_ref_name: String::new(),
            ..base()
        },
        RefreshSuccess {
            head_ref_name: "tab\tref".into(),
            ..base()
        },
        RefreshSuccess {
            head_ref_name: "h".repeat(MAX_HEAD_REF_LEN + 1),
            ..base()
        },
        RefreshSuccess {
            additions: -1,
            ..base()
        },
        RefreshSuccess {
            deletions: -1,
            ..base()
        },
        RefreshSuccess {
            merged_at: None,
            ..merged()
        },
        RefreshSuccess {
            merged_at: Some("yesterday".into()),
            ..merged()
        },
        RefreshSuccess {
            merged_at: Some("2026-09-19 12:00:00".into()),
            ..merged()
        },
        RefreshSuccess {
            merged_at: Some(String::new()),
            ..merged()
        },
        RefreshSuccess {
            merged_at: Some("2026-09-19T12:00:00Z".into()),
            ..base()
        },
        RefreshSuccess {
            merged_at: Some("2026-09-19T12:00:00Z".into()),
            ..success(URL, 200, PrState::Closed)
        },
        RefreshSuccess {
            attempted_at: -1,
            ..base()
        },
        RefreshSuccess {
            attempted_at: MAX_ATTEMPTED_AT + 1,
            ..base()
        },
    ];
    let before = snapshot(&path);
    for refresh in malformed {
        assert!(
            invalid(store.record_pr_refresh(&RefreshOutcome::Success(refresh.clone()))),
            "{refresh:?}"
        );
        assert_eq!(snapshot(&path), before, "{refresh:?}");
    }
    // Validation precedes ordering: an older malformed result still fails.
    assert!(invalid(store.record_pr_refresh(&RefreshOutcome::Success(
        RefreshSuccess {
            additions: -1,
            ..success(URL, 1, PrState::Open)
        }
    ))));
    for at in [-1, i64::MIN, MAX_ATTEMPTED_AT + 1] {
        assert!(invalid(store.record_pr_refresh(&failure(
            URL,
            at,
            PrRefreshError::Timeout
        ))));
    }
    // The boundary itself is accepted.
    assert_eq!(
        store
            .record_pr_refresh(&failure(URL, MAX_ATTEMPTED_AT, PrRefreshError::Timeout))
            .unwrap(),
        RefreshWrite::Applied
    );
    let before = snapshot(&path);

    // Refresh never creates a stub: an unknown identity fails.
    let unknown = "https://github.com/octo-org/unknown/pull/1";
    assert!(invalid(store.record_pr_refresh(&RefreshOutcome::Success(
        success(unknown, 300, PrState::Open)
    ))));
    assert!(invalid(store.record_pr_refresh(&failure(
        unknown,
        300,
        PrRefreshError::NotFound
    ))));
    assert!(store.pull_request(&identity(unknown)).unwrap().is_none());
    assert_eq!(snapshot(&path), before);

    // Legacy rows the canonical lookup rejects are conflicts, never refreshed.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "INSERT INTO pull_requests(repo,number,url) VALUES
             ('Octo-Org/Upper',6,'https://github.com/Octo-Org/Upper/pull/6'),
             ('octo-org/alias',9,'https://github.com/octo-org/elsewhere/pull/9'),
             ('octo-org/split',3,'https://github.com/octo-org/split/pull/4'),
             ('octo-org/multi',2,'https://github.com/octo-org/multi/pull/2'),
             ('Octo-Org/Multi',2,'https://github.com/Octo-Org/Multi/pull/2');",
    )
    .unwrap();
    let before = snapshot(&path);
    for conflicting in [
        "https://github.com/octo-org/upper/pull/6",
        "https://github.com/octo-org/elsewhere/pull/9",
        "https://github.com/octo-org/alias/pull/9",
        "https://github.com/octo-org/split/pull/3",
        "https://github.com/octo-org/split/pull/4",
        "https://github.com/octo-org/multi/pull/2",
    ] {
        assert!(
            invalid(store.record_pr_refresh(&RefreshOutcome::Success(success(
                conflicting,
                300,
                PrState::Open
            )))),
            "{conflicting}"
        );
        assert!(invalid(store.record_pr_refresh(&failure(
            conflicting,
            300,
            PrRefreshError::Timeout
        ))));
        assert_eq!(snapshot(&path), before, "{conflicting}");
    }
}

#[test]
fn an_injected_write_failure_rolls_back() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory, &[URL]);
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(URL, 100, PrState::Open)))
        .unwrap();
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER inject_failure AFTER UPDATE ON pull_requests
         BEGIN SELECT RAISE(ABORT,'synthetic refresh failure'); END;",
    )
    .unwrap();
    let before = snapshot(&path);
    for outcome in [
        RefreshOutcome::Success(success(URL, 200, PrState::Merged)),
        failure(URL, 200, PrRefreshError::ExecutionFailed),
    ] {
        assert!(matches!(
            store.record_pr_refresh(&outcome),
            Err(Error::Sqlite(_))
        ));
        assert_eq!(snapshot(&path), before);
    }
    raw.execute_batch("DROP TRIGGER inject_failure").unwrap();
    assert_eq!(
        store
            .record_pr_refresh(&failure(URL, 200, PrRefreshError::ExecutionFailed))
            .unwrap(),
        RefreshWrite::Applied
    );
}

#[test]
fn older_attempts_are_stale_and_write_nothing() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory, &[URL]);
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(URL, 200, PrState::Merged)))
        .unwrap();
    let before = snapshot(&path);
    for outcome in [
        RefreshOutcome::Success(success(URL, 199, PrState::Open)),
        RefreshOutcome::Success(success(URL, 0, PrState::Closed)),
        failure(URL, 150, PrRefreshError::Timeout),
    ] {
        assert_eq!(
            store.record_pr_refresh(&outcome).unwrap(),
            RefreshWrite::Stale
        );
        assert_eq!(snapshot(&path), before);
    }
    // A newer failure becomes the ordering watermark for later successes too.
    store
        .record_pr_refresh(&failure(URL, 300, PrRefreshError::Unavailable))
        .unwrap();
    let before = snapshot(&path);
    assert_eq!(
        store
            .record_pr_refresh(&RefreshOutcome::Success(success(URL, 250, PrState::Open)))
            .unwrap(),
        RefreshWrite::Stale
    );
    assert_eq!(snapshot(&path), before);
}

#[test]
fn equal_attempts_replay_identically_or_conflict() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory, &[URL]);
    let merged = success(URL, 100, PrState::Merged);
    store
        .record_pr_refresh(&RefreshOutcome::Success(merged.clone()))
        .unwrap();
    let before = snapshot(&path);
    assert_eq!(
        store
            .record_pr_refresh(&RefreshOutcome::Success(merged.clone()))
            .unwrap(),
        RefreshWrite::Unchanged
    );
    assert_eq!(snapshot(&path), before);
    let differing = [
        RefreshOutcome::Success(RefreshSuccess {
            title: "Other".into(),
            ..merged.clone()
        }),
        RefreshOutcome::Success(RefreshSuccess {
            additions: 1,
            ..merged.clone()
        }),
        RefreshOutcome::Success(RefreshSuccess {
            deletions: 4,
            ..merged.clone()
        }),
        RefreshOutcome::Success(RefreshSuccess {
            head_ref_name: "other".into(),
            ..merged.clone()
        }),
        // The same instant spelled differently is still a different result.
        RefreshOutcome::Success(RefreshSuccess {
            merged_at: Some("2026-09-19T12:34:56.000Z".into()),
            ..merged.clone()
        }),
        RefreshOutcome::Success(success(URL, 100, PrState::Closed)),
        failure(URL, 100, PrRefreshError::Timeout),
    ];
    for outcome in &differing {
        assert!(invalid(store.record_pr_refresh(outcome)), "{outcome:?}");
        assert_eq!(snapshot(&path), before, "{outcome:?}");
    }

    store
        .record_pr_refresh(&failure(URL, 200, PrRefreshError::Timeout))
        .unwrap();
    let before = snapshot(&path);
    assert_eq!(
        store
            .record_pr_refresh(&failure(URL, 200, PrRefreshError::Timeout))
            .unwrap(),
        RefreshWrite::Unchanged
    );
    for outcome in [
        failure(URL, 200, PrRefreshError::Cancelled),
        RefreshOutcome::Success(RefreshSuccess {
            attempted_at: 200,
            ..merged.clone()
        }),
    ] {
        assert!(invalid(store.record_pr_refresh(&outcome)), "{outcome:?}");
        assert_eq!(snapshot(&path), before);
    }
}

#[test]
fn status_persists_across_reopen_and_reads_in_deterministic_order() {
    let urls = [
        "https://github.com/zeta/app/pull/10",
        "https://github.com/alpha/app/pull/3",
        "https://github.com/zeta/app/pull/2",
        "https://github.com/alpha/app/pull/12",
    ];
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory, &urls);
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(
            urls[0],
            10,
            PrState::Open,
        )))
        .unwrap();
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(
            urls[1],
            20,
            PrState::Merged,
        )))
        .unwrap();
    store
        .record_pr_refresh(&failure(urls[1], 30, PrRefreshError::Unauthorized))
        .unwrap();
    store
        .record_pr_refresh(&failure(urls[2], 40, PrRefreshError::OutputTooLarge))
        .unwrap();
    let before = store.all_pull_requests().unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let after = store.all_pull_requests().unwrap();
    assert_eq!(after, before);
    let summary = after
        .iter()
        .map(|pr| (pr.identity.url(), pr.refresh_status()))
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            (
                "https://github.com/alpha/app/pull/3".into(),
                PrRefreshStatus::FailedAfterRefresh(PrRefreshError::Unauthorized)
            ),
            (
                "https://github.com/alpha/app/pull/12".into(),
                PrRefreshStatus::NeverAttempted
            ),
            (
                "https://github.com/zeta/app/pull/2".into(),
                PrRefreshStatus::FailedNeverRefreshed(PrRefreshError::OutputTooLarge)
            ),
            (
                "https://github.com/zeta/app/pull/10".into(),
                PrRefreshStatus::Refreshed
            ),
        ]
    );
    for pr in &after {
        assert_eq!(store.pull_request(&pr.identity).unwrap().as_ref(), Some(pr));
    }
}

#[test]
fn only_typed_codes_are_stored_and_no_other_table_changes() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory, &[URL]);
    let others = |path: &Path| {
        rows(
            path,
            "SELECT (SELECT count(*) FROM records),(SELECT count(*) FROM tool_uses),
                    (SELECT count(*) FROM capture_receipts),(SELECT count(*) FROM settings),
                    (SELECT count(*) FROM native_checkpoints),
                    (SELECT group_concat(coalesce(title,'')||coalesce(cwd,'')||coalesce(repo,'')) FROM sessions)",
        )
    };
    let before = others(&path);
    for (offset, error) in (1..).zip(ERRORS) {
        store
            .record_pr_refresh(&failure(URL, offset, error))
            .unwrap();
    }
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(URL, 50, PrState::Closed)))
        .unwrap();
    store
        .record_pr_refresh(&failure(URL, 60, PrRefreshError::InvalidResponse))
        .unwrap();
    // The table gains exactly the attempt time and the typed code: no message,
    // stderr, response body or stale flag.
    assert_eq!(
        rows(
            &path,
            "SELECT name FROM pragma_table_info('pull_requests') ORDER BY cid"
        )
        .into_iter()
        .map(|row| match &row[0] {
            SqlValue::Text(name) => name.clone(),
            other => panic!("{other:?}"),
        })
        .collect::<Vec<_>>(),
        [
            "id",
            "repo",
            "number",
            "url",
            "title",
            "state",
            "merged_at",
            "additions",
            "deletions",
            "head_ref_name",
            "refreshed_at",
            "last_attempted_at",
            "refresh_error",
        ]
    );
    assert_eq!(
        rows(
            &path,
            "SELECT last_attempted_at, refresh_error, refreshed_at FROM pull_requests"
        ),
        vec![vec![
            SqlValue::Integer(60),
            SqlValue::Text("invalid_response".into()),
            SqlValue::Integer(50),
        ]]
    );
    assert_eq!(others(&path), before);
    assert_eq!(
        ERRORS.map(PrRefreshError::as_str),
        [
            "unavailable",
            "timeout",
            "cancelled",
            "output_too_large",
            "invalid_response",
            "execution_failed",
            "not_found",
            "unauthorized",
            "rate_limited",
        ]
    );
}

/// One read for both facts: the rows a caller lists and the number of sessions
/// linking each of them come from the same snapshot, so a writer on another
/// connection cannot be observed half applied. Ordering is the stable
/// repository/number order every pull-request reader uses.
#[test]
fn linked_pull_requests_read_metadata_and_link_counts_together() {
    let directory = TempDir::new().unwrap();
    let second = "https://github.com/octo-org/hello.world/pull/7";
    let third = "https://github.com/octo-org/aardvark/pull/1";
    let (mut store, path) = open(&directory, &[URL, second, third]);
    // A second session links only the first pull request.
    store
        .record_pr_link(&PrLinkObservation {
            session_id: "session-b".into(),
            pull_request: identity(URL),
            confidence: PrConfidence::Inferred,
            first_seen_at: 30,
            last_seen_at: 40,
        })
        .unwrap();
    store
        .record_pr_refresh(&RefreshOutcome::Success(success(URL, 50, PrState::Open)))
        .unwrap();

    let listed = store.linked_pull_requests().unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|row| (row.pull_request.identity.url(), row.linked_sessions))
            .collect::<Vec<_>>(),
        vec![
            (identity(third).url(), 1),
            (identity(second).url(), 1),
            (identity(URL).url(), 2),
        ]
    );
    // The metadata is the same the single-row reader returns.
    let refreshed = listed
        .iter()
        .find(|row| row.pull_request.identity == identity(URL))
        .unwrap();
    assert_eq!(
        refreshed.pull_request,
        store.pull_request(&identity(URL)).unwrap().unwrap()
    );
    assert_eq!(
        refreshed.pull_request.title.as_deref(),
        Some("Synthetic title 50")
    );

    // A row no session links any more is still reported, with a zero count,
    // so a caller can tell an unknown identifier from an unlinked one.
    drop(store);
    Connection::open(&path)
        .unwrap()
        .execute(
            "DELETE FROM pr_links WHERE pr_id=(SELECT id FROM pull_requests WHERE url=?1)",
            [identity(second).url()],
        )
        .unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .linked_pull_requests()
            .unwrap()
            .iter()
            .map(|row| row.linked_sessions)
            .collect::<Vec<_>>(),
        vec![1, 0, 2]
    );
}
