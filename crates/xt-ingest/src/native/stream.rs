//! The shared read-only reader stream: one session header line followed by that
//! session's canonical records, repeated per session. Codex and Cursor sessions
//! arrive this way from the pinned `readers_cli.py`; Claude files are lifted into
//! the same shape by `claude_fs`. Parsing is strict: a malformed header or record
//! marks its session explicitly instead of importing a partial guess, while the
//! other sessions in the stream keep importing.

use crate::canonical::{Parsed, ParsedRecord, SourceContext, parse_with_context};
use serde::Deserialize;
use xt_store::{Host, SessionSource};

/// The exact header the pinned producer emits. Unknown keys are a contract
/// change, so they fail parsing rather than being ignored.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub kind: String,
    pub host: String,
    pub native_session_id: String,
    pub conversation_id: String,
    pub source_surface: Option<String>,
    pub started_at: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    /// Emitted by the producer, deliberately never persisted: it derives from prompts.
    pub title: Option<String>,
    pub path: String,
    pub mtime: f64,
}

impl SessionHeader {
    pub fn context(&self, host: Host, source: SessionSource) -> SourceContext {
        SourceContext {
            conversation_id: Some(self.conversation_id.clone()),
            native_session_id: Some(self.native_session_id.clone()),
            source_platform: Some(host.as_str().to_owned()),
            source_surface: self.source_surface.clone(),
            started_at: self.started_at.clone(),
            source: Some(source),
        }
    }
}

#[derive(Debug)]
pub struct SessionBlock {
    pub header: SessionHeader,
    pub records: Vec<ParsedRecord>,
    /// Structural lines the canonical parser recognised but does not store.
    pub inert: usize,
    pub dropped: usize,
}

#[derive(Debug)]
pub enum BlockOutcome {
    Ready(Box<SessionBlock>),
    /// The session is skipped as a whole; `line` is 1-based within the stream.
    Malformed {
        native_session_id: Option<String>,
        line: usize,
        reason: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// A record appeared before any session header: the stream itself is not
    /// the shared contract, so nothing in it can be attributed.
    RecordBeforeHeader { line: usize },
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecordBeforeHeader { line } => {
                write!(f, "reader stream line {line} precedes any session header")
            }
        }
    }
}
impl std::error::Error for StreamError {}

fn is_header(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(|kind| kind == "session")
        })
        .unwrap_or(false)
}

/// Split one stream into per-session blocks. `host` is the host the caller asked
/// the producer for; a header claiming another host is malformed.
pub fn parse_stream<I>(
    lines: I,
    host: Host,
    source: SessionSource,
) -> Result<Vec<BlockOutcome>, StreamError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut blocks = Vec::new();
    let mut current: Option<(SessionBlock, SourceContext)> = None;
    // A malformed block swallows its records until the next header.
    let mut skipping = false;
    for (index, line) in lines.into_iter().enumerate() {
        let line = line.as_ref();
        let number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        if is_header(line) {
            if let Some((block, _)) = current.take() {
                blocks.push(BlockOutcome::Ready(Box::new(block)));
            }
            skipping = false;
            match serde_json::from_str::<SessionHeader>(line) {
                Ok(header) => {
                    let native = header.native_session_id.clone();
                    let reason = if header.host != host.as_str() {
                        Some("session header names another host")
                    } else if native.trim().is_empty() || header.conversation_id.trim().is_empty() {
                        Some("session header lacks an identity")
                    } else if header.conversation_id != expected_conversation_id(host, &native) {
                        Some("session header conversation ID does not derive from its native ID")
                    } else if header
                        .started_at
                        .as_deref()
                        .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
                    {
                        Some("session header start is not RFC3339")
                    } else if !header.mtime.is_finite() || header.mtime < 0.0 {
                        Some("session header clock is not a representable instant")
                    } else {
                        None
                    };
                    if let Some(reason) = reason {
                        blocks.push(BlockOutcome::Malformed {
                            native_session_id: Some(native),
                            line: number,
                            reason,
                        });
                        skipping = true;
                        continue;
                    }
                    let context = header.context(host, source);
                    current = Some((
                        SessionBlock {
                            header,
                            records: Vec::new(),
                            inert: 0,
                            dropped: 0,
                        },
                        context,
                    ));
                }
                Err(_) => {
                    blocks.push(BlockOutcome::Malformed {
                        native_session_id: None,
                        line: number,
                        reason: "session header does not match the shared stream contract",
                    });
                    skipping = true;
                }
            }
            continue;
        }
        if skipping {
            continue;
        }
        let Some((block, context)) = current.as_mut() else {
            return Err(StreamError::RecordBeforeHeader { line: number });
        };
        match parse_with_context(line, context) {
            Ok(Parsed::Record(record)) => block.records.push(*record),
            Ok(Parsed::Dropped(_)) => block.dropped += 1,
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => block.inert += 1,
            Err(_) => {
                let (block, _) = current.take().unwrap();
                blocks.push(BlockOutcome::Malformed {
                    native_session_id: Some(block.header.native_session_id),
                    line: number,
                    reason: "canonical record does not match the shared stream contract",
                });
                skipping = true;
            }
        }
    }
    if let Some((block, _)) = current.take() {
        blocks.push(BlockOutcome::Ready(Box::new(block)));
    }
    Ok(blocks)
}

pub fn expected_conversation_id(host: Host, native: &str) -> String {
    match host {
        Host::Claude => native.to_owned(),
        Host::Codex => format!("codex-{native}"),
        Host::Cursor => format!("cursor-{native}"),
        Host::Other => format!("other-{native}"),
    }
}
