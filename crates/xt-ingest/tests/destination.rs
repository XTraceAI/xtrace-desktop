//! The index destination check: inside or aliased to native history is
//! refused before anything is written; a source entry the process cannot
//! read is not a reason to refuse (the scan reports it), so an unreadable
//! project directory neither blocks the CLI nor the app's startup.
#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use xt_ingest::native::validate_index_destination;

fn home_with_project(root: &Path) -> std::path::PathBuf {
    let home = root.join("home");
    fs::create_dir_all(home.join(".claude/projects/-repo-a")).unwrap();
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    home
}

#[test]
fn an_unreadable_source_directory_does_not_refuse_a_destination_beside_the_home() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with_project(temp.path());
    let sealed = home.join(".claude/projects/-repo-sealed");
    fs::create_dir(&sealed).unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
    let db = temp.path().join("data/xtrace.db");
    let verdict = validate_index_destination(&db, &home);
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(verdict, Ok(()));
}

#[test]
fn a_readable_alias_and_a_destination_inside_the_sources_are_still_refused() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with_project(temp.path());
    let data = temp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let db = data.join("xtrace.db");
    fs::write(&db, b"").unwrap();
    // An alias inside a readable project pointing at the database.
    std::os::unix::fs::symlink(&db, home.join(".claude/projects/-repo-a/link.jsonl")).unwrap();
    assert_eq!(
        validate_index_destination(&db, &home),
        Err("Native source alias points at index destination")
    );
    // A destination inside the native history, readable or not.
    assert_eq!(
        validate_index_destination(&home.join(".codex/sessions/xtrace.db"), &home),
        Err("Index database must be outside native history directories")
    );
}

#[test]
fn an_unresolvable_source_root_does_not_refuse_a_destination_beside_the_home() {
    // A dangling `.codex` alias and an untraversable `.config` are the
    // scan's to report; a destination beside the home is still accepted, and
    // one spelled inside the dangling root is still refused.
    let temp = tempfile::TempDir::new().unwrap();
    let home = home_with_project(temp.path());
    fs::remove_dir_all(home.join(".codex")).unwrap();
    std::os::unix::fs::symlink(temp.path().join("absent-target"), home.join(".codex")).unwrap();
    fs::create_dir_all(home.join(".config/memhub-plugin")).unwrap();
    fs::set_permissions(home.join(".config"), fs::Permissions::from_mode(0o000)).unwrap();
    let beside = validate_index_destination(&temp.path().join("data/xtrace.db"), &home);
    let inside = validate_index_destination(&home.join(".codex/xtrace.db"), &home);
    fs::set_permissions(home.join(".config"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(beside, Ok(()));
    assert!(inside.is_err(), "{inside:?}");
}
