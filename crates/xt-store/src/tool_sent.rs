//! Source proofs that a saved Codex or Cursor user input was written by the
//! tool, not typed by a person.
//!
//! The pinned reader recognises a closed set of such inputs from structural
//! markers in the native source (see [`ToolSentKind`]) and hands a claim
//! beside the canonical record it converted from that same native row. The
//! ingest layer validates the claim against the line's own record and
//! session and passes only its kind here
//! ([`crate::batch::IngestBatch::tool_sent`]); nothing here looks at the
//! record's text.
//!
//! As for a Claude task notification, a proof binds to the stored record with
//! the input's UUID, in the batch's own transaction, whether this input
//! inserted it or the index already held it: the claim belongs to the very
//! native row the UUID names, so reading that row again is evidence about the
//! same record. Every Codex and Cursor scan reads every session again, which
//! is what corrects inputs indexed before the claim was read. A proof binds
//! only where the input was accepted and the stored row is an unconflicted,
//! human-classified user input of the batch's own session on the kind's own
//! host; anything else abstains and the record stays exactly as ingestion
//! stored it. Raw classification, the record, its usage and its tools are
//! never changed; the shared record projection reads the proof as a durable
//! override of that one input's human eligibility.

use crate::{
    Host, Result,
    batch::{AffectedSession, IngestBatch, RecordOutcome},
};
use rusqlite::{Connection, params};

const RULE_VERSION: i64 = 1;

/// The closed kinds of tool-sent input this build keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolSentKind {
    /// Codex announcing that a subagent it started finished
    /// (`multi_agent.subagent_notification`).
    CodexSubagentNotification,
    /// Codex's note that the previous turn was interrupted
    /// (`generic.turn_aborted`).
    CodexTurnAborted,
    /// The Codex app's own record of a page it opened
    /// (`additional_content.codex_apps_open_page`).
    CodexAppsOpenPage,
    /// The summary Cursor writes when it compacts a conversation
    /// (`providerOptions.cursor.isSummary`).
    CursorConversationSummary,
    /// The `[Imported from Cursor ...]` record the reader itself writes first
    /// in every Cursor session.
    CursorImportBanner,
}

impl ToolSentKind {
    pub const ALL: [Self; 5] = [
        Self::CodexSubagentNotification,
        Self::CodexTurnAborted,
        Self::CodexAppsOpenPage,
        Self::CursorConversationSummary,
        Self::CursorImportBanner,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CodexSubagentNotification => "codex_subagent_notification",
            Self::CodexTurnAborted => "codex_turn_aborted",
            Self::CodexAppsOpenPage => "codex_apps_open_page",
            Self::CursorConversationSummary => "cursor_conversation_summary",
            Self::CursorImportBanner => "cursor_import_banner",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// The only host whose sessions this kind may name.
    pub fn host(self) -> Host {
        match self {
            Self::CodexSubagentNotification | Self::CodexTurnAborted | Self::CodexAppsOpenPage => {
                Host::Codex
            }
            Self::CursorConversationSummary | Self::CursorImportBanner => Host::Cursor,
        }
    }
}

/// Keep a proof for every accepted input the batch marks as tool-sent, and
/// name the owning session of each newly proven record in `affected` so its
/// measurements are read again. Returns how many proofs were newly kept.
pub(crate) fn apply(
    connection: &Connection,
    batch: &IngestBatch<'_>,
    outcomes: &[RecordOutcome],
    affected: &mut Vec<AffectedSession>,
) -> Result<usize> {
    if batch.tool_sent.is_empty() {
        return Ok(0);
    }
    if batch.tool_sent.len() != batch.records.len() {
        return Err(crate::Error::InvalidInput(
            "tool-sent markers must align with the batch's inputs",
        ));
    }
    let mut kept = 0;
    for outcome in outcomes {
        let Some(kind) = batch.tool_sent[outcome.input_index] else {
            continue;
        };
        if !outcome.disposition.is_accepted()
            || kind.host() != batch.session.host
            || batch.records[outcome.input_index].message.role.as_deref() != Some("user")
        {
            continue;
        }
        let Some(uuid) = outcome.uuid.as_deref() else {
            continue;
        };
        let added = connection.execute(
            "INSERT OR IGNORE INTO tool_sent_inputs(record_uuid,session_id,evidence_kind,rule_version)
             SELECT r.uuid,r.session_id,?3,?4 FROM records r JOIN sessions s ON s.session_id=r.session_id
             WHERE r.uuid=?1 AND r.session_id=?2 AND r.type='user' AND r.role='user'
               AND r.is_meta=0 AND r.has_conflict=0 AND r.is_human=1 AND s.host=?5",
            params![
                uuid,
                batch.session.session_id,
                kind.as_str(),
                RULE_VERSION,
                kind.host().as_str()
            ],
        )?;
        if added == 0 {
            continue;
        }
        kept += 1;
        let surface: Option<String> = connection.query_row(
            "SELECT surface FROM sessions WHERE session_id=?1",
            [&batch.session.session_id],
            |row| row.get(0),
        )?;
        if !affected
            .iter()
            .any(|owner| owner.session_id == batch.session.session_id)
        {
            affected.push(AffectedSession {
                session_id: batch.session.session_id.clone(),
                surface,
            });
        }
    }
    Ok(kept)
}

/// Whether a stored proof names this record.
pub(crate) fn proven(connection: &Connection, uuid: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM tool_sent_inputs WHERE record_uuid=?1)",
        [uuid],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_name_their_host() {
        for kind in ToolSentKind::ALL {
            assert_eq!(ToolSentKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ToolSentKind::parse("claude_task_notification"), None);
        assert_eq!(
            ToolSentKind::CodexTurnAborted.host(),
            Host::Codex,
            "a Codex kind names Codex sessions"
        );
        assert_eq!(ToolSentKind::CursorImportBanner.host(), Host::Cursor);
    }
}
