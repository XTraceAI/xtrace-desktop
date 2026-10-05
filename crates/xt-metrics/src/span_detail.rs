//! What happened inside one active span of one session, for the detail a lane
//! shows when its bar is pointed at.
//!
//! A span is named by its session and its endpoints on the stored millisecond
//! projection, exactly as [`crate::ActiveSpan`] reports them; every event M-05
//! folded into that span has a `ts_ms` inside `[start_ms, end_ms]`, and no
//! other event of the same session does. Nothing is redefined here:
//!
//! * **Output tokens** are [`MetricsDb::session_windows`]' M-04 slice over the
//!   span's own interval, so a response repeated across records is selected
//!   once, by the same global projection the Dashboard sums.
//! * **The most-used tool** counts record-bound `tool_uses` rows joined through
//!   `v_records`, the same rows and work exclusions M-17 counts, restricted to
//!   the span. Slash commands and hook summaries are not tool calls.
//! * **The last prompt** is the latest record the stored M-02 classification
//!   calls a person's message (`human_is_eligible`), so sidechain, injected,
//!   confirmed automated and sub-session inputs are never chosen. Its words are
//!   read back where content retention kept them, else from the short preview
//!   the index keeps of a person's message (see [`xt_store::record_preview`]).
//! * **The automatic line** is the latest input inside the span that a stored
//!   source proof says Claude Code wrote as a task notification (a background
//!   agent, command or monitor it started finished). Such an input is not a
//!   person's message, so it is never the last prompt; what it announced is
//!   its own `<summary>`, read the same way as the prompt's words.
//!
//! An unknown stays unknown: no stored usage is not zero output, an
//! unclassified input is not "no prompt", and unkept words are not an empty
//! message.

use crate::{Error, MetricsDb, Result, SessionWindow, Window};
use serde::Serialize;
use xt_store::{
    record_preview::{self, Preview, PreviewKind},
    record_text::{self, RecordText},
    timestamp::{self, InstantKey},
};

/// Whether the session is a measurable user session at all.
pub(crate) const EXISTS_QUERY: &str = "SELECT 1 FROM sessions WHERE kind='user' AND session_id=?1";
/// The span's record-bound tool calls by stored name, most frequent first and
/// then by name, so a tie always resolves to the same tool; only the first
/// row is read.
pub(crate) const TOOL_QUERY: &str = "SELECT t.name,count(*) FROM tool_uses t
     JOIN v_records v ON v.uuid=t.uuid
     WHERE t.uuid IS NOT NULL AND v.session_id=?1 AND v.ts_ms>=?2 AND v.ts_ms<=?3
     GROUP BY t.name ORDER BY count(*) DESC,t.name ASC";
/// The tool calls the span's records state, as the canonical
/// `tool_use_count`; `NULL` from `sum` with any unknown count is read
/// separately, because SQLite's `sum` skips nulls.
pub(crate) const STATED_QUERY: &str =
    "SELECT count(*),count(tool_use_count),coalesce(sum(tool_use_count),0)
     FROM v_session_events WHERE session_id=?1 AND type='assistant' AND ts_ms>=?2 AND ts_ms<=?3";
/// User records at or before the span's end that are, or may be, a person's
/// message, newest first. An unclassified record is a candidate too: when it
/// is the newest, which message came last is unknown.
///
/// `human_text_len` is the stored M-02 length of the person's part. When an
/// accepted human-input adjustment says only part of the record is theirs —
/// a Codex question reply or image wrapper — it differs from `text_len`, and
/// the record's joined text is not the person's words.
pub(crate) const PROMPT_QUERY: &str = "SELECT uuid,ts,ts_ms,human_is_eligible,
       human_text_len IS NOT text_len FROM v_session_events
     WHERE session_id=?1 AND type='user' AND ts_ms<=?2
       AND (human_is_eligible IS NULL OR human_is_eligible<>0)
     ORDER BY ts_ms DESC";

