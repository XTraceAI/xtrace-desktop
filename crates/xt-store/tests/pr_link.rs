//! Exact pull-request evidence identity and transactional link persistence.
//! Synthetic identities and disposable databases only; no network or Git.

use rusqlite::{Connection, types::Value as SqlValue};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use xt_store::{
    Error, SessionMeta, SessionSource, Store,
    pr_link::{MAX_URL_LEN, PrConfidence, PrIdentity, PrLinkObservation, PrLinkOutcome},
};

const URL: &str = "https://github.com/octo-org/hello.world/pull/42";

fn open(directory: &TempDir) -> (Store, PathBuf) {
    let path = directory.path().join("pr.sqlite");
    let mut store = Store::open(&path).unwrap();
    for id in ["session-a", "session-b"] {
        store
            .upsert_session(
                &SessionMeta::new(id, "claude", SessionSource::Fixture),
                false,
            )
            .unwrap();
    }
    (store, path)
}

fn identity(url: &str) -> PrIdentity {
    PrIdentity::from_url(url).unwrap()
}

fn observation(
    session: &str,
    url: &str,
    confidence: PrConfidence,
    first: i64,
    last: i64,
) -> PrLinkObservation {
    PrLinkObservation {
        session_id: session.into(),
        pull_request: identity(url),
        confidence,
        first_seen_at: first,
        last_seen_at: last,
    }
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

fn rejected(result: xt_store::Result<PrIdentity>) -> bool {
    matches!(result, Err(Error::InvalidInput(_)))
}

#[test]
fn canonical_urls_normalize_case_and_round_trip() {
    let canonical = identity(URL);
    assert_eq!(canonical.repository(), "octo-org/hello.world");
    assert_eq!(canonical.number(), 42);
    assert_eq!(canonical.url(), URL);
    for spelling in [
        "https://github.com/Octo-Org/Hello.World/pull/42",
        "HTTPS://GITHUB.COM/octo-org/hello.world/pull/42",
        "https://GitHub.com/OCTO-ORG/HELLO.WORLD/pull/42",
    ] {
        assert_eq!(identity(spelling), canonical, "{spelling}");
    }
    assert_eq!(
        PrIdentity::from_parts("OCTO-org/hello.WORLD", 42).unwrap(),
        canonical
    );
    // Distinct repositories and numbers stay distinct after normalization.
    assert_ne!(
        identity("https://github.com/octo-org/hello-world/pull/42"),
        canonical
    );
    assert_ne!(
        identity("https://github.com/octo-org/hello.world/pull/43"),
        canonical
    );
    let largest = format!("https://github.com/o/r/pull/{}", i64::MAX);
    assert_eq!(identity(&largest).number(), i64::MAX as u64);
    let owner = "a".repeat(39);
    let repo = "b".repeat(100);
    let longest = format!("https://github.com/{owner}/{repo}/pull/{}", i64::MAX);
    assert!(longest.len() <= MAX_URL_LEN);
    assert_eq!(identity(&longest).url(), longest);
}

#[test]
fn unsupported_or_malformed_urls_are_rejected() {
    let oversized = format!("https://github.com/o/{}/pull/1", "r".repeat(MAX_URL_LEN));
    let long_owner = format!("https://github.com/{}/r/pull/1", "o".repeat(40));
    let long_repo = format!("https://github.com/o/{}/pull/1", "r".repeat(101));
    let cases = [
        // Credentials, ports and other hosts or schemes.
        "https://user:secret@github.com/o/r/pull/1",
        "https://user@github.com/o/r/pull/1",
        "https://github.com:443/o/r/pull/1",
        "https://github.com.evil.example/o/r/pull/1",
        "https://www.github.com/o/r/pull/1",
        "https://api.github.com/repos/o/r/pulls/1",
        "https://ghe.example.com/o/r/pull/1",
        "http://github.com/o/r/pull/1",
        "ssh://github.com/o/r/pull/1",
        "git@github.com:o/r/pull/1",
        "//github.com/o/r/pull/1",
        "github.com/o/r/pull/1",
        " https://github.com/o/r/pull/1",
        // Queries, fragments and extra or missing segments.
        "https://github.com/o/r/pull/1?diff=split",
        "https://github.com/o/r/pull/1#discussion",
        "https://github.com/o/r/pull/1/",
        "https://github.com/o/r/pull/1/files",
        "https://github.com/o/r/pull/",
        "https://github.com/o/r/pull",
        "https://github.com/o/r/pulls/1",
        "https://github.com/o/r/issues/1",
        "https://github.com/o/r/PULL/1",
        "https://github.com/o/pull/1",
        "https://github.com//r/pull/1",
        "https://github.com/o//pull/1",
        "https://github.com/o/r/x/pull/1",
        "https://github.com/",
        "",
        // Dot traversal, escapes and alias spellings.
        "https://github.com/o/../pull/1",
        "https://github.com/o/./pull/1",
        "https://github.com/../r/pull/1",
        "https://github.com/o/%2e%2e/pull/1",
        "https://github.com/o/r%2F/pull/1",
        "https://github.com/o/r.git/pull/1",
        "https://github.com/o/r.GIT/pull/1",
        "https://github.com/-o/r/pull/1",
        "https://github.com/o_o/r/pull/1",
        // Numbers.
        "https://github.com/o/r/pull/0",
        "https://github.com/o/r/pull/042",
        "https://github.com/o/r/pull/-1",
        "https://github.com/o/r/pull/+1",
        "https://github.com/o/r/pull/1.0",
        "https://github.com/o/r/pull/1e3",
        "https://github.com/o/r/pull/9223372036854775808",
        "https://github.com/o/r/pull/99999999999999999999",
        // Control, whitespace and non-ASCII identity components.
        "https://github.com/o/r\u{0}/pull/1",
        "https://github.com/o/r\n/pull/1",
        "https://github.com/o/r /pull/1",
        "https://github.com/o/r/pull/1\t",
        "https://github.com/\u{f6}/r/pull/1",
        "https://github.com/o/r\u{e9}/pull/1",
        "https://github.com/o/r/pull/\u{661}",
        "https://github.com/o/r\\x/pull/1",
    ];
    for url in cases {
        assert!(rejected(PrIdentity::from_url(url)), "{url:?}");
    }
    for url in [&oversized, &long_owner, &long_repo] {
        assert!(rejected(PrIdentity::from_url(url)), "{url}");
    }
    for repository in [
        "",
        "o",
        "o/",
        "/r",
        "o/r/x",
        "o/r.git",
        "o/..",
        "https://github.com/o/r",
        "o/r ",
        "o\u{0}/r",
        "\u{f6}/r",
    ] {
        assert!(
            rejected(PrIdentity::from_parts(repository, 1)),
            "{repository:?}"
        );
    }
    assert!(rejected(PrIdentity::from_parts(&"o/".repeat(200), 1)));
    for number in [0, i64::MAX as u64 + 1, u64::MAX] {
        assert!(rejected(PrIdentity::from_parts("o/r", number)), "{number}");
    }
}

#[test]
fn supplied_url_repository_and_number_must_reconcile() {
    let canonical = identity(URL);
    for (url, repository, number) in [
        (Some(URL), None, None),
        (Some(URL), Some("octo-org/hello.world"), Some(42)),
        (Some(URL), Some("Octo-Org/Hello.World"), None),
        (
            Some("https://github.com/OCTO-ORG/hello.world/pull/42"),
            None,
            Some(42),
        ),
        (None, Some("octo-org/HELLO.world"), Some(42)),
    ] {
        assert_eq!(
            PrIdentity::reconcile(url, repository, number).unwrap(),
            canonical,
            "{url:?} {repository:?} {number:?}"
        );
    }
    for (url, repository, number) in [
        (Some(URL), Some("octo-org/hello-world"), Some(42)),
        (Some(URL), Some("other/hello.world"), None),
        (Some(URL), None, Some(43)),
        (Some(URL), Some("octo-org/hello.world"), Some(0)),
        (Some(URL), Some("octo-org/hello.world.git"), Some(42)),
        (
            Some("https://github.com/octo-org/hello.world/pull/042"),
            None,
            Some(42),
        ),
        (None, Some("octo-org/hello.world"), None),
        (None, None, Some(42)),
        (None, None, None),
    ] {
        assert!(
            rejected(PrIdentity::reconcile(url, repository, number)),
            "{url:?} {repository:?} {number:?}"
        );
    }
}

#[test]
fn one_stub_and_one_link_per_canonical_identity() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory);
    let first = store
        .record_pr_link(&observation("session-a", URL, PrConfidence::Exact, 10, 20))
        .unwrap();
    assert_eq!(
        first,
        PrLinkOutcome {
            pull_request_id: first.pull_request_id,
            pull_request_created: true,
            link_created: true,
            link_changed: true,
        }
    );
    // Case-variant spellings resolve to the same stub and link.
    let variant = "https://GitHub.com/OCTO-ORG/Hello.World/pull/42";
    let again = store
        .record_pr_link(&observation(
            "session-a",
            variant,
            PrConfidence::Exact,
            10,
            20,
        ))
        .unwrap();
    assert_eq!(
        again,
        PrLinkOutcome {
            pull_request_id: first.pull_request_id,
            pull_request_created: false,
            link_created: false,
            link_changed: false,
        }
    );
    let other_session = store
        .record_pr_link(&observation("session-b", variant, PrConfidence::Sha, 5, 5))
        .unwrap();
    assert_eq!(other_session.pull_request_id, first.pull_request_id);
    assert!(!other_session.pull_request_created && other_session.link_created);
    assert_eq!(
        rows(&path, "SELECT repo, number, url FROM pull_requests"),
        vec![vec![
            SqlValue::Text("octo-org/hello.world".into()),
            SqlValue::Integer(42),
            SqlValue::Text(URL.into()),
        ]]
    );
    assert_eq!(
        rows(&path, "SELECT count(*) FROM pr_links"),
        vec![vec![SqlValue::Integer(2)]]
    );
}

