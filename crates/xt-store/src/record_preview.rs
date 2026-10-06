//! Short previews of two kinds of saved user input: a person's message and a
//! Claude Code task notification's `<summary>`. They are the only words the
//! index keeps whatever the content retention setting says, so the
//! Dashboard's activity bubble can show them without opening the session's
//! original file on every hover. Each is one line of at most
//! [`MAX_PREVIEW_CHARS`] characters.
//!
//! A preview is written inside the ingest batch's own transaction, after the
//! batch's records and its task-notification and injected-context proofs, so
//! a rejected batch leaves none behind. It is written only for an accepted
//! input whose every occurrence in the batch was accepted, whose stored record
//! is unconflicted and holds exactly the text this input carries:
//!
//! * a person preview where the stored classification, read through the
//!   shared `v_human_inputs` projection in the same transaction, calls the
//!   record a person's whole message (eligible, and its counted length is the
//!   record's own: no adjustment says only part of it is theirs), and the
//!   caller did not withhold it;
//! * an automatic preview where a stored task-notification proof names the
//!   record and its text has a nonblank `<summary>` ahead of any `<result>`.
//!
//! A Codex or Cursor input a stored tool-sent proof names gets neither: it is
//! not a person's message, and it has no summary of its own to show. A person
//! preview an earlier read kept for it is withdrawn.
//!
//! Rows fill once and are never rewritten by a replay. Readers choose the
//! record by the current classification first and only then read its preview
//! (see [`read`]), so a proof that later makes the record ineligible hides it.

use crate::{
    Result,
    batch::{IngestBatch, RecordOutcome},
    model::RecordType,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

/// The longest preview kept, in characters.
pub const MAX_PREVIEW_CHARS: usize = 280;
const RULE_VERSION: i64 = 1;
/// The longest record identifier `record_previews` holds (its CHECK).
const MAX_PREVIEW_ID_CHARS: usize = 256;

/// Which kind of input a preview previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    /// A message the stored classification calls a person's.
    Person,
    /// A Claude Code task notification's own `<summary>`.
    Automatic,
}

impl PreviewKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Automatic => "automatic",
        }
    }
}

/// One line of at most [`MAX_PREVIEW_CHARS`] characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub text: String,
    /// More followed the kept characters.
    pub truncated: bool,
}

/// Whitespace runs collapsed to single spaces, at most [`MAX_PREVIEW_CHARS`]
/// characters, from a message's joined text however it was obtained.
pub fn excerpt(text: &str) -> Preview {
    let mut line = String::new();
    let mut truncated = false;
    let mut length = 0;
    for word in text.split_whitespace() {
        let separator = usize::from(!line.is_empty());
        let size = word.chars().count();
        if length + separator + size > MAX_PREVIEW_CHARS {
            let room = MAX_PREVIEW_CHARS.saturating_sub(length + separator);
            if room > 0 {
                if separator == 1 {
                    line.push(' ');
                }
                line.extend(word.chars().take(room));
            }
            truncated = true;
            break;
        }
        if separator == 1 {
            line.push(' ');
        }
        line.push_str(word);
        length += separator + size;
    }
    Preview {
        text: line,
        truncated,
    }
}

/// The `<summary>` of a Claude Code task notification's joined text, as one
/// bounded line. Claude Code writes the notification as `<task-notification>`
/// elements (`<task-id>`, `<status>`, `<summary>`, `<result>`, …); only the
/// first `<summary>` ahead of any `<result>` is the notification's own, since
/// a result quotes arbitrary task output. Text that is not a notification, or
/// has no nonblank summary there, has none.
pub fn notification_summary(text: &str) -> Option<Preview> {
    text.trim()
        .strip_prefix("<task-notification>")
        .and_then(|body| {
            let open = body.find("<summary>")?;
            if body.find("<result>").is_some_and(|result| result < open) {
                return None;
            }
            let rest = &body[open + "<summary>".len()..];
            Some(&rest[..rest.find("</summary>")?])
        })
        .filter(|summary| !summary.trim().is_empty())
        .map(excerpt)
}

/// The kept preview of one record, if any: only an unconflicted record's, of
/// the named kind, whose text was not deleted. The caller has already chosen
/// the record by its current classification; this never chooses one.
pub fn read(connection: &Connection, uuid: &str, kind: PreviewKind) -> Result<Option<Preview>> {
    Ok(connection
        .query_row(
            "SELECT p.text,p.truncated FROM record_previews p
             JOIN records r ON r.uuid=p.record_uuid AND r.session_id=p.session_id
             WHERE p.record_uuid=?1 AND p.kind=?2 AND p.text IS NOT NULL AND r.has_conflict=0",
            params![uuid, kind.as_str()],
            |row| {
                Ok(Preview {
                    text: row.get(0)?,
                    truncated: row.get(1)?,
                })
            },
        )
        .optional()?)
}

impl crate::Store {
    /// [`read`] on this store's own connection.
    pub fn record_preview(&self, uuid: &str, kind: PreviewKind) -> Result<Option<Preview>> {
        read(&self.connection, uuid, kind)
    }
}

