//! Grading runs (migration 25): the typed API, the table's checks, the
//! running-to-finished transition, the newest run per (project, skill) and
//! the content retention registration. Synthetic disposable databases only.

use rusqlite::Connection;
use tempfile::TempDir;
use xt_store::{
    Store,
    grades::{CONTENT_COLUMNS, GradeFinish, GradeStatus, NewGradeRun},
    retention::{ContentRegistry, RetentionMode},
};

const A: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const B: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
const C: &str = "11111111-2222-4333-8444-555555555555";
const D: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

fn open(directory: &TempDir) -> Store {
    Store::open(directory.path().join("grades.sqlite")).unwrap()
}

fn run<'a>(run_id: &'a str, project: &'a str, started_at_ms: i64) -> NewGradeRun<'a> {
    NewGradeRun {
        run_id,
        project_path: project,
        skill_id: "claude-md-doctor",
        skill_version: "0.5.1+xtrace.1",
        started_at_ms,
    }
}

fn succeeded(finished_at_ms: i64) -> GradeFinish<'static> {
    with(finished_at_ms, 75, INSIGHTS, Some("report.html"))
}

const INSIGHTS: &str = r#"[{"severity":"critical","title":"Dead path"}]"#;

fn with(
    finished_at_ms: i64,
    score: u8,
    insights_json: &'static str,
    report_file: Option<&'static str>,
) -> GradeFinish<'static> {
    GradeFinish::Succeeded {
        finished_at_ms,
        score,
        complete: true,
        summary: "One dead reference.",
        insights_json,
        report_file,
    }
}

#[test]
fn migration_25_adds_the_grades_table() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    assert_eq!(store.schema_version().unwrap(), 27);
    assert!(store.grades(10).unwrap().is_empty());
}

#[test]
fn a_succeeded_grade_survives_closing_and_reopening_version_25() {
    let directory = TempDir::new().unwrap();
    let saved = {
        let mut store = open(&directory);
        store.grade_insert_running(&run(A, "/p", 1_000)).unwrap();
        assert!(store.grade_finish(A, &succeeded(2_000)).unwrap());
        store.grade(A).unwrap().unwrap()
    };

    let reopened = open(&directory);
    // Reopened at the current version: migration 26 (skills added to
    // Scores) leaves the grades table as migration 25 made it.
    assert_eq!(reopened.schema_version().unwrap(), 27);
    let restored = reopened.grade(A).unwrap().unwrap();
    assert_eq!(restored, saved);
    assert_eq!(restored.status, GradeStatus::Succeeded);
    assert_eq!(restored.finished_at_ms, Some(2_000));
    assert_eq!(restored.score, Some(75));
    assert_eq!(restored.complete, Some(true));
    assert_eq!(restored.summary.as_deref(), Some("One dead reference."));
    assert_eq!(restored.insights_json.as_deref(), Some(INSIGHTS));
    assert_eq!(restored.report_file.as_deref(), Some("report.html"));
    assert_eq!(restored.failure_reason, None);
}

#[test]
fn a_run_is_inserted_running_and_finished_exactly_once() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    store.grade_insert_running(&run(A, "/p", 1_000)).unwrap();
    let row = store.grade(A).unwrap().unwrap();
    assert_eq!(row.status, GradeStatus::Running);
    assert_eq!(
        (row.finished_at_ms, row.score, row.complete),
        (None, None, None)
    );
    assert!(store.grade_finish(A, &succeeded(2_000)).unwrap());
    let done = store.grade(A).unwrap().unwrap();
    assert_eq!(done.status, GradeStatus::Succeeded);
    assert_eq!(done.finished_at_ms, Some(2_000));
    assert_eq!(done.score, Some(75));
    assert_eq!(done.complete, Some(true));
    assert_eq!(done.summary.as_deref(), Some("One dead reference."));
    assert_eq!(done.report_file.as_deref(), Some("report.html"));
    assert_eq!(done.failure_reason, None);
    // A second finish of any kind changes nothing.
    for again in [
        GradeFinish::Failed {
            finished_at_ms: 3_000,
            reason: "timeout",
        },
        GradeFinish::Cancelled {
            finished_at_ms: 3_000,
        },
        succeeded(3_000),
    ] {
        assert!(!store.grade_finish(A, &again).unwrap());
        assert_eq!(store.grade(A).unwrap().unwrap(), done);
    }
    // An unknown run does not move either.
    assert!(!store.grade_finish(B, &succeeded(3_000)).unwrap());
    assert!(store.grade(B).unwrap().is_none());
}

