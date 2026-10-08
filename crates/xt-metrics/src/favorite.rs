use crate::{Error, MetricsDb, Result, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const OUTPUT_QUERY: &str =
    "SELECT model,output_tokens,ts FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2";
pub(crate) const TURN_QUERY: &str = "SELECT session_id,uuid,ts,is_human,role,model,confirmed_automated_input FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FavoriteUnknown {
    NoMeasuredOutput,
    UnknownModelOutput,
    UnknownTurnAttribution,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FavoriteModel {
    pub model: Option<String>,
    pub output_tokens: Option<u64>,
    pub unknown_reason: Option<FavoriteUnknown>,
}
impl FavoriteModel {
    fn unknown(reason: FavoriteUnknown) -> Self {
        Self {
            model: None,
            output_tokens: None,
            unknown_reason: Some(reason),
        }
    }
    fn measured(model: String, output: u64) -> Self {
        Self {
            model: Some(model),
            output_tokens: Some(output),
            unknown_reason: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FavoriteComparison {
    pub current: FavoriteModel,
    pub previous: FavoriteModel,
}
struct Event {
    instant: InstantKey,
    uuid: String,
    human: Option<bool>,
    role: Option<String>,
    model: Option<String>,
    /// A confirmed automated input, which is no turn boundary: M-03 skips it.
    automated: bool,
}
/// The in-window responses that measured output, by named model.
#[derive(Default)]
pub(crate) struct Outputs {
    models: BTreeMap<String, u64>,
    unknown_output: u64,
    unknown_identity: bool,
    measured: bool,
}
impl Outputs {
    pub(crate) fn push(&mut self, model: Option<&str>, output: u64) -> Result<()> {
        self.measured = true;
        if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
            let total = self.models.entry(model.to_owned()).or_default();
            *total = total.checked_add(output).ok_or(Error::CounterOverflow)?;
        } else {
            self.unknown_identity = true;
            self.unknown_output = self
                .unknown_output
                .checked_add(output)
                .ok_or(Error::CounterOverflow)?;
        }
        Ok(())
    }
}
fn instant(raw: &str, column: usize) -> Result<InstantKey> {
    Ok(timestamp::parse(raw)
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?
        .0)
}
impl MetricsDb {
    /// Measured output observations participate independently of other missing
    /// counters. A provable output winner never depends on uncertain turn facts.
    pub fn favorite_model(&self, window: Window) -> Result<FavoriteModel> {
        let snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let report = self.favorite_in_snapshot(window)?;
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(report)
    }
    /// Categorical comparison, with an equal-elapsed preceding event window.
    pub fn favorite_comparison(&self, window: Window) -> Result<FavoriteComparison> {
        let previous = window.previous()?;
        let snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let report = FavoriteComparison {
            current: self.favorite_model(window)?,
            previous: self.favorite_model(previous)?,
        };
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(report)
    }
    pub fn favorite_iso_week(&self, anchor_ms: i64, zone: TimeZone) -> Result<FavoriteModel> {
        self.favorite_model(Window::iso_week(anchor_ms, zone)?)
    }
    fn favorite_in_snapshot(&self, window: Window) -> Result<FavoriteModel> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut outputs = Outputs::default();
        let mut statement = self.connection.prepare(OUTPUT_QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let ts = instant(&row.get::<_, String>(2)?, 2)?;
            if ts < start || ts >= end {
                continue;
            }
            let Some(output) = row.get::<_, Option<i64>>(1)? else {
                continue;
            };
            let output = u64::try_from(output)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, output))?;
            outputs.push(row.get::<_, Option<String>>(0)?.as_deref(), output)?;
        }
        drop(rows);
        drop(statement);
        self.favorite_from(window, outputs)
    }
    /// [`MetricsDb::favorite_model`] from the window's measured outputs,
    /// inside the caller's snapshot; a tie reads the window's turns.
    pub(crate) fn favorite_from(&self, window: Window, outputs: Outputs) -> Result<FavoriteModel> {
        let Outputs {
            models,
            unknown_output,
            unknown_identity,
            measured,
        } = outputs;
        if !measured {
            return Ok(FavoriteModel::unknown(FavoriteUnknown::NoMeasuredOutput));
        }
        let Some(&leader) = models.values().max() else {
            return Ok(FavoriteModel::unknown(FavoriteUnknown::UnknownModelOutput));
        };
        let contenders: Vec<String> = models
            .iter()
            .filter(|(_, value)| **value == leader)
            .map(|(model, _)| model.clone())
            .collect();
        if unknown_identity && (leader == 0 || unknown_output > 0) {
            let rival = models
                .iter()
                .filter(|(model, _)| *model != &contenders[0])
                .map(|(_, value)| *value)
                .max()
                .unwrap_or(0);
            if contenders.len() != 1 || leader.saturating_sub(rival) <= unknown_output {
                return Ok(FavoriteModel::unknown(FavoriteUnknown::UnknownModelOutput));
            }
        }
        if contenders.len() == 1 {
            return Ok(FavoriteModel::measured(contenders[0].clone(), leader));
        }
        let Some(turns) = self.favorite_turns(window, &contenders)? else {
            return Ok(FavoriteModel::unknown(
                FavoriteUnknown::UnknownTurnAttribution,
            ));
        };
        // BTreeMap's lexical order resolves only fully measured turn ties.
        let peak = turns.values().copied().max().unwrap_or(0);
        let model = turns
            .into_iter()
            .find(|(_, count)| *count == peak)
            .unwrap()
            .0;
        Ok(FavoriteModel::measured(model, leader))
    }
    fn favorite_turns(
        &self,
        window: Window,
        contenders: &[String],
    ) -> Result<Option<BTreeMap<String, u64>>> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut sessions = BTreeMap::<String, Vec<Event>>::new();
        let mut statement = self.connection.prepare(TURN_QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let ts = instant(&row.get::<_, String>(2)?, 2)?;
            if ts < start || ts >= end {
                continue;
            }
            sessions.entry(row.get(0)?).or_default().push(Event {
                instant: ts,
                uuid: row.get(1)?,
                human: row.get(3)?,
                role: row.get(4)?,
                model: row.get(5)?,
                automated: row.get(6)?,
            });
        }
        let mut turns: BTreeMap<_, u64> =
            contenders.iter().cloned().map(|model| (model, 0)).collect();
        for events in sessions.values_mut() {
            events.sort_unstable_by(|a, b| {
                a.instant.cmp(&b.instant).then_with(|| a.uuid.cmp(&b.uuid))
            });
            let mut pending = false;
            let mut uncertain_boundary = false;
            // M-03's boundaries: a confirmed automated input neither opens a
            // turn nor answers one, exactly as the turn count reads it.
            for event in events.iter().filter(|event| !event.automated) {
                let may_be_assistant = event.role.as_deref().is_none_or(|role| role == "assistant");
                match event.human {
                    Some(true) => {
                        pending = true;
                        uncertain_boundary = false;
                    }
                    None => {
                        if pending && may_be_assistant {
                            return Ok(None);
                        }
                        if !pending {
                            uncertain_boundary = true;
                        }
                    }
                    Some(false) => {
                        if uncertain_boundary && may_be_assistant {
                            return Ok(None);
                        }
                        if pending {
                            match event.role.as_deref() {
                                None => return Ok(None),
                                Some("assistant") => {
                                    let Some(model) = event
                                        .model
                                        .as_ref()
                                        .filter(|model| !model.trim().is_empty())
                                    else {
                                        return Ok(None);
                                    };
                                    if let Some(count) = turns.get_mut(model) {
                                        *count =
                                            count.checked_add(1).ok_or(Error::CounterOverflow)?;
                                    }
                                    pending = false;
                                }
                                Some(_) => {}
                            }
                        }
                    }
                }
            }
        }
        Ok(Some(turns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicBool, Ordering},
    };
    #[test]
    fn favorite_comparison_and_turn_reads_share_snapshot_and_reuse_transaction() {
        let f = xt_fixtures::Fixture::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F11"),
        )
        .unwrap();
        let mut db = xt_fixtures::TempDb::empty().unwrap();
        for session in f.snapshots()["favorite"]["sessions"].as_array().unwrap() {
            let mut metadata = f.sessions()[0].metadata.clone();
            metadata.session_id = session["session_id"].as_str().unwrap().into();
            db.store_mut().upsert_session(&metadata, false).unwrap();
            let rows: Vec<xt_store::CanonicalRecord> =
                serde_json::from_value(session["records"].clone()).unwrap();
            db.store_mut()
                .upsert_records(&metadata.session_id, &rows, false)
                .unwrap();
        }
        let metrics = MetricsDb::open(db.path()).unwrap();
        let writer = Connection::open(db.path()).unwrap();
        writer.execute_batch("DROP VIEW v_response_usage; CREATE VIEW v_response_usage AS SELECT model,output_tokens,favorite_snapshot_write(ts) AS ts,ts_ms FROM v_usage_records").unwrap();
        let path = db.path().to_owned();
        let written = AtomicBool::new(false);
        metrics
            .connection
            .create_scalar_function(
                "favorite_snapshot_write",
                1,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    if !written.swap(true, Ordering::SeqCst) {
                        Connection::open(&path)?
                            .execute("UPDATE records SET model='B' WHERE model='A'", [])?;
                    }
                    context.get::<String>(0)
                },
            )
            .unwrap();
        let window = Window::new(1788739200000, 1789344000000).unwrap(); // 2026-09-07 through 09-14.
        let report = metrics.favorite_comparison(window).unwrap();
        assert_eq!(report.current.model.as_deref(), Some("B"));
        assert_eq!(report.current.output_tokens, Some(30));
        assert_eq!(report.previous.model.as_deref(), Some("A"));
        assert!(metrics.connection.is_autocommit());
        assert_eq!(
            metrics
                .favorite_comparison(window)
                .unwrap()
                .previous
                .model
                .as_deref(),
            Some("B")
        );
        let transaction = metrics.connection.unchecked_transaction().unwrap();
        assert_eq!(
            metrics
                .favorite_comparison(window)
                .unwrap()
                .previous
                .model
                .as_deref(),
            Some("B")
        );
        assert!(!metrics.connection.is_autocommit());
        transaction.commit().unwrap();
    }
}