/// The span's task notifications, newest first: inputs of this session inside
/// the span that a stored proof says Claude Code wrote itself, never a
/// sidechain's.
pub(crate) const AUTOMATIC_QUERY: &str = "SELECT v.uuid,v.ts,v.ts_ms FROM v_session_events v
     JOIN task_notification_inputs n ON n.record_uuid=v.uuid AND n.session_id=v.session_id
     WHERE v.session_id=?1 AND v.type='user' AND v.is_sidechain=0
       AND v.ts_ms>=?2 AND v.ts_ms<=?3
     ORDER BY v.ts_ms DESC";

/// The longest prompt excerpt sent, in characters. The bubble shows one line;
/// this only bounds what crosses to it.
pub const MAX_PROMPT_CHARS: usize = record_preview::MAX_PREVIEW_CHARS;

/// One span's detail, or nothing measurable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SpanDetail {
    /// No indexed user session owns this identifier.
    Missing,
    Indexed {
        tool: SpanTool,
        /// M-04 output tokens of the responses selected inside the span;
        /// `None` when no selected response stated the counter.
        output_tokens: Option<u64>,
        prompt: SpanPrompt,
        automatic: SpanAutomatic,
    },
}

/// The tool the span called most often.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SpanTool {
    /// The stored name exactly as ingest kept it, and its calls in the span.
    Called { name: String, calls: u64 },
    /// Every record in the span states its tool calls, and they sum to zero.
    NoCalls,
    /// No call is stored, but a record states calls or does not say.
    Unknown,
}

/// The last message a person typed in the span, else before it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SpanPrompt {
    /// The session has no person's message at or before the span's end.
    NoMessage,
    /// The newest candidate's classification is unknown, so which message a
    /// person typed last cannot be said.
    Unclassified,
    Found {
        /// The message's own instant on the millisecond projection.
        at_ms: i64,
        /// Inside the span, rather than the latest one before it started.
        in_span: bool,
        /// The record's saved identity, exactly as stored: what a caller
        /// matches in the session's original source when the words were not
        /// kept. Never sent on by this report.
        #[serde(skip)]
        record_uuid: String,
        text: PromptText,
    },
}

/// The latest task notification Claude Code wrote inside the span: an
/// automatic input, never a person's message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SpanAutomatic {
    /// No input inside the span is a proven task notification.
    NoNotification,
    Found {
        /// The notification's own instant on the millisecond projection.
        at_ms: i64,
        /// The record's saved identity, exactly as stored, for reading its
        /// words from the session's original source. Never sent on.
        #[serde(skip)]
        record_uuid: String,
        text: AutomaticText,
    },
}

/// What a task notification announced: its `<summary>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AutomaticText {
    /// Neither content retention nor a preview kept the record's words. The
    /// caller may read them from the session's original source by
    /// `record_uuid`.
    NotStored,
    /// The summary as one line of at most [`MAX_PROMPT_CHARS`] characters,
    /// whitespace collapsed; `truncated` says whether more followed.
    Stored { text: String, truncated: bool },
    /// The words hold no nonblank `<summary>` of a task notification.
    NoSummary,
}

/// A chosen message's words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PromptText {
    /// Neither content retention nor a preview kept this record's words. The
    /// caller may read them from the session's original source by
    /// `record_uuid`.
    NotStored,
    /// Whitespace runs collapsed to single spaces, at most
    /// [`MAX_PROMPT_CHARS`] characters; `truncated` says whether more followed.
    Stored { text: String, truncated: bool },
    /// Only part of the record is the person's (a Codex question reply or
    /// image wrapper), and nothing locates that part, so no words are given.
    Wrapped,
}

