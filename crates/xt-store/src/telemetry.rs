//! Saved product-analytics settings and the local delivery ledger.
//!
//! The consent facts live in the existing `settings` table, one key each: the
//! on/off preference (absent reads as on, the documented default) and whether
//! the disclosure was shown (absent reads as not shown). The anonymous install
//! ID, the last local day a daily heartbeat was accepted and the time the last
//! hourly heartbeat was accepted live there too.
//! A stored value this build cannot read is an error, never a silent default.
//!
//! The ledger (`telemetry_events`) keeps the last [`LEDGER_CAPACITY`] events
//! the app tried to send, with the properties sent and a delivery status.

use crate::{Error, Host, Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

const ENABLED: &str = "telemetry.enabled";
const DISCLOSURE_SHOWN: &str = "telemetry.disclosure_shown";
const DISTINCT_ID: &str = "telemetry.distinct_id";
const DAILY_ACTIVE_DAY: &str = "telemetry.daily_active_day";
const HOURLY_ACTIVE_AT: &str = "telemetry.hourly_active_at";

/// How many ledger rows are kept; older rows are removed on insert.
pub const LEDGER_CAPACITY: i64 = 200;

/// The two saved consent facts, read together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TelemetrySettings {
    /// The saved preference; on when nothing was saved.
    pub enabled: bool,
    /// Whether the disclosure was actually shown; false when nothing was saved.
    pub disclosure_shown: bool,
}

/// A ledger row's delivery status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Sent,
    Failed,
}

impl DeliveryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }
    fn parse(text: &str) -> Result<Self> {
        match text {
            "pending" => Ok(Self::Pending),
            "sent" => Ok(Self::Sent),
            "failed" => Ok(Self::Failed),
            _ => Err(Error::InvalidInput("stored telemetry status is invalid")),
        }
    }
}

/// One ledger row as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryLedgerRow {
    pub id: i64,
    pub recorded_at_ms: i64,
    pub event: String,
    pub properties_json: String,
    pub status: DeliveryStatus,
}

impl Store {
    pub fn telemetry_settings(&self) -> Result<TelemetrySettings> {
        Ok(TelemetrySettings {
            enabled: read_bool(&self.connection, ENABLED)?.unwrap_or(true),
            disclosure_shown: read_bool(&self.connection, DISCLOSURE_SHOWN)?.unwrap_or(false),
        })
    }

    /// Saves the preference and returns both facts read back after commit.
    pub fn set_telemetry_enabled(&mut self, enabled: bool) -> Result<TelemetrySettings> {
        self.write_setting(ENABLED, &enabled.to_string())?;
        self.telemetry_settings()
    }

    /// Records that the disclosure was shown. It is never unset.
    pub fn set_telemetry_disclosure_shown(&mut self) -> Result<TelemetrySettings> {
        self.write_setting(DISCLOSURE_SHOWN, "true")?;
        self.telemetry_settings()
    }

