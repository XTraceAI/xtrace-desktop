//! Embedded migrations are registered in lexical filename order, one transaction
//! per version. Extend this chain; never modify an applied migration or reuse a
//! version. Each applied version also records the fingerprint of the file that
//! applied it, and a database whose applied history differs from this build's
//! is refused before anything is written.

use crate::{Error, Result, Store};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;

/// One embedded migration file. `sha256` pins the lowercase hex SHA-256 of the
/// file's SQL with CRLF read as LF; a test checks every pin against its file.
/// `name` is the file name without `.sql`.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
    sha256: &'static str,
}

macro_rules! migration {
    ($version:literal, $name:literal, $sha256:literal $(,)?) => {
        Migration {
            version: $version,
            name: $name,
            sql: include_str!(concat!("../migrations/", $name, ".sql")),
            sha256: $sha256,
        }
    };
}

const MIGRATIONS: &[Migration] = &[
    migration!(
        1,
        "0001_canonical",
        "85cb6f63364cfb21a00f9076a0f65a7b4d47b0ebb0f3bab1953f2e8aead1f217"
    ),
    migration!(
        2,
        "0002_ingest",
        "500a51940f46adf4202569bb43225837846e01c0e0a5b5c56dcb074ad1a819e2"
    ),
    migration!(
        3,
        "0003_native_checkpoints",
        "a6b98b981debce7209164d6024dcdcc5de067c5bc1821094f9d3234111a5f6a3"
    ),
    migration!(
        4,
        "0004_native_record_copies",
        "bc28849faadf03091276aff9581eff20315154f8318f21778a12bcd8d1a346ce"
    ),
    migration!(
        5,
        "0005_human_classification_replay",
        "7cd610b6e70e025646a63957a1fda5a6f17c806bee8a188ed7e939a55d505143"
    ),
    migration!(
        6,
        "0006_structural_tool_kinds",
        "29cb344d9afeb1e14fd66899a02f69fe35eee2fb89d72b517374356bf42b1104"
    ),
    migration!(
        7,
        "0007_native_pr_witnesses",
        "522a7de4bee61ecf3b9f830a5dc51e1b0f336f0855e8d8da2eec83fe1f83cf57"
    ),
    migration!(
        8,
        "0008_pr_refresh_status",
        "0c6baa7df6e51cd5cf579d58821e0319b1024946b8fec737787ceca6be26fa98"
    ),
    migration!(
        9,
        "0009_repeat_group_keys",
        "ca7caff47c14e3324245624f3143ca95e45529e72273244fcbb6b86627006427"
    ),
    migration!(
        10,
        "0010_confirmed_automated_inputs",
        "20d15254490daba3d4415de8c97dfade312f98f616e9ab7981da15b63a2e1e4f"
    ),
    migration!(
        11,
        "0011_session_creation_relations",
        "cdd38ff8d2ba0068e035adb41ee1fde41b13a174640fcd2273c52ab9e98f4af9"
    ),
    migration!(
        12,
        "0012_guardian_turn_inputs",
        "353f2c6570dbe1f4e4cfc7220236b4a7f55f8287fd1f206b27b929a92893ca41"
    ),
    migration!(
        13,
        "0013_injected_context_inputs",
        "02c34bc7a49b7abb3cbba72c3eede84760af3a291fff64c8c36035ba77fe83fb"
    ),
    migration!(
        14,
        "0014_human_input_estimate",
        "e6a21899d8202f86f0d2009869aa5e1ee877db117087e9fe3418d3a271de4f90"
    ),
    migration!(
        15,
        "0015_claude_launch_creations",
        "215ee18c5ad3d1cb9ca8dccb66e417945079caff9b2859916b681dae2021575f"
    ),
    migration!(
        16,
        "0016_task_notification_inputs",
        "99354a115c8e6cf4991e88694eaf459aa0f12df11513f7790832ecb397b8e6e9"
    ),
    migration!(
        17,
        "0017_record_previews",
        "34cd8ac5ac36b0ba391fb0d3e1004909fee93209c48df98f3b67411cf7c2582f"
    ),
    migration!(
        18,
        "0018_session_child_facts",
        "331a8ecf093b727f04a5dddcd3ab00d403101c92c32a846325e39b0b349df043"
    ),
    migration!(
        19,
        "0019_claude_launch_binding_witnesses",
        "f25df83d84f549cf0faa3bd40b7d91239d5c3b2fb155f062abae2da50ad70bc8"
    ),
    migration!(
        20,
        "0020_codex_cli_launch_children",
        "5bce8cdeb0717a54c9d49486f740e4f49e4f0801713014944d5bcbe0e9e6e0e9"
    ),
    migration!(
        21,
        "0021_tool_sent_inputs",
        "af7b13fb61cc7ffa924169c66a2fcf86409d8bb90e29866d19790b4394bb5416"
    ),
    migration!(
        22,
        "0022_session_child_checks",
        "adac33a6535bd59d7336832f66ee325ec6ccf14bfceacae6d1b4391b07d8f043"
    ),
    migration!(
        23,
        "0023_pr_manual_failure",
        "86712a00fbfa88b91d6c22817206b5c861478bced3cae83f36c2fe1916d26617"
    ),
];