impl MetricsDb {
    /// The detail of one span of one session: its most-used tool, its output
    /// tokens and the last message a person typed in it or before it.
    ///
    /// `start_ms`/`end_ms` are an [`crate::ActiveSpan`]'s endpoints, both
    /// inclusive; a single-event span has them equal. Read inside one
    /// snapshot, so every part describes the same committed state.
    pub fn span_detail(&self, session_id: &str, start_ms: i64, end_ms: i64) -> Result<SpanDetail> {
        if start_ms > end_ms {
            return Err(Error::InvalidWindow);
        }
        // The token slice is half-open, so it ends one millisecond after the
        // span's last event; that also validates both endpoints.
        let window = Window::new(start_ms, end_ms.checked_add(1).ok_or(Error::InvalidWindow)?)?;
        self.read_snapshot(|metrics| {
            let mut exists = metrics.connection.prepare(EXISTS_QUERY)?;
            if !exists.exists([session_id])? {
                return Ok(SpanDetail::Missing);
            }
            let output_tokens = match metrics
                .session_windows(window, &[session_id])?
                .remove(session_id)
            {
                Some(SessionWindow::Indexed { tokens, .. }) => tokens.counters.output_tokens,
                Some(SessionWindow::Missing) | None => None,
            };
            Ok(SpanDetail::Indexed {
                tool: metrics.span_tool(session_id, start_ms, end_ms)?,
                output_tokens,
                prompt: metrics.span_prompt(session_id, start_ms, end_ms)?,
                automatic: metrics.span_automatic(session_id, start_ms, end_ms)?,
            })
        })
    }