#[test]
fn each_terminal_status_stores_only_its_own_fields() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    let cases = [
        (
            A,
            GradeFinish::Failed {
                finished_at_ms: 5,
                reason: "result_missing",
            },
            GradeStatus::Failed,
            Some("result_missing"),
        ),
        (
            B,
            GradeFinish::Cancelled { finished_at_ms: 5 },
            GradeStatus::Cancelled,
            None,
        ),
        (
            C,
            GradeFinish::Interrupted {
                finished_at_ms: 5,
                reason: "app_closed",
            },
            GradeStatus::Interrupted,
            Some("app_closed"),
        ),
    ];
    for (id, finish, status, reason) in cases {
        store.grade_insert_running(&run(id, "/p", 1)).unwrap();
        assert!(store.grade_finish(id, &finish).unwrap());
        let row = store.grade(id).unwrap().unwrap();
        assert_eq!(row.status, status);
        assert_eq!(row.failure_reason.as_deref(), reason);
        assert_eq!(row.finished_at_ms, Some(5));
        assert_eq!(
            (
                row.score,
                row.complete,
                row.summary,
                row.insights_json,
                row.report_file
            ),
            (None, None, None, None, None)
        );
    }
}

#[test]
fn a_finish_before_the_start_is_stored_at_the_start() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for (id, finish) in [
        (A, succeeded(9)),
        (
            B,
            GradeFinish::Failed {
                finished_at_ms: 9,
                reason: "timeout",
            },
        ),
        (C, GradeFinish::Cancelled { finished_at_ms: 9 }),
        (
            D,
            GradeFinish::Interrupted {
                finished_at_ms: 9,
                reason: "app_closed",
            },
        ),
    ] {
        store.grade_insert_running(&run(id, "/p", 10)).unwrap();
        assert!(store.grade_finish(id, &finish).unwrap(), "{id}");
        let row = store.grade(id).unwrap().unwrap();
        assert_eq!(row.finished_at_ms, Some(10), "{id}");
        assert_ne!(row.status, GradeStatus::Running, "{id}");
    }
}

#[test]
fn only_one_run_can_be_running() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    store.grade_insert_running(&run(A, "/p", 1)).unwrap();
    assert!(store.grade_insert_running(&run(B, "/q", 2)).is_err());
    // Nor can an ID be reused.
    store
        .grade_finish(A, &GradeFinish::Cancelled { finished_at_ms: 2 })
        .unwrap();
    assert!(store.grade_insert_running(&run(A, "/q", 3)).is_err());
    store.grade_insert_running(&run(B, "/q", 3)).unwrap();
}

#[test]
fn the_api_refuses_values_the_checks_forbid() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for bad in [
        run("not-a-uuid", "/p", 1),
        run("0F8FAD5B-D9CB-469F-A165-70867728950E", "/p", 1),
        run(A, "relative/path", 1),
        run(A, "/p", -1),
        NewGradeRun {
            skill_id: "Bad Skill",
            ..run(A, "/p", 1)
        },
        NewGradeRun {
            skill_version: "",
            ..run(A, "/p", 1)
        },
    ] {
        assert!(store.grade_insert_running(&bad).is_err(), "{bad:?}");
    }
    store.grade_insert_running(&run(A, "/p", 10)).unwrap();
    for bad in [
        with(20, 101, INSIGHTS, None),
        with(20, 75, r#"{"not":"a list"}"#, None),
        with(20, 75, "not json", None),
        with(20, 75, INSIGHTS, Some("/abs/report.html")),
        with(20, 75, INSIGHTS, Some("../report.html")),
        GradeFinish::Failed {
            finished_at_ms: 20,
            reason: "/Users/someone/secret",
        },
        GradeFinish::Failed {
            finished_at_ms: 20,
            reason: "",
        },
        GradeFinish::Interrupted {
            finished_at_ms: 20,
            reason: "App Closed",
        },
    ] {
        assert!(store.grade_finish(A, &bad).is_err(), "{bad:?}");
        assert_eq!(
            store.grade(A).unwrap().unwrap().status,
            GradeStatus::Running,
            "{bad:?}"
        );
    }
}

