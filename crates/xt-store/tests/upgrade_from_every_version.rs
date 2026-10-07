//! Every index an older build could have written upgrades to the current
//! schema. Each older index is built only from the migration files that build
//! applied, never from the current tables with later additions removed, so a
//! migration that cannot run on its real predecessor fails here.
//!
//! New applied versions record the fingerprint of the file that applied them.
//! Legacy files without fingerprints prove current schema compatibility only;
//! their historical SQL cannot be recovered. A mismatch is refused unchanged.
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use xt_store::{Error, Store};

/// One migration file: its version, its name without `.sql`, and its SQL.
struct File {
    version: i64,
    name: String,
    sql: String,
}

/// Every migration file, in the order they apply.
fn migrations() -> Vec<File> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    paths.sort();
    let migrations: Vec<_> = paths
        .iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
            File {
                version: name.split('_').next().unwrap().parse().unwrap(),
                name,
                sql: std::fs::read_to_string(path).unwrap(),
            }
        })
        .collect();
    let versions: Vec<_> = migrations.iter().map(|file| file.version).collect();
    assert_eq!(versions, (1..=versions.len() as i64).collect::<Vec<_>>());
    migrations
}

fn fingerprint(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n")))
}

/// What a build whose newest migration was `version` left: its migrations
/// applied one transaction each, and one session stored before the second.
/// Builds from before fingerprints kept only `schema_version`.
fn index_written_by(path: &std::path::Path, migrations: &[File], version: i64) {
    let mut sql = Connection::open(path).unwrap();
    sql.execute_batch(
        "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
    )
    .unwrap();
    for file in &migrations[..version as usize] {
        let transaction = sql.transaction().unwrap();
        transaction.execute_batch(&file.sql).unwrap();
        transaction
            .execute(
                "INSERT INTO schema_version(version, applied_at) VALUES (?1, 'older-build')",
                [file.version],
            )
            .unwrap();
        if file.version == 1 {
            transaction
                .execute_batch(
                    "INSERT INTO sessions(session_id, host, source)
                     VALUES ('synthetic-older-session', 'claude', 'transcript')",
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }
}

/// Apply `file` as `version` the way an older build that numbered it so did.
fn apply_as(path: &std::path::Path, file: &File, version: i64) {
    let mut sql = Connection::open(path).unwrap();
    let transaction = sql.transaction().unwrap();
    transaction.execute_batch(&file.sql).unwrap();
    transaction
        .execute(
            "INSERT INTO schema_version(version, applied_at) VALUES (?1, 'other-build')",
            [version],
        )
        .unwrap();
    transaction.commit().unwrap();
}

fn count(path: &std::path::Path, query: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

/// The recorded `(version, name, sha256)` rows; empty without the table.
fn recorded(path: &std::path::Path) -> Vec<(i64, String, String)> {
    let sql = Connection::open(path).unwrap();
    let exists = sql
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE name='migration_fingerprints'",
            [],
            |_| Ok(()),
        )
        .optional()
        .unwrap()
        .is_some();
    if !exists {
        return Vec::new();
    }
    sql.prepare("SELECT version, name, sha256 FROM migration_fingerprints ORDER BY version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The rows this build's files 1..=`through` must have recorded.
fn expected(migrations: &[File], through: i64) -> Vec<(i64, String, String)> {
    migrations[..through as usize]
        .iter()
        .map(|file| (file.version, file.name.clone(), fingerprint(&file.sql)))
        .collect()
}

/// The app's databases use WAL. Converting here, before the snapshot, keeps
/// that header change out of the byte comparison of a refused open.
fn use_wal(path: &std::path::Path) {
    let sql = Connection::open(path).unwrap();
    let mode: String = sql
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

/// Open must refuse with a history error. Returns its message after checking
/// that the file is byte-identical and no `-wal` file was left behind.
fn refused(path: &std::path::Path) -> String {
    let before = std::fs::read(path).unwrap();
    let message = match Store::open(path) {
        Ok(_) => panic!("a database with a different migration history opened"),
        Err(Error::MigrationHistory(message)) => message,
        Err(other) => panic!("refused for another reason: {other}"),
    };
    assert!(
        std::fs::read(path).unwrap() == before,
        "a refused open changed the file"
    );
    let wal = path.with_extension("db-wal");
    assert!(std::fs::metadata(&wal).map_or(true, |m| m.len() == 0));
    message
}

#[test]
fn an_index_from_every_older_version_upgrades_to_the_current_schema_once() {
    let migrations = migrations();
    let current = migrations.len() as i64;
    for version in 1..=current {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("xtrace.db");
        index_written_by(&path, &migrations, version);

        let upgraded = Store::open(&path)
            .unwrap_or_else(|error| panic!("schema {version} did not upgrade: {error}"));
        assert_eq!(upgraded.schema_version().unwrap(), current as u32);
        drop(upgraded);
        assert_eq!(count(&path, "SELECT count(*) FROM schema_version"), current);
        assert_eq!(
            count(
                &path,
                "SELECT count(*) FROM schema_version WHERE applied_at='older-build'"
            ),
            version,
            "schema {version}: an applied migration ran again"
        );
        assert_eq!(count(&path, "SELECT count(*) FROM sessions"), 1);
        // Legacy schema compatibility was checked before fingerprints were
        // initialized; later migrations recorded them as they applied.
        assert_eq!(
            recorded(&path),
            expected(&migrations, current),
            "schema {version}"
        );

        // A second open finds nothing left to apply.
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.schema_version().unwrap(), current as u32);
        drop(reopened);
        assert_eq!(recorded(&path), expected(&migrations, current));
    }
}

#[test]
fn a_new_database_records_the_fingerprint_of_every_migration_file() {
    let migrations = migrations();
    let current = migrations.len() as i64;
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("xtrace.db");
    drop(Store::open(&path).unwrap());
    assert_eq!(recorded(&path), expected(&migrations, current));
    let history = |path: &std::path::Path| -> Vec<(i64, String)> {
        Connection::open(path)
            .unwrap()
            .prepare("SELECT version, applied_at FROM schema_version ORDER BY version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let applied = history(&path);
    drop(Store::open(&path).unwrap());
    assert_eq!(history(&path), applied);
    assert_eq!(recorded(&path), expected(&migrations, current));
}

/// The incident: builds before a merge numbered `session_child_checks` 21; this
/// build's 21 is `tool_sent_inputs` and its 22 is `session_child_checks`.
#[test]
fn a_database_that_applied_another_file_under_a_reused_number_is_refused_unchanged() {
    let migrations = migrations();
    let session_child_checks = &migrations[21];
    assert_eq!(session_child_checks.name, "0022_session_child_checks");
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("xtrace.db");
    index_written_by(&path, &migrations, 20);
    apply_as(&path, session_child_checks, 21);
    use_wal(&path);

    let message = refused(&path);
    assert!(message.contains("migrations 1-21"), "{message}");
    assert!(
        message.contains("table session_child_checks is unexpected"),
        "{message}"
    );
    assert!(
        message.contains("table tool_sent_inputs is missing"),
        "{message}"
    );
    assert_eq!(count(&path, "SELECT count(*) FROM schema_version"), 21);
    assert!(recorded(&path).is_empty());
}

#[test]
fn a_recorded_fingerprint_from_another_file_is_refused_unchanged() {
    let migrations = migrations();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("xtrace.db");
    drop(Store::open(&path).unwrap());
    let all = recorded(&path);

    // Version 21 recorded as the old 0021 that is now 0022.
    let sql = Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE migration_fingerprints SET name='0021_session_child_checks', sha256=?1 WHERE version=21",
        [fingerprint(&migrations[21].sql)],
    )
    .unwrap();
    drop(sql);
    let message = refused(&path);
    assert_eq!(
        message,
        "version 21 was applied as 0021_session_child_checks; \
         this build's version 21 is 0021_tool_sent_inputs"
    );

    // Version 4 recorded under its own name from an edited file.
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(
        "UPDATE migration_fingerprints SET sha256=(SELECT sha256 FROM migration_fingerprints WHERE version=3) WHERE version=4",
    )
    .unwrap();
    drop(sql);
    let message = refused(&path);
    assert!(
        message.starts_with(
            "version 4 was applied from a different 0004_native_record_copies than this build's"
        ),
        "{message}"
    );
    assert_eq!(recorded(&path).len(), all.len());
}

/// A build without fingerprints (a rollback, say) can apply versions after
/// fingerprinted ones. Only those are unrecorded; they are checked the same way.
#[test]
fn versions_applied_without_fingerprints_after_recorded_ones_are_checked() {
    let migrations = migrations();
    let current = migrations.len() as i64;
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("xtrace.db");
    drop(Store::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM migration_fingerprints WHERE version>=21", [])
        .unwrap();
    drop(Store::open(&path).unwrap());
    assert_eq!(recorded(&path), expected(&migrations, current));

    // The same history, but that build's 21 was today's 22.
    let mixed = directory.path().join("mixed.db");
    index_written_by(&mixed, &migrations, 20);
    let sql = Connection::open(&mixed).unwrap();
    sql.execute_batch(
        "CREATE TABLE migration_fingerprints (
            version INTEGER PRIMARY KEY, name TEXT NOT NULL, sha256 TEXT NOT NULL
        )",
    )
    .unwrap();
    for (version, name, sha256) in expected(&migrations, 20) {
        sql.execute(
            "INSERT INTO migration_fingerprints VALUES (?1, ?2, ?3)",
            rusqlite::params![version, name, sha256],
        )
        .unwrap();
    }
    drop(sql);
    apply_as(&mixed, &migrations[21], 21);
    use_wal(&mixed);
    let message = refused(&mixed);
    assert!(
        message.contains("table session_child_checks is unexpected"),
        "{message}"
    );
    assert_eq!(recorded(&mixed), expected(&migrations, 20));
}

/// Tests and manual repairs step a database back by deleting `schema_version`
/// rows and undoing that version's schema. The fingerprint row left behind is
/// ignored, and replaced when the version applies again.
#[test]
fn a_fingerprint_left_for_an_unapplied_version_is_replaced_when_it_applies() {
    let migrations = migrations();
    let current = migrations.len() as i64;
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("xtrace.db");
    drop(Store::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "UPDATE migration_fingerprints SET name='0022_draft', sha256=(SELECT sha256 FROM migration_fingerprints WHERE version=1) WHERE version=22;
             ALTER TABLE pull_requests DROP COLUMN manual_failed_at;
             DROP TABLE session_child_checks;
             ALTER TABLE claude_launch_groups DROP COLUMN unfinished_starts;
             ALTER TABLE claude_launch_candidates DROP COLUMN child_check_complete;
             ALTER TABLE claude_launch_candidates DROP COLUMN launch_check_fingerprint;
             ALTER TABLE claude_launch_staged_candidates DROP COLUMN launch_check_fingerprint;
             DELETE FROM schema_version WHERE version>=22;",
        )
        .unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), current as u32);
    drop(store);
    assert_eq!(recorded(&path), expected(&migrations, current));
}

#[test]
fn a_gap_in_the_applied_versions_is_refused_unchanged() {
    let migrations = migrations();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("xtrace.db");
    index_written_by(&path, &migrations, 3);
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM schema_version WHERE version=2", [])
        .unwrap();
    use_wal(&path);
    let message = refused(&path);
    assert_eq!(
        message,
        "the applied versions [1, 3] are not 1, 2, 3 and so on without gaps"
    );
}

#[test]
fn a_healthy_public_0_1_2_database_upgrades_using_its_exact_22_migration_files() {
    // These hashes were read from the public 0.1.2 source, independently of the
    // current build's MIGRATIONS pins. Equality binds the SQL this test applies
    // to that release without duplicating all 22 migration files.
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/public-0.1.2-migrations.json")).unwrap();
    assert_eq!(
        manifest["commit"],
        "92b54b4a7f27d44f31cf0e1318bf39d4ca33656e"
    );
    let files = migrations();
    let public = manifest["files"].as_array().unwrap();
    assert_eq!(public.len(), 22);
    for (file, expected) in files.iter().zip(public) {
        assert_eq!(format!("{}.sql", file.name), expected["file"]);
        assert_eq!(
            format!("{:x}", Sha256::digest(file.sql.as_bytes())),
            expected["sha256"]
        );
    }
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("public-release.db");
    index_written_by(&path, &files, 22);
    use_wal(&path);
    assert!(recorded(&path).is_empty());
    drop(Store::open(&path).unwrap());
    assert_eq!(count(&path, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM schema_version WHERE applied_at='older-build'"
        ),
        22
    );
    assert_eq!(recorded(&path), expected(&files, files.len() as i64));
}

#[test]
fn legacy_same_name_checks_indexes_and_triggers_with_different_definitions_are_refused() {
    for (version, file_index, from, to, object) in [
        (1, 0, "text_len >= 0", "text_len >= -1", "table records"),
        (
            1,
            0,
            "records_ts ON records(ts_ms)",
            "records_ts ON records(ts_ms DESC)",
            "index records_ts",
        ),
        (
            2,
            1,
            "RAISE(ABORT, 'capture receipt is immutable')",
            "RAISE(IGNORE)",
            "trigger capture_receipts_immutable_delete",
        ),
    ] {
        let mut files = migrations();
        assert!(
            files[file_index].sql.contains(from),
            "missing control: {from}"
        );
        files[file_index].sql = files[file_index].sql.replace(from, to);
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("different.db");
        index_written_by(&path, &files, version);
        use_wal(&path);
        let message = refused(&path);
        assert!(
            message.contains(&format!("{object} has a different definition")),
            "{message}"
        );
        assert!(recorded(&path).is_empty());
        assert_eq!(count(&path, "SELECT count(*) FROM schema_version"), version);
    }
}