#[test]
fn duplicate_and_reversed_arrivals_converge_on_the_widest_interval() {
    let arrivals = [
        observation("session-a", URL, PrConfidence::Exact, 300, 400),
        observation("session-a", URL, PrConfidence::Exact, 100, 150),
        observation("session-a", URL, PrConfidence::Exact, 300, 400),
        observation("session-a", URL, PrConfidence::Exact, 200, 500),
    ];
    let mut finals = Vec::new();
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2]] {
        let directory = TempDir::new().unwrap();
        let (mut store, _) = open(&directory);
        for index in order {
            store.record_pr_link(&arrivals[index]).unwrap();
        }
        let links = store.session_pr_links("session-a").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].first_seen_at, links[0].last_seen_at), (100, 500));
        // An identical replay changes nothing.
        let replay = store.record_pr_link(&arrivals[order[0]]).unwrap();
        assert!(!replay.link_changed && !replay.link_created);
        finals.push(links);
    }
    assert!(finals.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn confidence_upgrades_but_never_downgrades() {
    use PrConfidence::{Exact, Inferred, Sha};
    assert_eq!(Inferred.strongest(Sha), Sha);
    assert_eq!(Sha.strongest(Exact), Exact);
    assert_eq!(Exact.strongest(Inferred), Exact);
    assert_eq!(Exact.strongest(Sha), Exact);
    for (sequence, expected) in [
        (vec![Inferred, Sha, Exact], vec![Inferred, Sha, Exact]),
        (vec![Exact, Sha, Inferred], vec![Exact, Exact, Exact]),
        (
            vec![Sha, Inferred, Exact, Sha],
            vec![Sha, Sha, Exact, Exact],
        ),
    ] {
        let directory = TempDir::new().unwrap();
        let (mut store, _) = open(&directory);
        let mut previous = None;
        for (confidence, expected) in sequence.into_iter().zip(expected) {
            let outcome = store
                .record_pr_link(&observation("session-a", URL, confidence, 1, 1))
                .unwrap();
            let stored = store.session_pr_links("session-a").unwrap()[0].confidence;
            assert_eq!(stored, expected);
            // Same interval every time, so only a confidence upgrade changes it.
            assert_eq!(outcome.link_changed, previous != Some(stored));
            previous = Some(stored);
        }
    }
}

#[test]
fn invalid_observations_and_missing_sessions_write_nothing() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory);
    let before = snapshot(&path);
    for invalid in [
        observation("missing-session", URL, PrConfidence::Exact, 1, 2),
        observation("", URL, PrConfidence::Exact, 1, 2),
        observation("   ", URL, PrConfidence::Exact, 1, 2),
        observation("session-a", URL, PrConfidence::Exact, 3, 2),
    ] {
        assert!(matches!(
            store.record_pr_link(&invalid),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(snapshot(&path), before);
    }
}

#[test]
fn late_failures_and_conflicting_rows_roll_back_stub_and_link_together() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory);
    store
        .record_pr_link(&observation(
            "session-a",
            URL,
            PrConfidence::Inferred,
            10,
            20,
        ))
        .unwrap();
    let raw = Connection::open(&path).unwrap();
    // A link write failing after its new stub was inserted leaves no stub.
    raw.execute_batch(
        "CREATE TRIGGER inject_failure BEFORE INSERT ON pr_links
         BEGIN SELECT RAISE(ABORT,'synthetic link failure'); END;",
    )
    .unwrap();
    let before = snapshot(&path);
    let fresh = "https://github.com/octo-org/fresh/pull/7";
    assert!(matches!(
        store.record_pr_link(&observation("session-a", fresh, PrConfidence::Exact, 1, 2)),
        Err(Error::Sqlite(_))
    ));
    assert_eq!(snapshot(&path), before);
    assert!(store.pull_request(&identity(fresh)).unwrap().is_none());
    raw.execute_batch("DROP TRIGGER inject_failure").unwrap();

    // Existing rows that disagree with a canonical identity are conflicts.
    raw.execute_batch(
        "INSERT INTO pull_requests(repo,number,url)
             VALUES ('octo-org/alias',9,'https://github.com/octo-org/elsewhere/pull/9');
         INSERT INTO pull_requests(repo,number,url)
             VALUES ('octo-org/legacy',5,'https://github.com/Octo-Org/Legacy/pull/5');
         INSERT INTO pull_requests(repo,number,url)
             VALUES ('octo-org/split',3,'https://github.com/octo-org/split/pull/4');
         INSERT INTO pull_requests(repo,number,url)
             VALUES ('Octo-Org/Upper',6,'https://github.com/Octo-Org/Upper/pull/6');",
    )
    .unwrap();
    let before = snapshot(&path);
    for conflicting in [
        "https://github.com/octo-org/elsewhere/pull/9",
        "https://github.com/octo-org/alias/pull/9",
        "https://github.com/octo-org/legacy/pull/5",
        "https://github.com/octo-org/split/pull/3",
        "https://github.com/octo-org/split/pull/4",
        "https://github.com/octo-org/upper/pull/6",
    ] {
        for session in ["session-a", "session-b"] {
            assert!(
                matches!(
                    store.record_pr_link(&observation(
                        session,
                        conflicting,
                        PrConfidence::Exact,
                        0,
                        99
                    )),
                    Err(Error::InvalidInput(_))
                ),
                "{conflicting}"
            );
            assert_eq!(snapshot(&path), before, "{conflicting}");
        }
    }
    // The pre-existing canonical link kept its interval and confidence.
    let links = store.session_pr_links("session-a").unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].confidence, PrConfidence::Inferred);
    assert_eq!((links[0].first_seen_at, links[0].last_seen_at), (10, 20));
    // Readers report a non-canonical stored identity instead of re-spelling it.
    assert!(store.all_pr_links().is_ok());
    raw.execute(
        "INSERT INTO pr_links VALUES ('session-b',(SELECT id FROM pull_requests WHERE repo='octo-org/legacy'),'exact',1,1)",
        [],
    )
    .unwrap();
    assert!(matches!(
        store.session_pr_links("session-b"),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn links_persist_across_reopen_and_read_in_deterministic_order() {
    let directory = TempDir::new().unwrap();
    let (mut store, _) = open(&directory);
    for (session, url) in [
        ("session-b", "https://github.com/zeta/app/pull/2"),
        ("session-a", "https://github.com/zeta/app/pull/10"),
        ("session-b", "https://github.com/alpha/app/pull/3"),
        ("session-a", "https://github.com/zeta/app/pull/2"),
        ("session-a", "https://github.com/alpha/app/pull/3"),
    ] {
        store
            .record_pr_link(&observation(session, url, PrConfidence::Exact, 1, 2))
            .unwrap();
    }
    let version = store.schema_version().unwrap();
    drop(store);
    let store = Store::open(directory.path().join("pr.sqlite")).unwrap();
    assert_eq!(store.schema_version().unwrap(), version);
    let names = |links: Vec<xt_store::pr_link::StoredPrLink>| {
        links
            .into_iter()
            .map(|link| format!("{}:{}", link.session_id, link.pull_request.url()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(store.session_pr_links("session-a").unwrap()),
        [
            "session-a:https://github.com/alpha/app/pull/3",
            "session-a:https://github.com/zeta/app/pull/2",
            "session-a:https://github.com/zeta/app/pull/10",
        ]
    );
    assert_eq!(
        names(
            store
                .pull_request_links(&identity("https://github.com/zeta/app/pull/2"))
                .unwrap()
        ),
        [
            "session-a:https://github.com/zeta/app/pull/2",
            "session-b:https://github.com/zeta/app/pull/2",
        ]
    );
    assert_eq!(
        names(store.all_pr_links().unwrap()),
        [
            "session-a:https://github.com/alpha/app/pull/3",
            "session-b:https://github.com/alpha/app/pull/3",
            "session-a:https://github.com/zeta/app/pull/2",
            "session-b:https://github.com/zeta/app/pull/2",
            "session-a:https://github.com/zeta/app/pull/10",
        ]
    );
    assert!(store.session_pr_links("unknown").unwrap().is_empty());
    assert!(
        store
            .pull_request(&identity("https://github.com/alpha/app/pull/4"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn exact_links_leave_merge_and_refresh_state_unknown_and_store_no_content() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory);
    let counts = |path: &Path| {
        rows(
            path,
            "SELECT (SELECT count(*) FROM records),(SELECT count(*) FROM tool_uses),
                    (SELECT count(*) FROM capture_receipts),(SELECT count(*) FROM settings),
                    (SELECT group_concat(coalesce(title,'')||coalesce(cwd,'')||coalesce(repo,'')) FROM sessions)",
        )
    };
    let before = counts(&path);
    store
        .record_pr_link(&observation("session-a", URL, PrConfidence::Exact, 1, 2))
        .unwrap();
    let stored = store.pull_request(&identity(URL)).unwrap().unwrap();
    assert_eq!(stored.identity, identity(URL));
    assert_eq!(stored.state, None);
    assert_eq!(stored.merged_at, None);
    assert_eq!(stored.additions, None);
    assert_eq!(stored.deletions, None);
    assert_eq!(stored.refreshed_at, None);
    assert_eq!(
        rows(
            &path,
            "SELECT title,state,merged_at,additions,deletions,head_ref_name,refreshed_at FROM pull_requests"
        ),
        vec![vec![SqlValue::Null; 7]]
    );
    // Only identity, confidence and times are stored; no other table changes.
    assert_eq!(
        rows(&path, "SELECT * FROM pr_links"),
        vec![vec![
            SqlValue::Text("session-a".into()),
            SqlValue::Integer(stored.id),
            SqlValue::Text("exact".into()),
            SqlValue::Integer(1),
            SqlValue::Integer(2),
        ]]
    );
    assert_eq!(counts(&path), before);
}

#[test]
fn identity_readers_report_legacy_conflicts_instead_of_absence() {
    let directory = TempDir::new().unwrap();
    let (mut store, path) = open(&directory);
    let clean = "https://github.com/octo-org/clean/pull/1";
    store
        .record_pr_link(&observation("session-b", clean, PrConfidence::Sha, 3, 4))
        .unwrap();
    // Legacy rows written outside the typed writer, each with a link.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "INSERT INTO pull_requests(repo,number,url) VALUES
             ('Octo-Org/Upper',6,'https://github.com/Octo-Org/Upper/pull/6'),
             ('octo-org/legacy',5,'https://github.com/Octo-Org/Legacy/pull/5'),
             ('octo-org/alias',9,'https://github.com/octo-org/elsewhere/pull/9'),
             ('octo-org/split',3,'https://github.com/octo-org/split/pull/4'),
             ('octo-org/multi',2,'https://github.com/octo-org/multi/pull/2'),
             ('Octo-Org/Multi',2,'https://github.com/Octo-Org/Multi/pull/2');
         INSERT INTO pr_links SELECT 'session-a', id, 'exact', 1, 2 FROM pull_requests
             WHERE repo != 'octo-org/clean';",
    )
    .unwrap();
    let before = snapshot(&path);
    for conflicting in [
        // Case variants of repository and/or URL.
        "https://github.com/octo-org/upper/pull/6",
        "https://github.com/octo-org/legacy/pull/5",
        // URL alias: the URL names one identity, the row another.
        "https://github.com/octo-org/elsewhere/pull/9",
        "https://github.com/octo-org/alias/pull/9",
        // Split: repository/number and URL disagree.
        "https://github.com/octo-org/split/pull/3",
        "https://github.com/octo-org/split/pull/4",
        // Several candidates, even though one is exactly canonical.
        "https://github.com/octo-org/multi/pull/2",
    ] {
        let identity = identity(conflicting);
        assert!(
            matches!(store.pull_request(&identity), Err(Error::InvalidInput(_))),
            "{conflicting}"
        );
        assert!(
            matches!(
                store.pull_request_links(&identity),
                Err(Error::InvalidInput(_))
            ),
            "{conflicting}"
        );
        // The writer resolves the same candidates to the same conflict.
        assert!(
            matches!(
                store.record_pr_link(&observation(
                    "session-b",
                    conflicting,
                    PrConfidence::Exact,
                    1,
                    2
                )),
                Err(Error::InvalidInput(_))
            ),
            "{conflicting}"
        );
    }
    // A unique canonical row still reads normally, and no candidate is absent.
    let stored = store.pull_request(&identity(clean)).unwrap().unwrap();
    assert_eq!(stored.identity, identity(clean));
    let links = store.pull_request_links(&identity(clean)).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].session_id, "session-b");
    assert_eq!(links[0].confidence, PrConfidence::Sha);
    let absent = identity("https://github.com/octo-org/absent/pull/6");
    assert!(store.pull_request(&absent).unwrap().is_none());
    assert!(store.pull_request_links(&absent).unwrap().is_empty());
    // Reads never normalize or rewrite legacy rows.
    assert_eq!(snapshot(&path), before);
}