#[test]
fn the_table_checks_keep_status_and_fields_consistent() {
    let directory = TempDir::new().unwrap();
    drop(open(&directory));
    let sql = Connection::open(directory.path().join("grades.sqlite")).unwrap();
    let base = "run_id,project_path,skill_id,skill_version,started_at";
    let values = format!("'{A}','/p','s','1',1");
    for (columns, extra) in [
        // A running row has no finish, result or reason.
        (",status,finished_at", ",'running',2"),
        (",status,score", ",'running',5"),
        (",status,failure_reason", ",'running','x'"),
        // A succeeded row has a finish, a score and a completeness flag.
        (",status,finished_at,complete", ",'succeeded',2,1"),
        (",status,finished_at,score", ",'succeeded',2,5"),
        (",status,score,complete", ",'succeeded',5,1"),
        (
            ",status,finished_at,score,complete,failure_reason",
            ",'succeeded',2,5,1,'x'",
        ),
        (",status,finished_at,score,complete", ",'succeeded',2,5,2"),
        (",status,finished_at,score,complete", ",'succeeded',2,5.5,1"),
        // A failed or interrupted row has a reason and no result.
        (",status,finished_at", ",'failed',2"),
        (",status,finished_at", ",'interrupted',2"),
        (
            ",status,finished_at,failure_reason,score",
            ",'failed',2,'x',5",
        ),
        (
            ",status,finished_at,failure_reason,summary",
            ",'interrupted',2,'x','s'",
        ),
        // A cancelled row has neither.
        (",status,finished_at,failure_reason", ",'cancelled',2,'x'"),
        (",status,finished_at,report_file", ",'cancelled',2,'r.html'"),
        // An unknown status.
        (",status", ",'done'"),
    ] {
        let statement = format!("INSERT INTO grades({base}{columns}) VALUES({values}{extra})");
        assert!(sql.execute(&statement, []).is_err(), "{statement}");
    }
    // A purged succeeded row (no summary, insights or report) is valid.
    sql.execute(
        &format!(
            "INSERT INTO grades({base},status,finished_at,score,complete) \
             VALUES({values},'succeeded',2,40,0)"
        ),
        [],
    )
    .unwrap();
}

#[test]
fn leftover_running_rows_are_marked_interrupted() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    store.grade_insert_running(&run(A, "/p", 1_000)).unwrap();
    store.grade_finish(A, &succeeded(2_000)).unwrap();
    store.grade_insert_running(&run(B, "/p", 5_000)).unwrap();
    // A clock behind the start finishes the row at its start.
    assert_eq!(
        store.grades_interrupt_running(4_000, "app_closed").unwrap(),
        [B]
    );
    let row = store.grade(B).unwrap().unwrap();
    assert_eq!(row.status, GradeStatus::Interrupted);
    assert_eq!(row.failure_reason.as_deref(), Some("app_closed"));
    assert_eq!(row.finished_at_ms, Some(5_000));
    assert_eq!(
        store.grade(A).unwrap().unwrap().status,
        GradeStatus::Succeeded
    );
    assert!(
        store
            .grades_interrupt_running(6_000, "app_closed")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn lists_are_newest_first_bounded_and_one_per_pair() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    for (id, project, at) in [(A, "/p", 1), (B, "/q", 2), (C, "/p", 3), (D, "/q", 3)] {
        store.grade_insert_running(&run(id, project, at)).unwrap();
        store.grade_finish(id, &succeeded(at + 1)).unwrap();
    }
    let ids = |rows: Vec<xt_store::grades::GradeRow>| {
        rows.into_iter().map(|row| row.run_id).collect::<Vec<_>>()
    };
    // Equal starts order by ID, descending.
    assert_eq!(ids(store.grades(10).unwrap()), [D, C, B, A]);
    assert_eq!(ids(store.grades(2).unwrap()), [D, C]);
    assert_eq!(ids(store.grades_latest_per_pair(10).unwrap()), [D, C]);
    assert_eq!(ids(store.grades_latest_per_pair(1).unwrap()), [D]);
    // Another skill on the same project is its own pair.
    store
        .grade_insert_running(&NewGradeRun {
            skill_id: "other-skill",
            ..run("22222222-3333-4444-8555-666666666666", "/p", 0)
        })
        .unwrap();
    assert_eq!(store.grades_latest_per_pair(10).unwrap().len(), 3);
    assert_eq!(store.grade_newest_succeeded_ids(3).unwrap(), [D, C, B]);
}