    /// The saved anonymous install ID, or the one `generate` returns saved in
    /// the same transaction when none was saved yet.
    pub fn telemetry_distinct_id(&mut self, generate: impl FnOnce() -> String) -> Result<String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(saved) = read_string(&transaction, DISTINCT_ID)? {
            transaction.commit()?;
            return Ok(saved);
        }
        let id = generate();
        if id.trim().is_empty() || id.len() > 64 {
            return Err(Error::InvalidInput("telemetry install ID is invalid"));
        }
        upsert(&transaction, DISTINCT_ID, &serde_json::to_string(&id)?)?;
        transaction.commit()?;
        Ok(id)
    }

    /// The local day (`YYYY-MM-DD`) of the last accepted daily heartbeat.
    pub fn telemetry_daily_active_day(&self) -> Result<Option<String>> {
        read_string(&self.connection, DAILY_ACTIVE_DAY)
    }

    pub fn set_telemetry_daily_active_day(&mut self, day: &str) -> Result<()> {
        if day.len() != 10 {
            return Err(Error::InvalidInput("telemetry day must be YYYY-MM-DD"));
        }
        self.write_setting(DAILY_ACTIVE_DAY, &serde_json::to_string(day)?)
    }

    /// When the last hourly heartbeat was accepted, in Unix milliseconds.
    pub fn telemetry_hourly_active_at(&self) -> Result<Option<i64>> {
        read_raw(&self.connection, HOURLY_ACTIVE_AT)?
            .map(|value| {
                value
                    .as_i64()
                    .filter(|at| *at >= 0)
                    .ok_or(Error::InvalidInput("stored telemetry setting is invalid"))
            })
            .transpose()
    }

    pub fn set_telemetry_hourly_active_at(&mut self, at_ms: i64) -> Result<()> {
        if at_ms < 0 {
            return Err(Error::InvalidInput("telemetry time must not be negative"));
        }
        self.write_setting(HOURLY_ACTIVE_AT, &at_ms.to_string())
    }

    /// Adds a pending row and removes rows beyond the newest
    /// [`LEDGER_CAPACITY`], in one transaction. Returns the new row's ID.
    pub fn telemetry_record_pending(
        &mut self,
        recorded_at_ms: i64,
        event: &str,
        properties_json: &str,
    ) -> Result<i64> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO telemetry_events(recorded_at,event,properties_json,status)
             VALUES(?1,?2,?3,'pending')",
            params![recorded_at_ms, event, properties_json],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.execute(
            "DELETE FROM telemetry_events WHERE id <= ?1",
            [id - LEDGER_CAPACITY],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    /// Moves a pending row to `status`. A row that is no longer pending, or
    /// that the ring already removed, is left alone; returns whether it moved.
    pub fn telemetry_mark(&mut self, id: i64, status: DeliveryStatus) -> Result<bool> {
        if status == DeliveryStatus::Pending {
            return Err(Error::InvalidInput("a ledger row cannot return to pending"));
        }
        Ok(self.connection.execute(
            "UPDATE telemetry_events SET status=?2 WHERE id=?1 AND status='pending'",
            params![id, status.as_str()],
        )? == 1)
    }

    /// Every row still pending becomes failed: work left by an earlier run is
    /// never sent again. Returns how many rows changed.
    pub fn telemetry_fail_pending(&mut self) -> Result<usize> {
        Ok(self.connection.execute(
            "UPDATE telemetry_events SET status='failed' WHERE status='pending'",
            [],
        )?)
    }

    /// The newest rows first, at most `limit`.
    pub fn telemetry_ledger(&self, limit: u32) -> Result<Vec<TelemetryLedgerRow>> {
        let mut statement = self.connection.prepare(
            "SELECT id,recorded_at,event,properties_json,status FROM telemetry_events
             ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, recorded_at_ms, event, properties_json, status)| {
                Ok(TelemetryLedgerRow {
                    id,
                    recorded_at_ms,
                    event,
                    properties_json,
                    status: DeliveryStatus::parse(&status)?,
                })
            })
            .collect()
    }

    /// Every host with at least one indexed session, each once.
    pub fn indexed_hosts(&self) -> Result<Vec<Host>> {
        let mut statement = self
            .connection
            .prepare("SELECT DISTINCT host FROM sessions ORDER BY host")?;
        let hosts = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(hosts.iter().map(|host| Host::from_platform(host)).collect())
    }

    fn write_setting(&mut self, key: &str, value_json: &str) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert(&transaction, key, value_json)?;
        transaction.commit()?;
        Ok(())
    }
}

fn upsert(connection: &rusqlite::Connection, key: &str, value_json: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
        params![key, value_json],
    )?;
    Ok(())
}

fn read_raw(connection: &rusqlite::Connection, key: &str) -> Result<Option<serde_json::Value>> {
    let saved: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(saved
        .map(|saved| serde_json::from_str(&saved))
        .transpose()?)
}

/// Only a JSON boolean is accepted.
fn read_bool(connection: &rusqlite::Connection, key: &str) -> Result<Option<bool>> {
    read_raw(connection, key)?
        .map(|value| {
            value
                .as_bool()
                .ok_or(Error::InvalidInput("stored telemetry setting is invalid"))
        })
        .transpose()
}

