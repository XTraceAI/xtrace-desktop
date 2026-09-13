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

/// Incremental splitter: feed the stream one line at a time and take each
/// session block as soon as its successor header (or the end) arrives, so a
/// large history is never held in memory as a whole.
pub struct BlockReader {
    host: Host,
    source: SessionSource,
    current: Option<(SessionBlock, SourceContext)>,
    /// A malformed block swallows its records until the next header.
    skipping: bool,
    /// A malformed successor header that arrived together with a completed block.
    queued: Option<BlockOutcome>,
    line: usize,
}

impl BlockReader {
    /// `host` is the host the caller asked the producer for; a header claiming
    /// another host is malformed.
    pub fn new(host: Host, source: SessionSource) -> Self {
        Self {
            host,
            source,
            current: None,
            skipping: false,
            queued: None,
            line: 0,
        }
    }

    /// Consume one line. A completed block is returned when the next header
    /// begins; a record before any header rejects the stream.
    pub fn push(&mut self, line: &str) -> Result<Option<BlockOutcome>, StreamError> {
        self.line += 1;
        let number = self.line;
        if line.trim().is_empty() {
            return Ok(None);
        }
        if is_header(line) {
            let finished = self
                .current
                .take()
                .map(|(block, _)| BlockOutcome::Ready(Box::new(block)));
            self.skipping = false;
            let pending = match self.start_block(line, number) {
                Ok(()) => None,
                Err(malformed) => {
                    self.skipping = true;
                    Some(malformed)
                }
            };
            // A finished block precedes a malformed successor; the caller sees
            // the successor on the next push (it is queued behind the header).
            return Ok(match (finished, pending) {
                (Some(block), Some(malformed)) => {
                    self.queued = Some(malformed);
                    Some(block)
                }
                (Some(block), None) => Some(block),
                (None, Some(malformed)) => Some(malformed),
                (None, None) => None,
            });
        }
        if self.skipping {
            return Ok(None);
        }
        let Some((block, context)) = self.current.as_mut() else {
            return Err(StreamError::RecordBeforeHeader { line: number });
        };
        match parse_with_context(line, context) {
            Ok(Parsed::Record(record)) => block.records.push(*record),
            Ok(Parsed::Dropped(_)) => block.dropped += 1,
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => block.inert += 1,
            Err(_) => {
                let (block, _) = self.current.take().unwrap();
                self.skipping = true;
                return Ok(Some(BlockOutcome::Malformed {
                    native_session_id: Some(block.header.native_session_id),
                    line: number,
                    reason: "canonical record does not match the shared stream contract",
                }));
            }
        }
        Ok(None)
    }

    fn start_block(&mut self, line: &str, number: usize) -> Result<(), BlockOutcome> {
        let header =
            serde_json::from_str::<SessionHeader>(line).map_err(|_| BlockOutcome::Malformed {
                native_session_id: None,
                line: number,
                reason: "session header does not match the shared stream contract",
            })?;
        let native = header.native_session_id.clone();
        let reason = if header.host != self.host.as_str() {
            Some("session header names another host")
        } else if native.trim().is_empty() || header.conversation_id.trim().is_empty() {
            Some("session header lacks an identity")
        } else if header.conversation_id != expected_conversation_id(self.host, &native) {
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
            return Err(BlockOutcome::Malformed {
                native_session_id: Some(native),
                line: number,
                reason,
            });
        }
        let context = header.context(self.host, self.source);
        self.current = Some((
            SessionBlock {
                header,
                records: Vec::new(),
                inert: 0,
                dropped: 0,
            },
            context,
        ));
        Ok(())
    }

    /// A malformed successor header that was queued behind a completed block.
    pub fn take_queued(&mut self) -> Option<BlockOutcome> {
        self.queued.take()
    }

    /// The final block once the stream has ended.
    pub fn finish(mut self) -> Option<BlockOutcome> {
        self.queued.take().or_else(|| {
            self.current
                .take()
                .map(|(block, _)| BlockOutcome::Ready(Box::new(block)))
        })
    }
}

/// Split one complete stream into per-session blocks.
pub fn parse_stream<I>(
    lines: I,
    host: Host,
    source: SessionSource,
) -> Result<Vec<BlockOutcome>, StreamError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut reader = BlockReader::new(host, source);
    let mut blocks = Vec::new();
    for line in lines {
        if let Some(block) = reader.push(line.as_ref())? {
            blocks.push(block);
        }
        if let Some(queued) = reader.take_queued() {
            blocks.push(queued);
        }
    }
    blocks.extend(reader.finish());
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
