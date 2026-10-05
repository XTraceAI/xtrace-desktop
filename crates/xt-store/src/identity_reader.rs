//! A read-only connection for live-status identity checks, independent of the
//! application's writer. No migrations, views, content reads or writer setup.
use crate::{Error, Host, Result, creation::user_sessions_with_native};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

const QUERY_BUDGET: Duration = Duration::from_millis(100);

pub struct SessionIdentityReader {
    connection: Connection,
}

struct ProgressGuard<'a>(&'a Connection);

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

impl SessionIdentityReader {
    /// Open only an existing file. URI interpretation, creation and SQLite lock
    /// waiting are disabled. In particular this never calls Store::configure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::ZERO)?;
        Ok(Self { connection })
    }

    /// The caller validates host-specific canonical ID formats and bounds the
    /// input. Every check shares one snapshot; any error discards the batch.
    /// The deadline interrupts SQLite VM work cooperatively, not OS file I/O.
    pub fn eligible(&self, identities: &[(&str, Host, &str)]) -> Result<BTreeSet<String>> {
        self.read_batch(identities, Instant::now() + QUERY_BUDGET, |_| {})
    }

    fn read_batch(
        &self,
        identities: &[(&str, Host, &str)],
        deadline: Instant,
        mut after_identity: impl FnMut(usize),
    ) -> Result<BTreeSet<String>> {
        self.connection
            .progress_handler(100, Some(move || Instant::now() >= deadline))?;
        let _progress = ProgressGuard(&self.connection);
        let snapshot = self.connection.unchecked_transaction()?;
        let mut eligible = BTreeSet::new();
        for (index, &(id, host, native)) in identities.iter().enumerate() {
            check_deadline(deadline)?;
            // Sole saved ID equality also proves the requested row's exact
            // host/native identity and user kind. Metadata conflicts are not a
            // reason to refuse it, just as in the writer's existing query.
            if user_sessions_with_native(&snapshot, host, native)? == [id] {
                eligible.insert(id.to_owned());
            }
            after_identity(index);
        }
        check_deadline(deadline)?;
        snapshot.commit()?;
        check_deadline(deadline)?;
        Ok(eligible)
    }
}

fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error::InvalidInput("session identity query budget expired"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionMeta, SessionSource, Store};

    fn session(store: &mut Store, id: &str, host: &str, native: &str) {
        let mut meta = SessionMeta::new(id, host, SessionSource::Transcript);
        meta.native_session_id = Some(native.into());
        store.upsert_session(&meta, false).unwrap();
    }

    #[test]
    fn identity_reader_missing_database_is_not_created() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("absent.sqlite");
        assert!(SessionIdentityReader::open(&path).is_err());
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn identity_reader_open_does_not_configure_migrate_or_write() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("empty.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch("CREATE TABLE sentinel(value TEXT)")
            .unwrap();
        drop(writer);
        let before = std::fs::read(&path).unwrap();
        let reader = SessionIdentityReader::open(&path).unwrap();
        assert!(reader.connection.is_readonly("main").unwrap());
        assert_eq!(
            reader
                .connection
                .pragma_query_value(None, "busy_timeout", |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            reader
                .connection
                .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                .unwrap(),
            "delete"
        );
        assert_eq!(
            reader
                .connection
                .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(reader.eligible(&[("a", Host::Claude, "a")]).is_err());
        assert!(
            reader
                .connection
                .execute("INSERT INTO sentinel VALUES ('no')", [])
                .is_err()
        );
        assert!(
            reader
                .connection
                .execute_batch("CREATE TABLE forbidden(value TEXT)")
                .is_err()
        );
        assert_eq!(reader.connection.total_changes(), 0);
        drop(reader);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn identity_reader_reuses_exact_unique_user_checks_and_keeps_conflicts() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("xtrace.db");
        let mut writer = Store::open(&path).unwrap();
        session(&mut writer, "a", "claude", "native-a");
        session(&mut writer, "b", "codex", "native-b");
        writer
            .connection
            .execute(
                "UPDATE sessions SET has_conflict=1 WHERE session_id='a'",
                [],
            )
            .unwrap();
        let reader = SessionIdentityReader::open(&path).unwrap();
        let ids = [
            ("a", Host::Claude, "native-a"),
            ("b", Host::Codex, "native-b"),
        ];
        assert_eq!(
            reader.eligible(&ids).unwrap(),
            ["a".into(), "b".into()].into()
        );
        for wrong in [
            ("a", Host::Codex, "native-a"),
            ("a", Host::Claude, "native-b"),
            ("alias", Host::Claude, "native-a"),
        ] {
            assert!(reader.eligible(&[wrong]).unwrap().is_empty());
        }
        writer
            .connection
            .execute("UPDATE sessions SET kind='judge' WHERE session_id='b'", [])
            .unwrap();
        session(&mut writer, "alias", "claude", "native-a");
        assert!(reader.eligible(&ids).unwrap().is_empty());
        assert!(writer.session("a").unwrap().unwrap().has_conflict);
        assert_eq!(reader.connection.total_changes(), 0);
    }

    #[test]
    fn identity_reader_batch_is_one_snapshot_and_next_request_sees_committed_wal() {
        use std::sync::mpsc;
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("xtrace.db");
        let mut writer = Store::open(&path).unwrap();
        session(&mut writer, "a", "claude", "a");
        session(&mut writer, "b", "codex", "b");
        let reader = SessionIdentityReader::open(&path).unwrap();
        let (start, started) = mpsc::sync_channel(1);
        let (done, committed) = mpsc::sync_channel(1);
        let concurrent = std::thread::spawn(move || {
            if started.recv_timeout(Duration::from_secs(1)).is_ok() {
                let result = writer
                    .connection
                    .execute("UPDATE sessions SET kind='judge'", []);
                done.send(result).unwrap();
            }
            writer // keep writer/WAL open until after both reader requests
        });
        let mut write_result = None;
        let identities = [("a", Host::Claude, "a"), ("b", Host::Codex, "b")];
        // The same production batch body; a test callback commits the writer
        // between the two queries instead of depending on scheduler timing.
        let snapshot = reader.read_batch(&identities, Instant::now() + QUERY_BUDGET, |index| {
            if index == 0 {
                start.send(()).unwrap();
                write_result = Some(committed.recv_timeout(Duration::from_secs(1)));
            }
        });
        drop(start);
        let writer = concurrent.join().unwrap();
        write_result.unwrap().unwrap().unwrap();
        assert_eq!(snapshot.unwrap(), ["a".into(), "b".into()].into());
        assert!(path.with_extension("db-wal").exists());
        assert!(reader.eligible(&identities).unwrap().is_empty());
        assert_eq!(reader.connection.total_changes(), 0);
        drop(writer);
    }

    #[test]
    fn identity_reader_sqlite_busy_has_no_wait_and_recovers() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("locked.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("CREATE TABLE sessions(session_id TEXT,host TEXT,native_session_id TEXT,kind TEXT); INSERT INTO sessions VALUES('a','claude','a','user')").unwrap();
        let reader = SessionIdentityReader::open(&path).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let started = Instant::now();
        let result = reader.eligible(&[("a", Host::Claude, "a")]);
        writer.execute_batch("ROLLBACK").unwrap();
        assert!(
            matches!(result, Err(Error::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == rusqlite::ErrorCode::DatabaseBusy)
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(
            reader.eligible(&[("a", Host::Claude, "a")]).unwrap(),
            ["a".into()].into()
        );
    }

    #[test]
    fn identity_reader_sql_error_discards_partial_batch_and_clears_handler() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("error.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("CREATE VIEW sessions AS SELECT CASE WHEN native='b' THEN abs(-9223372036854775808) ELSE native END session_id, 'claude' host, native native_session_id, 'user' kind FROM (SELECT 'a' native UNION ALL SELECT 'b')").unwrap();
        let reader = SessionIdentityReader::open(&path).unwrap();
        assert_eq!(
            reader.eligible(&[("a", Host::Claude, "a")]).unwrap(),
            ["a".into()].into()
        );
        assert!(
            reader
                .eligible(&[("a", Host::Claude, "a"), ("b", Host::Claude, "b")])
                .is_err()
        );
        assert!(reader.connection.is_autocommit());
        // Waiting beyond the previous deadline would interrupt this query if
        // the previous handler had leaked into the next use of the connection.
        std::thread::sleep(QUERY_BUDGET);
        assert_eq!(reader.connection.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0)).unwrap(), 500500);
        assert_eq!(
            reader.eligible(&[("a", Host::Claude, "a")]).unwrap(),
            ["a".into()].into()
        );
    }

    #[test]
    fn identity_reader_progress_deadline_interrupts_work_and_recovers() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("budget.sqlite");
        let writer = Connection::open(&path).unwrap();
        // A deliberately costly synthetic view exercises SQLite's progress
        // callback with the real 100ms budget, without a timer thread or FFI.
        writer.execute_batch("CREATE VIEW sessions AS WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT 'a' session_id,'claude' host,'a' native_session_id,'user' kind FROM n").unwrap();
        let reader = SessionIdentityReader::open(&path).unwrap();
        let started = Instant::now();
        let result = reader.eligible(&[("a", Host::Claude, "a")]);
        assert!(
            matches!(result, Err(Error::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == rusqlite::ErrorCode::OperationInterrupted)
        );
        assert!(started.elapsed() >= QUERY_BUDGET);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(reader.connection.is_autocommit());
        assert_eq!(reader.connection.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0)).unwrap(), 500500);
        writer.execute_batch("DROP VIEW sessions; CREATE TABLE sessions(session_id TEXT,host TEXT,native_session_id TEXT,kind TEXT); INSERT INTO sessions VALUES('a','claude','a','user')").unwrap();
        let reader = SessionIdentityReader::open(&path).unwrap();
        assert_eq!(
            reader.eligible(&[("a", Host::Claude, "a")]).unwrap(),
            ["a".into()].into()
        );
    }

    #[test]
    fn identity_reader_expired_batch_discards_previous_eligible_ids() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("xtrace.db");
        let mut writer = Store::open(&path).unwrap();
        session(&mut writer, "a", "claude", "a");
        let reader = SessionIdentityReader::open(&path).unwrap();
        let identities = [("a", Host::Claude, "a"), ("a", Host::Claude, "a")];
        let result = reader.read_batch(&identities, Instant::now() + QUERY_BUDGET, |index| {
            if index == 0 {
                std::thread::sleep(QUERY_BUDGET);
            }
        });
        assert!(result.is_err());
        assert_eq!(reader.eligible(&identities).unwrap(), ["a".into()].into());
    }
}
