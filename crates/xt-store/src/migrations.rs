//! Embedded migrations are registered in lexical filename order, one transaction
//! per version. Extend this chain; never modify an applied migration or reuse a
//! version. Unknown or non-prefix histories fail.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_canonical.sql")),
    (2, include_str!("../migrations/0002_ingest.sql")),
    (3, include_str!("../migrations/0003_native_checkpoints.sql")),
    (
        4,
        include_str!("../migrations/0004_native_record_copies.sql"),
    ),
    (
        5,
        include_str!("../migrations/0005_human_classification_replay.sql"),
    ),
    (
        6,
        include_str!("../migrations/0006_structural_tool_kinds.sql"),
    ),
    (
        7,
        include_str!("../migrations/0007_native_pr_witnesses.sql"),
    ),
    (8, include_str!("../migrations/0008_pr_refresh_status.sql")),
    (9, include_str!("../migrations/0009_repeat_group_keys.sql")),
    (
        10,
        include_str!("../migrations/0010_confirmed_automated_inputs.sql"),
    ),
    (
        11,
        include_str!("../migrations/0011_session_creation_relations.sql"),
    ),
    (
        12,
        include_str!("../migrations/0012_guardian_turn_inputs.sql"),
    ),
    (
        13,
        include_str!("../migrations/0013_injected_context_inputs.sql"),
    ),
    (
        14,
        include_str!("../migrations/0014_human_input_estimate.sql"),
    ),
    (
        15,
        include_str!("../migrations/0015_claude_launch_creations.sql"),
    ),
    (
        16,
        include_str!("../migrations/0016_task_notification_inputs.sql"),
    ),
    (17, include_str!("../migrations/0017_record_previews.sql")),
    (
        18,
        include_str!("../migrations/0018_session_child_facts.sql"),
    ),
    (
        19,
        include_str!("../migrations/0019_claude_launch_binding_witnesses.sql"),
    ),
    (
        20,
        include_str!("../migrations/0020_codex_cli_launch_children.sql"),
    ),
    (21, include_str!("../migrations/0021_tool_sent_inputs.sql")),
    (
        22,
        include_str!("../migrations/0022_session_child_checks.sql"),
    ),
];

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
        // Views follow the same writer-owned lifecycle; no second migration
        // runner or version table. Replace only these named projections.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let views = [
            ("v_human_inputs", include_str!("../views/human_inputs.sql")),
            ("v_records", include_str!("../views/records.sql")),
            (
                "v_usage_records",
                include_str!("../views/usage_records.sql"),
            ),
            (
                "v_response_usage",
                include_str!("../views/response_usage.sql"),
            ),
            (
                "v_session_events",
                include_str!("../views/session_events.sql"),
            ),
        ];
        let mut changed = false;
        for (name, sql) in views {
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
            transaction.execute_batch(
                "DROP VIEW IF EXISTS v_response_usage; DROP VIEW IF EXISTS v_usage_records; DROP VIEW IF EXISTS v_session_events; DROP VIEW IF EXISTS v_records;
                 DROP VIEW IF EXISTS v_human_inputs;",
            )?;
            for (_, sql) in views {
                transaction.execute_batch(sql)?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}