/// Keep the previews this batch's accepted inputs allow; see the module
/// documentation. Returns how many rows were newly kept.
pub(crate) fn apply(
    connection: &Connection,
    batch: &IngestBatch<'_>,
    outcomes: &[RecordOutcome],
) -> Result<usize> {
    if !batch.withheld_previews.is_empty() && batch.withheld_previews.len() != batch.records.len() {
        return Err(crate::Error::InvalidInput(
            "withheld previews must align with the batch's inputs",
        ));
    }
    let rejected = outcomes
        .iter()
        .filter(|outcome| !outcome.disposition.is_accepted())
        .filter_map(|outcome| outcome.uuid.as_deref())
        .collect::<BTreeSet<_>>();
    let mut kept = 0;
    for outcome in outcomes {
        let Some(uuid) = outcome.uuid.as_deref() else {
            continue;
        };
        if !outcome.disposition.is_accepted() || rejected.contains(uuid) {
            continue;
        }
        // The table bounds its identifiers; a record whose identifier it
        // cannot hold is still saved, only without a preview. A preview never
        // refuses a batch.
        if uuid.trim().is_empty() || uuid.chars().count() > MAX_PREVIEW_ID_CHARS {
            continue;
        }
        let input = &batch.records[outcome.input_index];
        if input.record_type != RecordType::User
            || input.message.role.as_deref() != Some("user")
            || input.is_meta
            || input.is_sidechain
        {
            continue;
        }
        let Some(blocks) = &input.message.content else {
            continue;
        };
        let text = crate::record_text::joined(blocks);
        let Ok(length) = i64::try_from(text.chars().count()) else {
            continue;
        };
        if batch
            .tool_sent
            .get(outcome.input_index)
            .is_some_and(Option::is_some)
            && crate::tool_sent::proven(connection, uuid)?
        {
            // A proven tool-sent input is never a person's message: a person
            // preview an earlier read kept for it goes, and none is kept.
            withdraw_person(connection, uuid)?;
            continue;
        }
        let notification = batch
            .task_notifications
            .get(outcome.input_index)
            .copied()
            .unwrap_or(false);
        kept += if notification {
            // A proven notification is never a person's message: a person
            // preview an earlier read kept for it goes.
            connection.execute(
                "DELETE FROM record_previews WHERE record_uuid=?1 AND kind='person'
                   AND EXISTS (SELECT 1 FROM task_notification_inputs WHERE record_uuid=?1)",
                [uuid],
            )?;
            let Some(summary) = notification_summary(&text) else {
                continue;
            };
            connection.execute(
                "INSERT INTO record_previews(record_uuid,session_id,kind,text,truncated,rule_version)
                 SELECT r.uuid,r.session_id,'automatic',?3,?4,?5 FROM task_notification_inputs n
                 JOIN records r ON r.uuid=n.record_uuid AND r.session_id=n.session_id
                 WHERE n.record_uuid=?1 AND r.has_conflict=0 AND r.text_len=?2
                   AND length(r.uuid)<=256
                 ON CONFLICT(record_uuid) DO NOTHING",
                params![uuid, length, summary.text, summary.truncated, RULE_VERSION],
            )?
        } else {
            if batch
                .withheld_previews
                .get(outcome.input_index)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let preview = excerpt(&text);
            connection.execute(
                "INSERT INTO record_previews(record_uuid,session_id,kind,text,truncated,rule_version)
                 SELECT r.uuid,r.session_id,'person',?3,?4,?5 FROM records r
                 JOIN sessions s ON s.session_id=r.session_id
                 JOIN v_human_inputs h ON h.uuid=r.uuid
                 WHERE r.uuid=?1 AND r.type='user' AND r.role='user' AND r.is_meta=0
                   AND r.is_sidechain=0 AND r.has_conflict=0 AND r.text_len=?2
                   AND s.kind='user' AND (r.model IS NULL OR r.model<>'<synthetic>')
                   AND h.human_is_eligible=1 AND h.human_text_len IS r.text_len
                   AND length(r.uuid)<=256
                 ON CONFLICT(record_uuid) DO NOTHING",
                params![uuid, length, preview.text, preview.truncated, RULE_VERSION],
            )?
        };
    }
    Ok(kept)
}

/// Remove the person preview of each named record: an accepted human-input
/// adjustment says only part, or none, of its text is the person's, so its
/// joined text is not their words.
pub(crate) fn withdraw_person(connection: &Connection, uuid: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM record_previews WHERE record_uuid=?1 AND kind='person'",
        [uuid],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_is_the_notifications_own_bounded_line() {
        let note = |inner: &str| {
            format!("<task-notification><task-id>t1</task-id>{inner}</task-notification>")
        };
        assert_eq!(
            notification_summary(&note(
                "<status>completed</status><summary>Agent \"Map  sources\"\n finished</summary><result>done</result>"
            )),
            Some(Preview {
                text: "Agent \"Map sources\" finished".into(),
                truncated: false
            })
        );
        assert_eq!(
            notification_summary(&note("<result><summary>quoted</summary></result>")),
            None
        );
        for none in [
            note("<status>completed</status>"),
            note("<summary>   </summary>"),
            note("<summary>unterminated"),
            "an ordinary message <summary>quoted</summary>".to_owned(),
        ] {
            assert_eq!(notification_summary(&none), None);
        }
    }

    #[test]
    fn excerpt_collapses_whitespace_and_bounds_characters() {
        assert_eq!(
            excerpt("  make the PR\n\tlink exact  "),
            Preview {
                text: "make the PR link exact".into(),
                truncated: false
            }
        );
        let long = "é".repeat(MAX_PREVIEW_CHARS + 5);
        let preview = excerpt(&format!("go {long}"));
        assert!(preview.truncated);
        assert_eq!(preview.text.chars().count(), MAX_PREVIEW_CHARS);
        let exact = "a".repeat(MAX_PREVIEW_CHARS);
        assert_eq!(
            excerpt(&exact),
            Preview {
                text: exact.clone(),
                truncated: false
            }
        );
    }
}
