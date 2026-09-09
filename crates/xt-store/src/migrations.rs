//! Embedded migrations are registered in lexical filename order, one transaction
//! per version. Extend this chain; never modify an applied migration or reuse a
//! version. Unknown or non-prefix histories fail.

use crate::{Error, Result, Store};
use rusqlite::TransactionBehavior;

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/0001_canonical.sql"))];

impl Store {
    pub fn migrate(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL
            )",
        )?;
        for (index, &(version, sql)) in MIGRATIONS.iter().enumerate() {
            // Serialize the history read and migration with competing openers.
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let applied = transaction
                .prepare("SELECT version FROM schema_version ORDER BY version")?
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if applied.len() > MIGRATIONS.len()
                || !applied.iter().zip(MIGRATIONS).all(|(a, (b, _))| a == b)
            {
                return Err(Error::IncompatibleSchema);
            }
            if applied.len() <= index {
                transaction.execute_batch(sql)?;
                transaction.execute(
                    "INSERT INTO schema_version(version, applied_at)
                     VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    [version],
                )?;
            }
            transaction.commit()?;
        }
        Ok(())
    }
}