/// Only a JSON string is accepted.
fn read_string(connection: &rusqlite::Connection, key: &str) -> Result<Option<String>> {
    read_raw(connection, key)?
        .map(|value| match value {
            serde_json::Value::String(text) => Ok(text),
            _ => Err(Error::InvalidInput("stored telemetry setting is invalid")),
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_is_on_and_not_yet_disclosed() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.telemetry_settings().unwrap(),
            TelemetrySettings {
                enabled: true,
                disclosure_shown: false
            }
        );
        assert_eq!(store.telemetry_daily_active_day().unwrap(), None);
    }

    #[test]
    fn saved_facts_survive_reopening() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("telemetry.sqlite");
        let mut store = Store::open(&path).unwrap();
        store.set_telemetry_disclosure_shown().unwrap();
        let saved = store.set_telemetry_enabled(false).unwrap();
        assert_eq!(
            saved,
            TelemetrySettings {
                enabled: false,
                disclosure_shown: true
            }
        );
        let id = store.telemetry_distinct_id(|| "first".into()).unwrap();
        store.set_telemetry_daily_active_day("2026-10-12").unwrap();
        assert_eq!(store.telemetry_hourly_active_at().unwrap(), None);
        store
            .set_telemetry_hourly_active_at(1_800_000_000_000)
            .unwrap();
        assert!(store.set_telemetry_hourly_active_at(-1).is_err());
        drop(store);
        let mut reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.telemetry_settings().unwrap(), saved);
        assert_eq!(
            reopened
                .telemetry_distinct_id(|| unreachable!("the saved ID is kept"))
                .unwrap(),
            id
        );
        assert_eq!(
            reopened.telemetry_daily_active_day().unwrap().as_deref(),
            Some("2026-10-12")
        );
        assert_eq!(
            reopened.telemetry_hourly_active_at().unwrap(),
            Some(1_800_000_000_000)
        );
    }

    #[test]
    fn stored_values_of_the_wrong_type_fail_visibly() {
        let store = Store::open_in_memory().unwrap();
        for value in ["1", "\"true\"", "null", "{}"] {
            upsert(&store.connection, ENABLED, value).unwrap();
            assert!(store.telemetry_settings().is_err(), "{value}");
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_two_hundred() {
        let mut store = Store::open_in_memory().unwrap();
        let mut ids = Vec::new();
        for n in 0..205 {
            ids.push(
                store
                    .telemetry_record_pending(n, "app_opened", "{}")
                    .unwrap(),
            );
            let count: i64 = store
                .connection
                .query_row("SELECT count(*) FROM telemetry_events", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!(count <= LEDGER_CAPACITY);
        }
        let rows = store.telemetry_ledger(1000).unwrap();
        assert_eq!(rows.len(), 200);
        assert_eq!(rows.first().unwrap().id, *ids.last().unwrap());
        assert_eq!(rows.last().unwrap().id, ids[5]);
        // A removed row cannot be marked.
        assert!(!store.telemetry_mark(ids[0], DeliveryStatus::Sent).unwrap());
    }

    #[test]
    fn a_row_leaves_pending_once() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store
            .telemetry_record_pending(1, "daily_active", r#"{"surface":"desktop"}"#)
            .unwrap();
        assert!(store.telemetry_mark(id, DeliveryStatus::Sent).unwrap());
        assert!(!store.telemetry_mark(id, DeliveryStatus::Failed).unwrap());
        assert!(store.telemetry_mark(id, DeliveryStatus::Pending).is_err());
        let other = store
            .telemetry_record_pending(2, "app_opened", "{}")
            .unwrap();
        assert_eq!(store.telemetry_fail_pending().unwrap(), 1);
        let rows = store.telemetry_ledger(10).unwrap();
        assert_eq!(
            rows.iter().map(|r| (r.id, r.status)).collect::<Vec<_>>(),
            [(other, DeliveryStatus::Failed), (id, DeliveryStatus::Sent)]
        );
    }

    #[test]
    fn indexed_hosts_lists_each_host_once() {
        let mut store = Store::open_in_memory().unwrap();
        assert!(store.indexed_hosts().unwrap().is_empty());
        for (id, platform) in [
            ("a", "codex"),
            ("b", "claude"),
            ("c", "codex"),
            ("d", "other-x"),
        ] {
            store
                .upsert_session(
                    &crate::SessionMeta::new(id, platform, crate::SessionSource::Plugin),
                    false,
                )
                .unwrap();
        }
        assert_eq!(
            store.indexed_hosts().unwrap(),
            [Host::Claude, Host::Codex, Host::Other]
        );
    }

    #[test]
    fn the_table_refuses_unknown_statuses_and_non_object_properties() {
        let store = Store::open_in_memory().unwrap();
        for (status, props) in [("cancelled", "{}"), ("pending", "[]"), ("pending", "x")] {
            assert!(
                store
                    .connection
                    .execute(
                        "INSERT INTO telemetry_events(recorded_at,event,properties_json,status)
                         VALUES(1,'app_opened',?1,?2)",
                        params![props, status],
                    )
                    .is_err(),
                "{status} {props}"
            );
        }
    }
}