    fn span_tool(&self, session_id: &str, start_ms: i64, end_ms: i64) -> Result<SpanTool> {
        let mut statement = self.connection.prepare(TOOL_QUERY)?;
        let mut rows = statement.query(rusqlite::params![session_id, start_ms, end_ms])?;
        if let Some(row) = rows.next()? {
            return Ok(SpanTool::Called {
                name: row.get(0)?,
                calls: crate::counts::counter(row, 1)?.ok_or(Error::CounterOverflow)?,
            });
        }
        let (records, known, stated): (i64, i64, i64) = self.connection.query_row(
            STATED_QUERY,
            rusqlite::params![session_id, start_ms, end_ms],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(if records == known && stated == 0 {
            SpanTool::NoCalls
        } else {
            SpanTool::Unknown
        })
    }

    fn span_prompt(&self, session_id: &str, start_ms: i64, end_ms: i64) -> Result<SpanPrompt> {
        // Every candidate sharing the newest millisecond, ordered by the
        // precise instant and then UUID, as every other timeline is ordered.
        let mut newest: Vec<(InstantKey, String, i64, Option<bool>, bool)> = Vec::new();
        let mut statement = self.connection.prepare(PROMPT_QUERY)?;
        let mut rows = statement.query(rusqlite::params![session_id, end_ms])?;
        while let Some(row) = rows.next()? {
            let ts_ms: i64 = row.get(2)?;
            if newest.first().is_some_and(|first| first.2 != ts_ms) {
                break;
            }
            let raw: String = row.get(1)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            newest.push((instant, row.get(0)?, ts_ms, row.get(3)?, row.get(4)?));
        }
        drop(rows);
        drop(statement);
        let Some((_, uuid, at_ms, eligible, partial)) = newest
            .into_iter()
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        else {
            return Ok(SpanPrompt::NoMessage);
        };
        if eligible.is_none() {
            return Ok(SpanPrompt::Unclassified);
        }
        // No reader separates the person's part of a wrapped input: its
        // adjustment records only a length. The joined text would show the
        // wrapper, so the words are withheld rather than shown wrong.
        let text = if partial {
            PromptText::Wrapped
        } else {
            match record_text::text(&self.connection, &uuid)? {
                RecordText::Stored(text) => prompt_excerpt(&text),
                RecordText::NotStored | RecordText::Missing => {
                    match record_preview::read(&self.connection, &uuid, PreviewKind::Person)? {
                        Some(Preview { text, truncated }) => PromptText::Stored { text, truncated },
                        None => PromptText::NotStored,
                    }
                }
            }
        };
        Ok(SpanPrompt::Found {
            at_ms,
            in_span: at_ms >= start_ms,
            record_uuid: uuid,
            text,
        })
    }
}

impl MetricsDb {
    fn span_automatic(
        &self,
        session_id: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<SpanAutomatic> {
        // Every notification sharing the newest millisecond, ordered by the
        // precise instant and then UUID, as the last prompt is chosen.
        let mut newest: Vec<(InstantKey, String, i64)> = Vec::new();
        let mut statement = self.connection.prepare(AUTOMATIC_QUERY)?;
        let mut rows = statement.query(rusqlite::params![session_id, start_ms, end_ms])?;
        while let Some(row) = rows.next()? {
            let ts_ms: i64 = row.get(2)?;
            if newest.first().is_some_and(|first| first.2 != ts_ms) {
                break;
            }
            let raw: String = row.get(1)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            newest.push((instant, row.get(0)?, ts_ms));
        }
        drop(rows);
        drop(statement);
        let Some((_, uuid, at_ms)) = newest
            .into_iter()
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        else {
            return Ok(SpanAutomatic::NoNotification);
        };
        let text = match record_text::text(&self.connection, &uuid)? {
            RecordText::Stored(text) => notification_summary(&text),
            RecordText::NotStored | RecordText::Missing => {
                match record_preview::read(&self.connection, &uuid, PreviewKind::Automatic)? {
                    Some(Preview { text, truncated }) => AutomaticText::Stored { text, truncated },
                    None => AutomaticText::NotStored,
                }
            }
        };
        Ok(SpanAutomatic::Found {
            at_ms,
            record_uuid: uuid,
            text,
        })
    }
}

/// The `<summary>` of a Claude Code task notification's joined text, as one
/// bounded line, however the text was obtained; the store's rule, which also
/// decides what a kept preview holds.
pub fn notification_summary(text: &str) -> AutomaticText {
    match record_preview::notification_summary(text) {
        Some(Preview { text, truncated }) => AutomaticText::Stored { text, truncated },
        None => AutomaticText::NoSummary,
    }
}

/// One line of at most [`MAX_PROMPT_CHARS`] characters, from a message's
/// joined text however it was obtained: kept by the index, previewed, or read
/// back from the session's original source.
pub fn prompt_excerpt(text: &str) -> PromptText {
    let Preview { text, truncated } = record_preview::excerpt(text);
    PromptText::Stored { text, truncated }
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
            AutomaticText::Stored {
                text: "Agent \"Map sources\" finished".into(),
                truncated: false
            }
        );
        // A result quoting a summary tag is task output, not the notification's.
        assert_eq!(
            notification_summary(&note("<result><summary>quoted</summary></result>")),
            AutomaticText::NoSummary
        );
        for none in [
            note("<status>completed</status>"),
            note("<summary>   </summary>"),
            note("<summary>unterminated"),
            "an ordinary message <summary>quoted</summary>".to_owned(),
        ] {
            assert_eq!(notification_summary(&none), AutomaticText::NoSummary);
        }
        let long = "x".repeat(MAX_PROMPT_CHARS + 10);
        let AutomaticText::Stored { text, truncated } =
            notification_summary(&note(&format!("<summary>{long}</summary>")))
        else {
            panic!("a long summary is still a summary");
        };
        assert!(truncated);
        assert_eq!(text.chars().count(), MAX_PROMPT_CHARS);
    }

    #[test]
    fn excerpt_collapses_whitespace_and_bounds_characters() {
        assert_eq!(
            prompt_excerpt("  make the PR\n\tlink exact  "),
            PromptText::Stored {
                text: "make the PR link exact".into(),
                truncated: false
            }
        );
        let long = "é".repeat(MAX_PROMPT_CHARS + 5);
        let PromptText::Stored { text, truncated } = prompt_excerpt(&format!("go {long}")) else {
            panic!("stored text stays stored");
        };
        assert!(truncated);
        assert_eq!(text.chars().count(), MAX_PROMPT_CHARS);
        assert!(text.starts_with("go é"));
        let exact = "a".repeat(MAX_PROMPT_CHARS);
        assert_eq!(
            prompt_excerpt(&exact),
            PromptText::Stored {
                text: exact.clone(),
                truncated: false
            }
        );
    }
}