/// Which file applied each version. Created inside the history transaction, so
/// a refused open leaves no trace. `name` only explains a refusal; `sha256`
/// alone decides whether a recorded version matches this build's.
const FINGERPRINT_TABLE: &str = "CREATE TABLE IF NOT EXISTS migration_fingerprints (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*')
)";

impl Store {
    /// Highest applied migration after opening this store.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.connection.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn migrate(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL
            )",
        )?;
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            // Serialize the history read and migration with competing openers.
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            if verify_history(&transaction)? <= index {
                // An applied migration knows only the views of its own day,
                // and a table it rebuilds may be one the current views read.
                // The whole managed set is removed in this migration's own
                // transaction, so a failure rolls the removal back; it is
                // installed again below, after the last migration.
                drop_managed_views(&transaction)?;
                transaction.execute_batch(migration.sql)?;
                transaction.execute(
                    "INSERT INTO schema_version(version, applied_at)
                     VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    [migration.version],
                )?;
                record_fingerprint(&transaction, migration)?;
            }
            transaction.commit()?;
        }
        // Views follow the same writer-owned lifecycle; no second migration
        // runner or version table. Replace only the managed projections, all
        // or none.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changed = false;
        for (name, sql) in crate::views::ALL {
            let current: Option<String> = transaction
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='view' AND name=?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()?;
            changed |= current.as_deref() != Some(sql.trim().trim_end_matches(';'));
        }
        if changed {
            drop_managed_views(&transaction)?;
            for (_, sql) in crate::views::ALL {
                transaction.execute_batch(sql)?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

/// Removes every view [`crate::views::ALL`] manages, readers first; any other
/// view is left alone.
fn drop_managed_views(transaction: &Transaction) -> Result<()> {
    for (name, _) in crate::views::ALL.iter().rev() {
        transaction.execute_batch(&format!("DROP VIEW IF EXISTS {name}"))?;
    }
    Ok(())
}

/// Decides whether this database's applied migration history matches this
/// build, and returns how many of this build's migrations it has applied. It
/// runs inside the transaction that applies the next migration; an error
/// rolls that transaction back, so a refused database is left unchanged.
///
/// Applied versions must be exactly 1..=n, with n no larger than this build's
/// newest version. A recorded fingerprint must equal this build's for its
/// version. Versions without one (every database written before fingerprints,
/// or versions a build without them applied later) are accepted only when the
/// database's tables, columns, indexes and triggers equal those this build's
/// migrations 1..=n create; the current schema is compatible and fingerprints
/// are initialized. This cannot recover the SQL an older build actually ran. A
/// fingerprint row for a version `schema_version` no longer lists is ignored
/// and replaced when that version is applied again.
fn verify_history(transaction: &Transaction) -> Result<usize> {
    transaction.execute_batch(FINGERPRINT_TABLE)?;
    let versions = transaction
        .prepare("SELECT version FROM schema_version ORDER BY version")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if versions
        .iter()
        .any(|&version| version > MIGRATIONS.len() as i64)
    {
        return Err(Error::IncompatibleSchema);
    }
    if versions
        .iter()
        .zip(1..)
        .any(|(&version, expected)| version != expected)
    {
        return Err(Error::MigrationHistory(format!(
            "the applied versions {versions:?} are not 1, 2, 3 and so on without gaps"
        )));
    }
    let applied = &MIGRATIONS[..versions.len()];
    let recorded = transaction
        .prepare("SELECT version, name, sha256 FROM migration_fingerprints")?
        .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?
        .collect::<rusqlite::Result<BTreeMap<i64, (String, String)>>>()?;
    let mut unrecorded = Vec::new();
    for migration in applied {
        let Some((name, sha256)) = recorded.get(&migration.version) else {
            unrecorded.push(migration);
            continue;
        };
        if sha256 == migration.sha256 {
            continue;
        }
        let (version, expected) = (migration.version, migration.name);
        return Err(Error::MigrationHistory(if name == expected {
            format!(
                "version {version} was applied from a different {name} than this build's \
                 (fingerprint {} recorded, {} expected)",
                sha256.get(..12).unwrap_or(sha256),
                &migration.sha256[..12],
            )
        } else {
            format!(
                "version {version} was applied as {name}; this build's version {version} is {expected}"
            )
        }));
    }
    if !unrecorded.is_empty() {
        let differences = schema_differences(transaction, applied)?;
        if !differences.is_empty() {
            return Err(Error::MigrationHistory(format!(
                "its tables, columns, indexes or triggers differ from what this build's migrations 1-{} create: {}",
                applied.len(),
                differences.join("; ")
            )));
        }
        for migration in unrecorded {
            record_fingerprint(transaction, migration)?;
        }
    }
    Ok(applied.len())
}

fn record_fingerprint(transaction: &Transaction, migration: &Migration) -> Result<()> {
    transaction.execute(
        "INSERT OR REPLACE INTO migration_fingerprints(version, name, sha256) VALUES (?1, ?2, ?3)",
        params![migration.version, migration.name, migration.sha256],
    )?;
    Ok(())
}

/// How `database` differs from a reference database built in memory from
/// `applied` alone, with the connection setup every store uses and without
/// views. Compares names, table columns in order, and SQL definitions for
/// tables, indexes and triggers; `sqlite_*` objects and the two history tables
/// are not compared. Empty when they match.
fn schema_differences(database: &Connection, applied: &[Migration]) -> Result<Vec<String>> {
    let mut reference = Connection::open_in_memory()?;
    Store::prepare_connection(&reference, false)?;
    for migration in applied {
        let transaction = reference.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.commit()?;
    }
    let (actual, expected) = (schema_shape(database)?, schema_shape(&reference)?);
    let mut differences = Vec::new();
    for (object, shape) in &actual {
        let Some(expected_shape) = expected.get(object) else {
            differences.push(format!("{object} is unexpected"));
            continue;
        };
        let before = differences.len();
        for column in expected_shape
            .columns
            .iter()
            .filter(|c| !shape.columns.contains(c))
        {
            differences.push(format!("{object} has no column {column}"));
        }
        for column in shape
            .columns
            .iter()
            .filter(|c| !expected_shape.columns.contains(c))
        {
            differences.push(format!("{object} has an unexpected column {column}"));
        }
        if differences.len() == before && shape.columns != expected_shape.columns {
            differences.push(format!("{object} has its columns in a different order"));
        }
        if shape.definition != expected_shape.definition {
            differences.push(format!("{object} has a different definition"));
        }
    }
    for object in expected
        .keys()
        .filter(|object| !actual.contains_key(*object))
    {
        differences.push(format!("{object} is missing"));
    }
    const SHOWN: usize = 12;
    if differences.len() > SHOWN {
        let more = differences.len() - SHOWN;
        differences.truncate(SHOWN);
        differences.push(format!("and {more} more differences"));
    }
    Ok(differences)
}

struct SchemaObject {
    columns: Vec<String>,
    definition: Vec<String>,
}

/// `"table <name>"`, `"index <name>"` or `"trigger <name>"` mapped to its
/// columns in order (empty for indexes and triggers) and definition tokens.
fn schema_shape(connection: &Connection) -> Result<BTreeMap<String, SchemaObject>> {
    let objects = connection
        .prepare(
            "SELECT type, name, sql FROM main.sqlite_schema
             WHERE type IN ('table', 'index', 'trigger')
               AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
               AND name NOT IN ('schema_version', 'migration_fingerprints')",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut shape = BTreeMap::new();
    for (kind, name, sql) in objects {
        let columns = if kind == "table" {
            connection
                .prepare("SELECT name FROM pragma_table_info(?1, 'main') ORDER BY cid")?
                .query_map([&name], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        } else {
            Vec::new()
        };
        shape.insert(
            format!("{kind} {name}"),
            SchemaObject {
                columns,
                definition: definition_tokens(&sql),
            },
        );
    }
    Ok(shape)
}

/// Ignore SQL formatting and comments, preserving token boundaries and every
/// byte inside quoted strings/identifiers. Removing all whitespace would erase
/// meaningful spaces in a CHECK literal or join separate tokens together.
fn definition_tokens(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"--") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            continue;
        }
        let start = at;
        if matches!(bytes[at], b'\'' | b'"' | b'`' | b'[') {
            let close = if bytes[at] == b'[' { b']' } else { bytes[at] };
            at += 1;
            while at < bytes.len() {
                if bytes[at] == close {
                    at += 1;
                    if close != b']' && bytes.get(at) == Some(&close) {
                        at += 1;
                    } else {
                        break;
                    }
                } else {
                    at += 1;
                }
            }
        } else if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' || bytes[at] >= 128 {
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' || bytes[at] >= 128)
            {
                at += 1;
            }
        } else {
            at += 1;
        }
        tokens.push(sql[start..at].to_owned());
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::{MIGRATIONS, definition_tokens, drop_managed_views};
    use crate::views;
    use crate::{Error, Store};
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};
    use std::path::Path;

    #[test]
    fn definition_comparison_ignores_formatting_and_comments_but_preserves_literals() {
        let tokens = definition_tokens;
        assert_eq!(
            tokens("CREATE TABLE x(a TEXT CHECK(a='two  words'))"),
            tokens("CREATE /* note */ TABLE\nx ( a TEXT -- note\n CHECK ( a = 'two  words' ) )")
        );
        for literal in [
            "two words",
            "two\t words",
            "two\n words",
            "two -- words",
            "two /* words */",
            "two'' words",
        ] {
            assert_ne!(
                tokens("CHECK(a='two  words')"),
                tokens(&format!("CHECK(a='{literal}')"))
            );
        }
        assert_ne!(tokens("CHECK(a>=0)"), tokens("CHECK(a>=-1)"));
        assert_ne!(tokens("a b"), tokens("ab"));
        assert_eq!(
            tokens("SELECT 'é -- /* */ ''x', \"a  b\", `x``y`, [a b]"),
            tokens(" SELECT\n'é -- /* */ ''x' , \"a  b\" , `x``y` , [a b] ")
        );
    }

    #[test]
    fn every_migration_file_matches_its_pinned_fingerprint() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let files = std::fs::read_dir(directory)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "sql")
            })
            .count();
        assert_eq!(
            files,
            MIGRATIONS.len(),
            "every migration file must be listed in MIGRATIONS"
        );
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(migration.version, index as i64 + 1, "{}", migration.name);
            assert!(
                migration
                    .name
                    .starts_with(&format!("{:04}_", migration.version)),
                "{} is registered as version {}",
                migration.name,
                migration.version
            );
            let actual = format!("{:x}", Sha256::digest(migration.sql.replace("\r\n", "\n")));
            assert_eq!(
                actual, migration.sha256,
                "{} no longer matches its pinned fingerprint. Applied migrations must never \
                 change: undo this edit and add a new migration instead. If only a development \
                 database applied a draft of this file, delete that database. Pin a new, never \
                 applied migration as {actual}.",
                migration.name
            );
        }
    }

    /// Every view's name and stored definition, in name order.
    fn views(path: &Path) -> Vec<(String, String)> {
        Connection::open(path)
            .unwrap()
            .prepare("SELECT name, sql FROM sqlite_schema WHERE type='view' ORDER BY name")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// The applied versions and the recorded fingerprints, in version order.
    fn history(path: &Path) -> (Vec<i64>, Vec<(i64, String)>) {
        let connection = Connection::open(path).unwrap();
        let versions = connection
            .prepare("SELECT version FROM schema_version ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let fingerprints = connection
            .prepare("SELECT version, sha256 FROM migration_fingerprints ORDER BY version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        (versions, fingerprints)
    }

    fn schema_cookie(path: &Path) -> i64 {
        Connection::open(path)
            .unwrap()
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap()
    }

    /// A current store with one session, one record and one view this build
    /// does not manage.
    fn current(directory: &tempfile::TempDir) -> std::path::PathBuf {
        let path = directory.path().join("views.sqlite");
        drop(Store::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "INSERT INTO sessions(session_id,host,source) VALUES ('s','claude','transcript');
                 INSERT INTO records(uuid,session_id,type,ts,ts_ms,is_meta,is_sidechain,
                     is_tool_result_carrier,has_conflict,first_seen_at)
                 VALUES ('r','s','user','2026-10-06T00:00:00Z',1791244800000,0,0,0,0,'2026-10-06T00:00:00Z');
                 CREATE VIEW v_unmanaged AS SELECT session_id FROM sessions;",
            )
            .unwrap();
        path
    }

    /// The managed set as this build defines it, plus the unmanaged views.
    fn installed(path: &Path) -> Vec<(String, String)> {
        let mut expected: Vec<(String, String)> = views::ALL
            .iter()
            .map(|(name, sql)| {
                (
                    name.to_string(),
                    sql.trim().trim_end_matches(';').to_string(),
                )
            })
            .collect();
        expected.extend(
            views(path)
                .into_iter()
                .filter(|(name, _)| !views::ALL.iter().any(|(managed, _)| managed == name)),
        );
        expected.sort();
        expected
    }

    /// The same rebuild an applied migration such as 0015 performs on a table
    /// the current projection reads: copy, drop, rename into place.
    const REBUILD: &str = "CREATE TABLE tool_sent_inputs_rebuilt AS SELECT * FROM tool_sent_inputs;
        DROP TABLE tool_sent_inputs;
        ALTER TABLE tool_sent_inputs_rebuilt RENAME TO tool_sent_inputs;";

    /// Undoes everything migration 22 applied, leaving it pending again.
    const UNDO_22: &str = "DROP TABLE session_child_checks;
        ALTER TABLE claude_launch_groups DROP COLUMN unfinished_starts;
        ALTER TABLE claude_launch_candidates DROP COLUMN child_check_complete;
        ALTER TABLE claude_launch_candidates DROP COLUMN launch_check_fingerprint;
        ALTER TABLE claude_launch_staged_candidates DROP COLUMN launch_check_fingerprint;";

    /// Undoes everything migration 23 applied, leaving it pending again.
    const UNDO_23: &str = "ALTER TABLE pull_requests DROP COLUMN manual_failed_at;";

    /// Forgets that migrations from `version` on were applied, leaving what
    /// they wrote in place.
    fn forget_from(version: u32) -> String {
        format!(
            "DELETE FROM schema_version WHERE version>={version};
             DELETE FROM migration_fingerprints WHERE version>={version};"
        )
    }

    #[test]
    fn only_removing_the_whole_managed_set_lets_a_pending_migration_rebuild_a_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = current(&directory);
        let mut connection = Connection::open(&path).unwrap();
        // Today's views as they stand, or without the internal projection
        // alone, both stop the rename: a public view would read a missing table.
        for removal in ["", "DROP VIEW v_record_metadata;"] {
            let transaction = connection.transaction().unwrap();
            transaction.execute_batch(removal).unwrap();
            let refused = transaction.execute_batch(REBUILD).unwrap_err().to_string();
            assert!(refused.contains("error in view"), "{refused}");
        }
        let transaction = connection.transaction().unwrap();
        drop_managed_views(&transaction).unwrap();
        transaction.execute_batch(REBUILD).unwrap();
        let left: Vec<String> = transaction
            .prepare("SELECT name FROM sqlite_schema WHERE type='view' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, ["session_work_records", "v_unmanaged"]);
    }

    #[test]
    fn a_reopen_with_nothing_pending_rewrites_no_view() {
        let directory = tempfile::tempdir().unwrap();
        let path = current(&directory);
        let (before, cookie) = (views(&path), schema_cookie(&path));
        drop(Store::open(&path).unwrap());
        assert_eq!(views(&path), before);
        assert_eq!(
            schema_cookie(&path),
            cookie,
            "nothing in the schema was rewritten"
        );
    }

    #[test]
    fn a_changed_or_missing_definition_reinstalls_the_whole_managed_set() {
        let directory = tempfile::tempdir().unwrap();
        let path = current(&directory);
        let expected = installed(&path);
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT 1 AS stale;
                 DROP VIEW v_record_metadata;",
            )
            .unwrap();
        drop(Store::open(&path).unwrap());
        assert_eq!(views(&path), expected);
    }

    #[test]
    fn pending_migrations_replace_the_managed_set_and_keep_every_other_view() {
        let directory = tempfile::tempdir().unwrap();
        let path = current(&directory);
        let (expected, (versions, fingerprints)) = (installed(&path), history(&path));
        // Migration 23 pending again: its column and history are gone.
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!("{UNDO_23} {}", forget_from(23)))
            .unwrap();
        drop(Store::open(&path).unwrap());
        assert_eq!(views(&path), expected);
        let (after, after_fingerprints) = history(&path);
        assert_eq!(after, versions);
        assert_eq!(after_fingerprints, fingerprints);
    }

    #[test]
    fn a_failing_pending_migration_brings_back_every_view_and_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = current(&directory);
        // Migration 23 pending, but its column is still there, so its SQL
        // fails after the managed set has been removed in the same transaction.
        Connection::open(&path)
            .unwrap()
            .execute_batch(&forget_from(23))
            .unwrap();
        let (before, history_before) = (views(&path), history(&path));
        assert!(Store::open(&path).is_err());
        assert_eq!(views(&path), before);
        assert_eq!(history(&path), history_before);
        let rows: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn an_earlier_migration_stays_committed_when_a_later_one_fails() {
        let directory = tempfile::tempdir().unwrap();
        let path = current(&directory);
        let expected = installed(&path);
        // 22 and 23 pending; 22 can apply, 23 cannot while its column remains.
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!("{UNDO_22} {}", forget_from(22)))
            .unwrap();
        assert!(Store::open(&path).is_err());
        let (versions, fingerprints) = history(&path);
        assert_eq!(versions.last(), Some(&22), "migration 22 stayed committed");
        assert_eq!(fingerprints.last().map(|(version, _)| *version), Some(22));
        // Once the conflict is gone, the next open finishes and reinstalls.
        Connection::open(&path)
            .unwrap()
            .execute_batch(UNDO_23)
            .unwrap();
        drop(Store::open(&path).unwrap());
        assert_eq!(views(&path), expected);
    }

    #[test]
    fn a_refused_history_removes_no_view() {
        for refusal in [
            // A recorded fingerprint that is not this build's.
            "UPDATE migration_fingerprints SET sha256=printf('%064d', 0) WHERE version=3;",
            // A gap in the applied versions.
            "DELETE FROM schema_version WHERE version=5;",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = current(&directory);
            Connection::open(&path)
                .unwrap()
                .execute_batch(&format!("{} {refusal}", forget_from(23)))
                .unwrap();
            let (before, history_before) = (views(&path), history(&path));
            assert!(matches!(
                Store::open(&path),
                Err(Error::MigrationHistory(_))
            ));
            assert_eq!(views(&path), before, "{refusal}");
            assert_eq!(history(&path), history_before, "{refusal}");
        }
    }
}