#[test]
fn grade_results_are_registered_content_and_purge_clears_only_them() {
    assert!(
        ContentRegistry::default()
            .columns()
            .filter(|(table, _)| *table == "grades")
            .map(|(_, column)| column)
            .eq(CONTENT_COLUMNS
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>())
    );
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    // Kept whatever the retention mode: the user asked for this grade.
    store
        .set_retention_mode(RetentionMode::MetadataOnly)
        .unwrap();
    store.grade_insert_running(&run(A, "/p", 1)).unwrap();
    store.grade_finish(A, &succeeded(2)).unwrap();
    let before = store.grade(A).unwrap().unwrap();
    assert!(before.summary.is_some());
    let outcome = store.purge_content(&ContentRegistry::default()).unwrap();
    assert!(outcome.invalidate_content);
    assert!(
        outcome
            .tables
            .iter()
            .any(|table| table.table == "grades" && table.rows == 1)
    );
    let after = store.grade(A).unwrap().unwrap();
    assert_eq!(
        (&after.summary, &after.insights_json, &after.report_file),
        (&None, &None, &None)
    );
    assert_eq!(after.score, before.score);
    assert_eq!(after.complete, before.complete);
    assert_eq!(after.status, GradeStatus::Succeeded);
    assert_eq!(after.project_path, before.project_path);
    // A second purge clears nothing.
    let again = store.purge_content(&ContentRegistry::default()).unwrap();
    assert!(
        again
            .tables
            .iter()
            .any(|table| table.table == "grades" && table.rows == 0)
    );
}

#[test]
fn each_pair_reads_its_newest_run_and_newest_success_in_one_snapshot() {
    const E: &str = "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff";
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory);
    // /p: succeeded, then a newer failure. /q: two successes, the newer
    // one shown. /r: only a running run. /s: nothing.
    store.grade_insert_running(&run(A, "/p", 1_000)).unwrap();
    store.grade_finish(A, &succeeded(1_500)).unwrap();
    store.grade_insert_running(&run(B, "/p", 2_000)).unwrap();
    store
        .grade_finish(
            B,
            &GradeFinish::Failed {
                finished_at_ms: 2_500,
                reason: "timeout",
            },
        )
        .unwrap();
    store.grade_insert_running(&run(C, "/q", 3_000)).unwrap();
    store.grade_finish(C, &with(3_500, 40, "[]", None)).unwrap();
    store.grade_insert_running(&run(D, "/q", 4_000)).unwrap();
    store.grade_finish(D, &with(4_500, 90, "[]", None)).unwrap();
    store.grade_insert_running(&run(E, "/r", 5_000)).unwrap();
    let read = store
        .grade_pairs(&[
            ("/p", "claude-md-doctor"),
            ("/q", "claude-md-doctor"),
            ("/r", "claude-md-doctor"),
            ("/s", "claude-md-doctor"),
            ("/p", "another-skill"),
        ])
        .unwrap();
    fn ids(pair: &xt_store::grades::GradePairRuns) -> (Option<&str>, Option<&str>) {
        (
            pair.newest.as_ref().map(|row| row.run_id.as_str()),
            pair.newest_succeeded
                .as_ref()
                .map(|row| row.run_id.as_str()),
        )
    }
    assert_eq!(
        read.pairs.iter().map(ids).collect::<Vec<_>>(),
        [
            (Some(B), Some(A)),
            (Some(D), Some(D)),
            (Some(E), None),
            (None, None),
            (None, None),
        ]
    );
    assert_eq!(
        read.pairs[1].newest_succeeded.as_ref().unwrap().score,
        Some(90)
    );
    assert_eq!(
        read.running.as_ref().map(|row| row.run_id.as_str()),
        Some(E)
    );
    store
        .grade_finish(
            E,
            &GradeFinish::Cancelled {
                finished_at_ms: 6_000,
            },
        )
        .unwrap();
    assert_eq!(store.grade_pairs(&[]).unwrap().running, None);
    let many = vec![("/p", "claude-md-doctor"); xt_store::grades::MAX_PAIRS + 1];
    assert!(store.grade_pairs(&many).is_err());
}
