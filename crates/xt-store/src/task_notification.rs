//! Source proofs that a saved Claude Code user input is a task notification.
//!
//! When a background agent, command or monitor that Claude Code started
//! finishes, Claude Code writes a `type: "user"` line into the conversation
//! announcing it, and marks that line structurally: `origin.kind` is
//! `"task-notification"`. The text is Claude Code's, not a person's, so the
//! input is automatic. The reader passes the marker beside the canonical
//! record it converted from the same line ([`crate::batch::IngestBatch::task_notifications`]);
//! nothing here looks at the record's text.
//!
//! A proof binds to the stored record with the input's UUID, in the batch's
//! own transaction, whether this input inserted it or the index already held
//! it: the marker belongs to the very line the UUID names, so reading that line
//! again is evidence about the same record. That is what lets a one-time
//! transcript replay correct the inputs indexed before the marker was read.
//! It binds only where the input was accepted and the stored row is an
//! unconflicted, human-classified Claude user input; anything else abstains
//! and the record stays exactly as ingestion stored it. Raw classification,
//! the record, its usage and its tools are never changed; the shared record
//! projection reads the proof as a durable override of that one input's human
//! eligibility, exactly as it reads a confirmation or an injected-context
//! proof.

use crate::{
    Result,
    batch::{AffectedSession, IngestBatch, RecordOutcome},
};
use rusqlite::{Connection, params};

/// The only evidence kind and rule version this build writes.
const EVIDENCE_KIND: &str = "claude_task_notification";
const RULE_VERSION: i64 = 1;

/// Keep a proof for every accepted input the batch marks as a task
/// notification, and name the owning session of each newly proven record in
/// `affected` so its measurements are read again. Returns how many proofs
/// were newly kept.
pub(crate) fn apply(
    connection: &Connection,
    batch: &IngestBatch<'_>,
    outcomes: &[RecordOutcome],
    affected: &mut Vec<AffectedSession>,
) -> Result<usize> {
    if batch.task_notifications.is_empty() {
        return Ok(0);
    }
    if batch.task_notifications.len() != batch.records.len() {
        return Err(crate::Error::InvalidInput(
            "task notification markers must align with the batch's inputs",
        ));
    }
    let mut kept = 0;
    for outcome in outcomes {
        if !outcome.disposition.is_accepted()
            || !batch.task_notifications[outcome.input_index]
            || batch.records[outcome.input_index].message.role.as_deref() != Some("user")
        {
            continue;
        }
        let Some(uuid) = outcome.uuid.as_deref() else {
            continue;
        };
        let added = connection.execute(
            "INSERT OR IGNORE INTO task_notification_inputs(record_uuid,session_id,evidence_kind,rule_version)
             SELECT r.uuid,r.session_id,?2,?3 FROM records r JOIN sessions s ON s.session_id=r.session_id
             WHERE r.uuid=?1 AND r.type='user' AND r.role='user' AND r.is_meta=0
               AND r.has_conflict=0 AND r.is_human=1 AND s.host='claude'",
            params![uuid, EVIDENCE_KIND, RULE_VERSION],
        )?;
        if added == 0 {
            continue;
        }
        kept += 1;
        let (session_id, surface): (String, Option<String>) = connection.query_row(
            "SELECT s.session_id,s.surface FROM task_notification_inputs n
             JOIN sessions s ON s.session_id=n.session_id WHERE n.record_uuid=?1",
            [uuid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !affected.iter().any(|owner| owner.session_id == session_id) {
            affected.push(AffectedSession {
                session_id,
                surface,
            });
        }
    }
    Ok(kept)
}
