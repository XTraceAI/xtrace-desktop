//! Every index an older build could have written upgrades to the current
//! schema. Each older index is built only from the migration files that build
//! applied, never from the current tables with later additions removed, so a
//! migration that cannot run on its real predecessor fails here.
use rusqlite::Connection;
use tempfile::TempDir;
use xt_store::Store;

/// `(version, sql)` for every migration file, in the order they apply.
fn migrations() -> Vec<(i64, String)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    files.sort();
    let migrations: Vec<_> = files
        .iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            let version = name.split('_').next().unwrap().parse().unwrap();
            (version, std::fs::read_to_string(path).unwrap())
        })
        .collect();
    let versions: Vec<_> = migrations.iter().map(|(version, _)| *version).collect();
    assert_eq!(versions, (1..=versions.len() as i64).collect::<Vec<_>>());
    migrations
}

/// What a build whose newest migration was `version` left: its migrations
/// applied one transaction each, and one session stored before the second.
fn index_written_by(path: &std::path::Path, migrations: &[(i64, String)], version: i64) {
    let mut sql = Connection::open(path).unwrap();
    sql.execute_batch(
        "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
    )
    .unwrap();
    for (applied, migration) in &migrations[..version as usize] {
        let transaction = sql.transaction().unwrap();
        transaction.execute_batch(migration).unwrap();
        transaction
            .execute(
                "INSERT INTO schema_version(version, applied_at) VALUES (?1, 'older-build')",
                [applied],
            )
            .unwrap();
        if *applied == 1 {
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

fn count(path: &std::path::Path, query: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn an_index_from_every_older_version_upgrades_to_the_current_schema_once() {
    let migrations = migrations();
    let current = migrations.len() as i64;
    for version in 1..current {
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

        // A second open finds nothing left to apply.
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.schema_version().unwrap(), current as u32);
    }
}
