//! Metadata-only Human input corrections; never changes canonical records.
use crate::{Error, Host, Result, Store};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionOrigin {
    pub session_id: String,
    pub host: Host,
    pub native_session_id: String,
    pub parent_host: Host,
    pub parent_native_session_id: String,
    pub method: String,
    pub evidence_id: String,
    pub launch_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginManifest {
    pub version: u32,
    pub sessions: Vec<SessionOrigin>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CorrectionReport {
    pub applied: usize,
    pub unchanged: usize,
    pub conflicted: usize,
    pub failed: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputAdjustment {
    pub record_uuid: String,
    pub session_id: String,
    pub original_ts: String,
    pub original_length: i64,
    /// None excludes the whole message. Some(0) retains an empty input.
    pub retained_length: Option<i64>,
    pub reason: String,
    pub native_item_id: Option<String>,
}
impl CorrectionReport {
    pub fn merge(&mut self, other: &Self) {
        self.applied += other.applied;
        self.unchanged += other.unchanged;
        self.conflicted += other.conflicted;
        self.failed += other.failed;
    }
}
type OriginState = (String, String, String, String, String, String, String, bool);
type AdjustmentState = (
    String,
    String,
    i64,
    Option<i64>,
    String,
    Option<String>,
    bool,
);
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b'\\'))
}
impl Store {
    pub fn import_human_session_origins(
        &mut self,
        manifest: &OriginManifest,
    ) -> Result<CorrectionReport> {
        if manifest.version != 1 || manifest.sessions.len() > 10_000 {
            return Err(Error::InvalidInput(
                "unsupported or oversized Human origin manifest",
            ));
        }
        let tx = self.connection.transaction()?;
        let mut unique = BTreeMap::new();
        for origin in &manifest.sessions {
            if !matches!(
                origin.method.as_str(),
                "returned_child_id"
                    | "explicit_session_id"
                    | "artifact_result_chain"
                    | "copied_worker_result"
                    | "exact_initial_text"
                    | "native_reviewer_header"
            ) || ![
                &origin.session_id,
                &origin.native_session_id,
                &origin.parent_native_session_id,
                &origin.evidence_id,
                &origin.launch_id,
            ]
            .into_iter()
            .all(|s| token(s))
                || (origin.host == origin.parent_host
                    && origin.native_session_id == origin.parent_native_session_id)
            {
                return Err(Error::InvalidInput(
                    "invalid Human origin identity or method",
                ));
            }
            if let Some(previous) = unique.insert(&origin.session_id, origin)
                && previous != origin
            {
                return Err(Error::InvalidInput(
                    "contradictory Human origin manifest entries",
                ));
            }
            let session = crate::read::session(&tx, &origin.session_id)?
                .ok_or(Error::InvalidInput("Human origin child is not indexed"))?;
            let identities: i64 = tx.query_row(
                "SELECT count(*) FROM sessions WHERE host=?1 AND native_session_id=?2",
                params![origin.host, origin.native_session_id],
                |row| row.get(0),
            )?;
            if identities != 1
                || session.meta.host != origin.host
                || session.meta.native_session_id.as_deref() != Some(&origin.native_session_id)
            {
                return Err(Error::InvalidInput("Human origin child identity mismatch"));
            }
        }
        let mut report = CorrectionReport::default();
        for origin in unique.values() {
            let existing: Option<OriginState> = tx.query_row(
                "SELECT host,native_session_id,parent_host,parent_native_session_id,method,evidence_id,launch_id,conflicted FROM human_session_origins WHERE session_id=?1", [&origin.session_id],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
            let expected = (
                origin.host.as_str().to_owned(),
                origin.native_session_id.clone(),
                origin.parent_host.as_str().to_owned(),
                origin.parent_native_session_id.clone(),
                origin.method.clone(),
                origin.evidence_id.clone(),
                origin.launch_id.clone(),
                false,
            );
            match existing {
                Some(row) if row == expected => report.unchanged += 1,
                Some(_) => {
                    tx.execute(
                        "UPDATE human_session_origins SET conflicted=1,conflict_reason='evidence_mismatch' WHERE session_id=?1",
                        [&origin.session_id],
                    )?;
                    report.conflicted += 1;
                }
                None => {
                    tx.execute("INSERT INTO human_session_origins(session_id,host,native_session_id,parent_host,parent_native_session_id,method,evidence_id,launch_id,rule_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1)", params![origin.session_id,origin.host,origin.native_session_id,origin.parent_host,origin.parent_native_session_id,origin.method,origin.evidence_id,origin.launch_id])?;
                    report.applied += 1;
                }
            }
        }
        tx.commit()?;
        Ok(report)
    }
    /// Applies only after the current conversion binds to the saved row.
    pub fn apply_human_input_adjustments(
        &mut self,
        inputs: &[InputAdjustment],
    ) -> Result<CorrectionReport> {
        let tx = self.connection.transaction()?;
        let mut report = CorrectionReport::default();
        for input in inputs {
            if !matches!(
                input.reason.as_str(),
                "heartbeat" | "selected_skill" | "question_reply" | "image_wrapper"
            ) || input.original_length < 0
                || input
                    .retained_length
                    .is_some_and(|n| n < 0 || n > input.original_length)
            {
                return Err(Error::InvalidInput("invalid Human input adjustment"));
            }
            let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM records r JOIN sessions s ON s.session_id=r.session_id WHERE r.uuid=?1 AND r.session_id=?2 AND r.ts=?3 AND r.text_len=?4 AND r.has_conflict=0 AND s.host='codex' AND r.role='user' AND r.is_human=1)", params![input.record_uuid,input.session_id,input.original_ts,input.original_length], |r| r.get(0))?;
            let held: Option<AdjustmentState> = tx.query_row("SELECT session_id,original_ts,original_length,retained_length,reason,native_item_id,conflicted FROM human_input_adjustments WHERE record_uuid=?1", [&input.record_uuid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
            let expected = (
                input.session_id.clone(),
                input.original_ts.clone(),
                input.original_length,
                input.retained_length,
                input.reason.clone(),
                input.native_item_id.clone(),
                false,
            );
            if !bound || held.as_ref().is_some_and(|row| *row != expected) {
                tx.execute("INSERT OR IGNORE INTO human_input_adjustments(record_uuid,session_id,original_ts,original_length,retained_length,reason,rule_version,native_item_id,conflicted) SELECT uuid,session_id,ts,text_len,NULL,?2,1,?3,1 FROM records WHERE uuid=?1 AND ts IS NOT NULL AND text_len IS NOT NULL", params![input.record_uuid,input.reason,input.native_item_id])?;
                tx.execute(
                    "UPDATE human_input_adjustments SET conflicted=1,conflict_reason='saved_binding_mismatch' WHERE record_uuid=?1",
                    [&input.record_uuid],
                )?;
                report.conflicted += 1;
            } else if held.is_some() {
                report.unchanged += 1;
            } else {
                tx.execute("INSERT INTO human_input_adjustments(record_uuid,session_id,original_ts,original_length,retained_length,reason,rule_version,native_item_id) VALUES(?1,?2,?3,?4,?5,?6,1,?7)", params![input.record_uuid,input.session_id,input.original_ts,input.original_length,input.retained_length,input.reason,input.native_item_id])?;
                report.applied += 1;
            }
            // Whatever became of the adjustment, it says the record's joined
            // text is not simply a person's words: keep no preview of it.
            crate::record_preview::withdraw_person(&tx, &input.record_uuid)?;
        }
        tx.commit()?;
        Ok(report)
    }
}
