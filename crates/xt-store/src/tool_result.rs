//! How each tool call ended: the single owner of the facts a `tool_result`
//! block states, of the rule that reads an outcome from those facts, and of
//! the call identifier that links a result to the call it answers.
//!
//! The facts are read from the record's own blocks at write time, as the
//! interruption flag and the structural tool columns are, so a metadata-only
//! index keeps them once the blocks are discarded. Nothing here keeps text: a
//! result's words are read only to recognise a few fixed host messages
//! ([`Wording`]), in memory, and dropped.
//!
//! What the facts mean is decided at read time by [`outcome`], so changing
//! that rule needs no rewrite of saved rows. A saved row is replaced only by
//! an observation with a higher [`FACT_VERSION`]; one with the same version
//! that states other facts marks the row conflicted, and a call observed with
//! two different identifiers links to nothing. Readers treat both as unknown.

use crate::{
    Host, Result,
    batch::RecordOutcome,
    model::{CanonicalRecord, RecordType},
};
use rusqlite::{Connection, params};
use serde_json::Value;

/// The start of the tool result Claude Code writes when the person declines a
/// tool call at its permission prompt. It comes with `is_error: true`.
pub const DECLINED_PREFIX: &str = "The user doesn't want to proceed with this tool use";
/// The start of the tool result Claude Code writes when the person interrupts
/// the agent while a call runs.
pub const INTERRUPTED_PREFIX: &str = "[Request interrupted by user for tool use]";
/// The start of the tool result Claude Code writes when its own permission
/// check, not the person, refused a call (auto mode's denials).
pub const PERMISSION_DENIED_PREFIX: &str = "Permission for this action was denied";
/// Claude Code's other permission refusal, from its settings or mode rather
/// than the person: "Permission to use Bash has been denied …", or "Permission
/// to use Bash with command … has been denied …". The result starts with the
/// first part and states the second.
pub const PERMISSION_TO_USE_PREFIX: &str = "Permission to use ";
pub const PERMISSION_TO_USE_DENIED: &str = " has been denied";
/// The write-time derivation of [`ResultFacts`] this build implements. Bump it
/// only together with a change to [`facts`]; rows written under a lower
/// version are then replaced when their source is read again.
pub const FACT_VERSION: i64 = 1;
/// The longest call identifier kept (the tables' CHECK).
const MAX_CALL_ID_CHARS: usize = 256;

/// Which fixed host message a result's text starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wording {
    Declined,
    Interrupted,
    PermissionDenied,
    Other,
}

impl Wording {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declined => "declined",
            Self::Interrupted => "interrupted",
            Self::PermissionDenied => "permission_denied",
            Self::Other => "other",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "declined" => Self::Declined,
            "interrupted" => Self::Interrupted,
            "permission_denied" => Self::PermissionDenied,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// What one `tool_result` block states, as saved: no text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultFacts {
    pub call_id: Option<String>,
    /// The `is_error` flag when it is a boolean.
    pub is_error: Option<bool>,
    /// Whether the block has an `is_error` field at all; with `is_error`
    /// `None`, a flag that is neither true nor false.
    pub flag_stated: bool,
    pub wording: Wording,
}

/// How one tool call ended, decided from its result's saved facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed,
    /// The person declined the call at its permission prompt.
    Declined,
    /// The person interrupted the agent while the call ran: not a failure.
    Interrupted,
    /// The host's own permission check refused the call: neither a failure
    /// nor the person's decision.
    PermissionDenied,
    /// The facts do not say.
    Unknown,
}

/// The text of a result's `content`, which the host writes as a string, as a
/// list of parts, or not at all.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

/// A block's call identifier when it is one the tables can hold.
fn call_id<'a>(block: &'a Value, field: &str) -> Option<&'a str> {
    block
        .get(field)
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty() && id.chars().count() <= MAX_CALL_ID_CHARS)
}

/// The facts one `tool_result` block states (version [`FACT_VERSION`]).
pub fn facts(block: &Value) -> ResultFacts {
    let text = result_text(block);
    let wording = if text.starts_with(DECLINED_PREFIX) {
        Wording::Declined
    } else if text.starts_with(INTERRUPTED_PREFIX) {
        Wording::Interrupted
    } else if text.starts_with(PERMISSION_DENIED_PREFIX)
        || (text.starts_with(PERMISSION_TO_USE_PREFIX) && text.contains(PERMISSION_TO_USE_DENIED))
    {
        Wording::PermissionDenied
    } else {
        Wording::Other
    };
    let flag = block.get("is_error");
    ResultFacts {
        call_id: call_id(block, "tool_use_id").map(str::to_owned),
        is_error: flag.and_then(Value::as_bool),
        flag_stated: flag.is_some(),
        wording,
    }
}

