//! Skills added to Scores (migration 26): insert once by id and by location,
//! remove, list oldest first, the table's checks, and that removing a skill
//! keeps its grades. Synthetic disposable databases only.

use rusqlite::Connection;
use tempfile::TempDir;
use xt_store::{
    Store,
    added_skills::AddedSkill,
    grades::{GradeFinish, NewGradeRun},
};

fn added(id: &str, location: &str, at: i64) -> AddedSkill {
    AddedSkill {
        skill_id: id.into(),
        location: location.into(),
        name: format!("{id} grader"),
        description: "Grades it.".into(),
        rubric_version: "1.0 #0123456789ab".into(),
        added_at_ms: at,
    }
}

#[test]
fn migration_26_adds_an_empty_table() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path().join("x.sqlite")).unwrap();
    assert_eq!(store.schema_version().unwrap(), 27);
    assert!(store.added_skills().unwrap().is_empty());
}

#[test]
fn a_skill_is_added_once_by_id_and_by_location_and_removed_by_id() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(directory.path().join("x.sqlite")).unwrap();
    assert!(
        store
            .added_skill_insert(&added("b-check", "/h/b", 20))
            .unwrap()
    );
    assert!(
        store
            .added_skill_insert(&added("a-check", "/h/a", 20))
            .unwrap()
    );
    assert!(
        store
            .added_skill_insert(&added("c-check", "/h/c", 10))
            .unwrap()
    );
    // The same id from another folder, or another id from the same folder.
    assert!(
        !store
            .added_skill_insert(&added("b-check", "/h/other", 30))
            .unwrap()
    );
    assert!(
        !store
            .added_skill_insert(&added("d-check", "/h/b", 30))
            .unwrap()
    );
    // A plugin's skill is stored by its plugin and skill name, once.
    assert!(
        store
            .added_skill_insert(&added("p-check", "plugin:tools@market/lint", 40))
            .unwrap()
    );
    assert!(
        !store
            .added_skill_insert(&added("q-check", "plugin:tools@market/lint", 40))
            .unwrap()
    );
    assert!(store.added_skill_remove("p-check").unwrap());
    let ids: Vec<_> = store
        .added_skills()
        .unwrap()
        .into_iter()
        .map(|skill| skill.skill_id)
        .collect();
    assert_eq!(ids, ["c-check", "a-check", "b-check"]);
    assert_eq!(
        store.added_skills().unwrap()[1],
        added("a-check", "/h/a", 20)
    );
    assert!(store.added_skill_remove("a-check").unwrap());
    assert!(!store.added_skill_remove("a-check").unwrap());
    assert_eq!(store.added_skills().unwrap().len(), 2);
}

#[test]
fn removing_a_skill_keeps_its_grades() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::open(directory.path().join("x.sqlite")).unwrap();
    store
        .added_skill_insert(&added("a-check", "/h/a", 1))
        .unwrap();
    let run = "0f8fad5b-d9cb-469f-a165-70867728950e";
    store
        .grade_insert_running(&NewGradeRun {
            run_id: run,
            project_path: "/p",
            skill_id: "a-check",
            skill_version: "1.0 #0123456789ab",
            started_at_ms: 5,
        })
        .unwrap();
    store
        .grade_finish(
            run,
            &GradeFinish::Succeeded {
                finished_at_ms: 6,
                score: 80,
                complete: true,
                summary: "s",
                insights_json: "[]",
                report_file: None,
            },
        )
        .unwrap();
    assert!(store.added_skill_remove("a-check").unwrap());
    assert_eq!(store.grade(run).unwrap().unwrap().score, Some(80));
}

#[test]
fn the_table_refuses_rows_that_break_its_rules() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("x.sqlite");
    drop(Store::open(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    let insert = |id: &str, location: &str, name: &str, version: &str, at: i64| {
        connection.execute(
            "INSERT INTO grading_added_skills VALUES(?1,?2,?3,'d',?4,?5)",
            rusqlite::params![id, location, name, version, at],
        )
    };
    assert!(insert("ok", "/h/ok", "Ok", "1", 0).is_ok());
    assert!(insert("plug", "plugin:a@m/s", "P", "1", 0).is_ok());
    for (id, location, name, version, at) in [
        ("Upper", "/h/u", "U", "1", 0),
        ("", "/h/e", "E", "1", 0),
        ("rel", "h/rel", "R", "1", 0),
        ("plugnokey", "plugin:/s", "P", "1", 0),
        ("plugnoskill", "plugin:a@m/", "P", "1", 0),
        ("plugbare", "plugin:a@m", "P", "1", 0),
        ("noname", "/h/n", "", "1", 0),
        ("nover", "/h/v", "V", "", 0),
        ("neg", "/h/neg", "N", "1", -1),
    ] {
        assert!(insert(id, location, name, version, at).is_err(), "{id}");
    }
}
