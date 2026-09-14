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

/// One parsed line of the stream. A consumer that writes as it reads never
/// holds more than the records between two of its own commits.
#[derive(Debug)]
pub enum StreamEvent {
    /// A new session begins; any previous session is complete.
    Session(SessionHeader),
    Record(Box<ParsedRecord>),
    /// A canonical line without a usable identity; it counts against coverage.
    Dropped,
    /// The session named (or the unnamed header at `line`) is skipped as a
    /// whole from this line on; a session that had already started ends here.
    Malformed {
        native_session_id: Option<String>,
        line: usize,
        reason: &'static str,
    },
}

/// Incremental splitter: feed the stream one line at a time. `host` is the host
/// the caller asked the producer for; a header claiming another host is malformed.
pub struct StreamEvents {
    host: Host,
    source: SessionSource,
    context: Option<(String, SourceContext)>,
    /// A malformed session swallows its records until the next header.
    skipping: bool,
    line: usize,
}

impl StreamEvents {
    pub fn new(host: Host, source: SessionSource) -> Self {
        Self {
            host,
            source,
            context: None,
            skipping: false,
            line: 0,
        }
    }

    /// Consume one line. A record before any header rejects the stream.
    pub fn push(&mut self, line: &str) -> Result<Option<StreamEvent>, StreamError> {
        self.line += 1;
        let number = self.line;
        if line.trim().is_empty() {
            return Ok(None);
        }
        if is_header(line) {
            self.context = None;
            self.skipping = false;
            return Ok(Some(match self.header(line, number) {
                Ok(header) => {
                    self.context = Some((
                        header.native_session_id.clone(),
                        header.context(self.host, self.source),
                    ));
                    StreamEvent::Session(header)
                }
                Err(malformed) => {
                    self.skipping = true;
                    *malformed
                }
            }));
        }
        if self.skipping {
            return Ok(None);
        }
        let Some((native, context)) = self.context.as_ref() else {
            return Err(StreamError::RecordBeforeHeader { line: number });
        };
        Ok(match parse_with_context(line, context) {
            Ok(Parsed::Record(record)) => Some(StreamEvent::Record(record)),
            Ok(Parsed::Dropped(_)) => Some(StreamEvent::Dropped),
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => None,
            Err(_) => {
                let native = native.clone();
                self.context = None;
                self.skipping = true;
                Some(StreamEvent::Malformed {
                    native_session_id: Some(native),
                    line: number,
                    reason: "canonical record does not match the shared stream contract",
                })
            }
        })
    }

    fn header(&self, line: &str, number: usize) -> Result<SessionHeader, Box<StreamEvent>> {
        let header =
            serde_json::from_str::<SessionHeader>(line).map_err(|_| StreamEvent::Malformed {
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
        match reason {
            Some(reason) => Err(Box::new(StreamEvent::Malformed {
                native_session_id: Some(native),
                line: number,
                reason,
            })),
            None => Ok(header),
        }
    }
}

/// Split one complete stream into per-session blocks (whole sessions in
/// memory; the importer consumes events instead).
pub fn parse_stream<I>(
    lines: I,
    host: Host,
    source: SessionSource,
) -> Result<Vec<BlockOutcome>, StreamError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut events = StreamEvents::new(host, source);
    let mut blocks = Vec::new();
    let mut open: Option<SessionBlock> = None;
    for line in lines {
        match events.push(line.as_ref())? {
            None => {}
            Some(StreamEvent::Session(header)) => {
                blocks.extend(
                    open.take()
                        .map(|block| BlockOutcome::Ready(Box::new(block))),
                );
                open = Some(SessionBlock {
                    header,
                    records: Vec::new(),
                    inert: 0,
                    dropped: 0,
                });
            }
            Some(StreamEvent::Record(record)) => {
                if let Some(block) = open.as_mut() {
                    block.records.push(*record);
                }
            }
            Some(StreamEvent::Dropped) => {
                if let Some(block) = open.as_mut() {
                    block.dropped += 1;
                }
            }
            Some(StreamEvent::Malformed {
                native_session_id,
                line,
                reason,
            }) => {
                open = None;
                blocks.push(BlockOutcome::Malformed {
                    native_session_id,
                    line,
                    reason,
                });
            }
        }
    }
    blocks.extend(
        open.take()
            .map(|block| BlockOutcome::Ready(Box::new(block))),
    );
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