/// How a call ended, from its result's saved facts and the session's host.
///
/// `is_error: true` is a failure, unless the result is one of the fixed
/// messages: the person's decline, the person's interruption, or the host's
/// own permission denial. `is_error: false` is a success, whatever the text
/// says. Claude Code omits `is_error` on many successful results and the
/// Messages API defines an omitted flag as false, so for a Claude session an
/// omitted flag is a success. Codex and Cursor results reach the index
/// through the pinned readers, which never state the flag, so there an
/// omitted flag is unknown. A flag that is neither true nor false is unknown.
pub fn outcome(is_error: Option<bool>, flag_stated: bool, wording: Wording, host: Host) -> Outcome {
    match is_error {
        Some(true) => match wording {
            Wording::Declined => Outcome::Declined,
            Wording::Interrupted => Outcome::Interrupted,
            Wording::PermissionDenied => Outcome::PermissionDenied,
            Wording::Other => Outcome::Failed,
        },
        Some(false) => Outcome::Ok,
        None if !flag_stated && host == Host::Claude => Outcome::Ok,
        None => Outcome::Unknown,
    }
}

/// Keep one result block's facts under `version`: a new row, a replacement
/// of a row written under a lower version, nothing for the same facts, and a
/// conflict mark for other facts under the same version. Only a record this
/// session owns, unconflicted, is written to. Returns whether a row was
/// written or replaced.
pub(crate) fn keep(
    connection: &Connection,
    session_id: &str,
    uuid: &str,
    block_index: i64,
    facts: &ResultFacts,
    version: i64,
) -> Result<bool> {
    let written = connection.execute(
        "INSERT INTO tool_results(record_uuid,session_id,block_index,call_id,is_error,flag_stated,
                                  wording,fact_version)
         SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE EXISTS(SELECT 1 FROM records
           WHERE uuid=?1 AND session_id=?2 AND has_conflict=0)
         ON CONFLICT(record_uuid,block_index) DO UPDATE SET
           call_id=excluded.call_id,is_error=excluded.is_error,flag_stated=excluded.flag_stated,
           wording=excluded.wording,fact_version=excluded.fact_version,conflicted=0
         WHERE tool_results.session_id=excluded.session_id
           AND excluded.fact_version>tool_results.fact_version",
        params![
            uuid,
            session_id,
            block_index,
            facts.call_id,
            facts.is_error,
            facts.flag_stated,
            facts.wording.as_str(),
            version
        ],
    )?;
    connection.execute(
        "UPDATE tool_results SET conflicted=1 WHERE record_uuid=?1 AND block_index=?2
           AND session_id=?3 AND fact_version=?8
           AND (call_id IS NOT ?4 OR is_error IS NOT ?5 OR flag_stated IS NOT ?6
                OR wording IS NOT ?7)",
        params![
            uuid,
            block_index,
            session_id,
            facts.call_id,
            facts.is_error,
            facts.flag_stated,
            facts.wording.as_str(),
            version
        ],
    )?;
    Ok(written > 0)
}

