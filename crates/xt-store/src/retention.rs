//! Persisted future-write policy and explicit transactional content removal.
//! Table owners register content columns, never arbitrary SQL or connection callbacks.

use crate::{Error, Result, Store};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const SETTING: &str = "content_retention";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionMode {
    FullContent,
    #[default]
    MetadataOnly,
}

impl Store {
    pub fn retention_mode(&self) -> Result<RetentionMode> {
        read_mode(&self.connection)
    }

    /// Changes future writes only. This does not call the content purge operation.
    pub fn set_retention_mode(&mut self, mode: RetentionMode) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            params![SETTING, serde_json::to_string(&mode)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Explicit deletion boundary. The caller owns user confirmation and publishes
    /// content invalidation only from the returned, committed outcome. Original
    /// host files are never opened. This clears logical fields, not backup bytes.
    pub fn purge_content(&mut self, registry: &ContentRegistry) -> Result<PurgeOutcome> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Validate every registered owner before any field is changed.
        for (table, columns) in &registry.tables {
            let kind: Option<String> = transaction
                .query_row(
                    "SELECT type FROM sqlite_schema WHERE name=?1",
                    [table],
                    |row| row.get(0),
                )
                .optional()?;
            if kind.as_deref() != Some("table") {
                return Err(Error::InvalidInput("registered content table is missing"));
            }
            let fields = transaction
                .prepare("SELECT name,\"notnull\",pk,hidden FROM pragma_table_xinfo(?1)")?
                .query_map([table], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if columns.iter().any(|column| {
                !fields.iter().any(|(name, required, pk, hidden)| {
                    name == column && !required && *pk == 0 && *hidden == 0
                })
            }) {
                return Err(Error::InvalidInput(
                    "registered content column must exist and be nullable",
                ));
            }
        }
        let mut tables = Vec::new();
        for (table, columns) in &registry.tables {
            let assignments = columns
                .iter()
                .map(|name| format!("\"{name}\"=NULL"))
                .collect::<Vec<_>>()
                .join(",");
            let predicate = columns
                .iter()
                .map(|name| format!("\"{name}\" IS NOT NULL"))
                .collect::<Vec<_>>()
                .join(" OR ");
            let rows = transaction.execute(
                &format!("UPDATE \"{table}\" SET {assignments} WHERE {predicate}"),
                [],
            )?;
            tables.push(PurgedTable {
                table: (*table).to_owned(),
                rows,
            });
        }
        transaction.commit()?;
        Ok(PurgeOutcome {
            invalidate_content: tables.iter().any(|table| table.rows > 0),
            tables,
        })
    }
}

pub(crate) fn allows_content(connection: &Connection, requested: bool) -> Result<bool> {
    Ok(read_mode(connection)? == RetentionMode::FullContent && requested)
}

fn read_mode(connection: &Connection) -> Result<RetentionMode> {
    let saved: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key=?1",
            [SETTING],
            |row| row.get(0),
        )
        .optional()?;
    saved
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|_| Error::InvalidInput("stored content retention mode is invalid"))
        })
        .unwrap_or(Ok(RetentionMode::default()))
}

/// Compiled table owners extend this inventory when they add content-bearing
/// storage. Registered names are code constants, never input from the renderer.
#[derive(Clone, Debug)]
pub struct ContentRegistry {
    tables: BTreeMap<&'static str, BTreeSet<&'static str>>,
}

impl Default for ContentRegistry {
    fn default() -> Self {
        Self {
            tables: [
                ("sessions", BTreeSet::from(["title"])),
                ("records", BTreeSet::from(["content_json"])),
                ("tool_uses", BTreeSet::from(["input_json"])),
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl ContentRegistry {
    pub fn register(&mut self, table: &'static str, columns: &[&'static str]) -> Result<()> {
        if !identifier(table)
            || table.starts_with("sqlite_")
            || columns.is_empty()
            || columns.iter().any(|column| !identifier(column))
        {
            return Err(Error::InvalidInput(
                "content registration requires plain SQL identifiers",
            ));
        }
        let unique = columns.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != columns.len()
            || self
                .tables
                .get(table)
                .is_some_and(|saved| unique.iter().any(|column| saved.contains(column)))
        {
            return Err(Error::InvalidInput("content column is already registered"));
        }
        self.tables.entry(table).or_default().extend(unique);
        Ok(())
    }

    pub fn columns(&self) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        self.tables
            .iter()
            .flat_map(|(table, columns)| columns.iter().map(move |column| (*table, *column)))
    }
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgedTable {
    pub table: String,
    pub rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub tables: Vec<PurgedTable>,
    pub invalidate_content: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::trace::{TraceEvent, TraceEventCodes};
    use std::{cell::RefCell, sync::mpsc, time::Duration};
    thread_local! { static EVENTS: RefCell<Option<mpsc::Sender<&'static str>>> = const { RefCell::new(None) }; }

    #[test]
    fn retention_write_waits_for_policy_change_before_reading_mode() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("retention.sqlite");
        let mut writer = Store::open(&path).unwrap();
        let control = Connection::open(&path).unwrap();
        control.execute_batch("BEGIN IMMEDIATE; INSERT INTO settings VALUES('content_retention','\"metadata_only\"');").unwrap();
        let (send, receive) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            EVENTS.set(Some(send));
            writer.connection.trace_v2(
                TraceEventCodes::SQLITE_TRACE_STMT,
                Some(|event| {
                    if let TraceEvent::Stmt(statement, _) = event {
                        let sql = statement.sql();
                        let signal = if sql.starts_with("BEGIN IMMEDIATE") {
                            Some("begin")
                        } else if sql.starts_with("SELECT value_json FROM settings") {
                            Some("policy")
                        } else {
                            None
                        };
                        if let Some(signal) = signal {
                            EVENTS.with_borrow(|sender| {
                                let _ = sender.as_ref().unwrap().send(signal);
                            });
                        }
                    }
                }),
            );
            let mut meta =
                crate::SessionMeta::new("waited", "claude", crate::SessionSource::Transcript);
            meta.title = Some("Do not retain".into());
            let result = writer.upsert_session(&meta, true);
            writer.connection.trace_v2(TraceEventCodes::empty(), None);
            result
        });
        let first = receive.recv_timeout(Duration::from_secs(3)).unwrap();
        // Release the blocker even if a future change reads policy too early.
        control.execute_batch("COMMIT").unwrap();
        handle.join().unwrap().unwrap();
        assert_eq!(first, "begin");
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(3)).unwrap(),
            "policy"
        );
        let reopened = Store::open(&path).unwrap();
        assert!(
            reopened
                .session("waited")
                .unwrap()
                .unwrap()
                .meta
                .title
                .is_none()
        );
    }
}