/// Keep the call identifiers and result facts the accepted inputs of one
/// write state. Only a record this session owns, unconflicted, is written to
/// (each statement checks that itself, so no row is read for it); an input
/// whose content was not observed states nothing. Returns how many result rows
/// were written or replaced.
pub(crate) fn apply(
    connection: &Connection,
    session_id: &str,
    records: &[CanonicalRecord],
    outcomes: &[RecordOutcome],
) -> Result<usize> {
    let mut kept = 0;
    for outcome in outcomes {
        let Some(uuid) = outcome.uuid.as_deref() else {
            continue;
        };
        if !outcome.disposition.is_accepted() {
            continue;
        }
        let input = &records[outcome.input_index];
        let Some(blocks) = &input.message.content else {
            continue;
        };
        for (index, block) in blocks.iter().enumerate() {
            let index = index as i64;
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") if input.record_type == RecordType::Assistant => {
                    let Some(id) = call_id(block, "id") else {
                        continue;
                    };
                    connection.execute(
                        "UPDATE tool_uses SET call_id=?3 WHERE uuid=?1 AND block_index=?2
                           AND session_id=?4 AND call_id IS NULL AND EXISTS(SELECT 1 FROM records
                           WHERE uuid=?1 AND session_id=?4 AND has_conflict=0)",
                        params![uuid, index, id, session_id],
                    )?;
                    connection.execute(
                        "UPDATE tool_uses SET call_id_conflict=1 WHERE uuid=?1 AND block_index=?2
                           AND session_id=?4 AND call_id IS NOT ?3",
                        params![uuid, index, id, session_id],
                    )?;
                }
                Some("tool_result") if input.record_type == RecordType::User => {
                    kept += usize::from(keep(
                        connection,
                        session_id,
                        uuid,
                        index,
                        &facts(block),
                        FACT_VERSION,
                    )?);
                }
                _ => {}
            }
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(flag: Value, content: Value) -> Value {
        let mut block = json!({"type":"tool_result","tool_use_id":"t","content":content});
        if !flag.is_null() {
            block["is_error"] = flag;
        }
        block
    }
    fn read(block: &Value, host: Host) -> Outcome {
        let facts = facts(block);
        outcome(facts.is_error, facts.flag_stated, facts.wording, host)
    }

    #[test]
    fn the_facts_hold_the_flag_and_the_wording_kind_only() {
        let declined = json!(
            "The user doesn't want to proceed with this tool use. The tool use was rejected."
        );
        assert_eq!(
            facts(&result(
                json!(true),
                json!([{"type":"text","text":declined}])
            )),
            ResultFacts {
                call_id: Some("t".into()),
                is_error: Some(true),
                flag_stated: true,
                wording: Wording::Declined,
            }
        );
        assert_eq!(
            facts(&result(Value::Null, json!("done"))),
            ResultFacts {
                call_id: Some("t".into()),
                is_error: None,
                flag_stated: false,
                wording: Wording::Other,
            }
        );
        let odd = facts(&result(json!("yes"), json!("done")));
        assert_eq!((odd.is_error, odd.flag_stated), (None, true));
        for wording in [
            Wording::Declined,
            Wording::Interrupted,
            Wording::PermissionDenied,
            Wording::Other,
        ] {
            assert_eq!(Wording::parse(wording.as_str()), Some(wording));
        }
    }

    #[test]
    fn a_higher_fact_version_replaces_a_row_and_a_lower_one_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.sqlite");
        let mut store = crate::Store::open(&path).unwrap();
        store
            .upsert_session(
                &crate::SessionMeta::new("s", "claude", crate::SessionSource::Transcript),
                false,
            )
            .unwrap();
        let block = result(json!(true), json!("exit 1"));
        let records: Vec<CanonicalRecord> = vec![
            serde_json::from_value(json!({"uuid":"r1","type":"user",
                "timestamp":"2026-09-07T12:00:01Z",
                "message":{"role":"user","content":[block.clone()]}}))
            .unwrap(),
        ];
        store.upsert_records("s", &records, false).unwrap();
        let connection = Connection::open(&path).unwrap();
        let row = || {
            connection
                .query_row(
                    "SELECT is_error,wording,fact_version,conflicted FROM tool_results",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, Option<bool>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, bool>(3)?,
                        ))
                    },
                )
                .unwrap()
        };
        assert_eq!(row(), (Some(true), "other".into(), FACT_VERSION, false));
        // The same version stating other facts is a contradiction.
        let declined = ResultFacts {
            wording: Wording::Declined,
            ..facts(&block)
        };
        assert!(!keep(&connection, "s", "r1", 0, &declined, FACT_VERSION).unwrap());
        assert_eq!(row(), (Some(true), "other".into(), FACT_VERSION, true));
        // A later derivation replaces the row and clears the mark.
        assert!(keep(&connection, "s", "r1", 0, &declined, FACT_VERSION + 1).unwrap());
        assert_eq!(
            row(),
            (Some(true), "declined".into(), FACT_VERSION + 1, false)
        );
        // Reading the source again under this build's lower version changes
        // nothing and is no contradiction.
        store.upsert_records("s", &records, false).unwrap();
        assert_eq!(
            row(),
            (Some(true), "declined".into(), FACT_VERSION + 1, false)
        );
    }

    #[test]
    fn the_outcome_is_read_from_the_flag_and_the_wording() {
        let declined = json!(
            "The user doesn't want to proceed with this tool use. The tool use was rejected."
        );
        for (block, claude, other) in [
            (
                result(json!(true), json!("exit 1")),
                Outcome::Failed,
                Outcome::Failed,
            ),
            (
                result(json!(true), declined.clone()),
                Outcome::Declined,
                Outcome::Declined,
            ),
            (
                result(json!(true), json!([{"type":"text","text":declined}])),
                Outcome::Declined,
                Outcome::Declined,
            ),
            (
                result(
                    json!(true),
                    json!("[Request interrupted by user for tool use]"),
                ),
                Outcome::Interrupted,
                Outcome::Interrupted,
            ),
            (
                result(
                    json!(true),
                    json!("Permission for this action was denied. Reason: risky."),
                ),
                Outcome::PermissionDenied,
                Outcome::PermissionDenied,
            ),
            (
                result(
                    json!(true),
                    json!(
                        "Permission to use Bash has been denied because Claude Code is running in don't ask mode."
                    ),
                ),
                Outcome::PermissionDenied,
                Outcome::PermissionDenied,
            ),
            (
                result(
                    json!(true),
                    json!("Permission to use Bash with command ls has been denied."),
                ),
                Outcome::PermissionDenied,
                Outcome::PermissionDenied,
            ),
            // Without the denial it is an ordinary failure.
            (
                result(json!(true), json!("Permission to use the cache: yes")),
                Outcome::Failed,
                Outcome::Failed,
            ),
            // The wording without the flag is an ordinary result.
            (result(json!(false), declined), Outcome::Ok, Outcome::Ok),
            (
                result(Value::Null, json!("done")),
                Outcome::Ok,
                Outcome::Unknown,
            ),
            (
                result(json!("yes"), json!("done")),
                Outcome::Unknown,
                Outcome::Unknown,
            ),
        ] {
            assert_eq!(read(&block, Host::Claude), claude, "{block}");
            assert_eq!(read(&block, Host::Codex), other, "{block}");
        }
    }
}
