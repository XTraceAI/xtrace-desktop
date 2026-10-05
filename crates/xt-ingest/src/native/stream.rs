//! The shared read-only reader stream: one session header line followed by that
//! session's canonical records, repeated per session. Codex and Cursor sessions
//! arrive this way from the pinned `readers_cli.py`; Claude files are lifted into
//! the same shape by `claude_fs`. Parsing is strict: a malformed header or record
//! marks its session explicitly instead of importing a partial guess, while the
//! other sessions in the stream keep importing. The opt-in Codex origin-evidence
//! mode ([`super::origin`]) judges evidence beside these events, never in them.

use super::origin::{Origin, OriginMarker, OriginSession};
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
    /// Only [`StreamEvents::with_codex_origin_evidence`] admits this key; an
    /// ordinary stream rejects its presence as the unknown key it is there.
    #[serde(default)]
    pub origin_evidence: OriginMarker,
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

/// Only decoded JSON establishes a boundary. A decoded object with the
/// header-only identity/mtime keys is an invalid header if its discriminator
/// is missing. Invalid JSON is an error in the active session, never a guess
/// about a successor's identity.
enum LineKind {
    Header,
    MalformedHeader,
    Other,
}

fn classify(line: &str) -> LineKind {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(serde_json::Value::Object(object)) => {
            if object.get("type").and_then(serde_json::Value::as_str) == Some("session") {
                LineKind::Header
            } else if object.contains_key("mtime") && object.contains_key("native_session_id") {
                LineKind::MalformedHeader
            } else {
                LineKind::Other
            }
        }
        Ok(_) => LineKind::Other,
        // Invalid JSON cannot establish a session boundary. Abandon the active
        // session and resume only at a later decodable header.
        Err(_) => LineKind::Other,
    }
}

/// One parsed line of the stream. A consumer that writes as it reads never
/// holds more than the records between two of its own commits.
#[derive(Debug)]
pub enum StreamEvent {
    /// A new session begins; any previous session is complete.
    Session(Box<SessionHeader>),
    Record(Box<ParsedRecord>),
    /// A canonical line without a usable identity; it counts against coverage.
    Dropped,
    /// A header that does not open a usable session. Like `Session`, it is a
    /// boundary: the previous session is complete; the named (or unnamed)
    /// session is skipped until the next header.
    MalformedHeader {
        native_session_id: Option<String>,
        line: usize,
        reason: &'static str,
    },
    /// A record that violates the contract inside the current session, which
    /// ends here and is skipped until the next header.
    MalformedRecord {
        native_session_id: String,
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
    /// `Some` only in origin-evidence mode; the inner value is the open
    /// session's evidence bookkeeping.
    origin: Option<Option<OriginSession>>,
}

impl StreamEvents {
    pub fn new(host: Host, source: SessionSource) -> Self {
        Self {
            host,
            source,
            context: None,
            skipping: false,
            line: 0,
            origin: None,
        }
    }

    /// The opt-in Codex origin-evidence mode: headers may carry the evidence
    /// marker, and [`Self::push_with_origin`] judges each record's claim. The
    /// events themselves are exactly those of an ordinary stream: evidence
    /// never ends, skips or alters a session or record.
    pub fn with_codex_origin_evidence() -> Self {
        Self {
            origin: Some(None),
            ..Self::new(Host::Codex, SessionSource::ReadersCli)
        }
    }

    /// Consume one line. A record before any header rejects the stream.
    pub fn push(&mut self, line: &str) -> Result<Option<StreamEvent>, StreamError> {
        Ok(self.push_with_origin(line)?.map(|(event, _)| event))
    }

    /// Like [`Self::push`], with the evidence sidecar of a `Session` or
    /// `Record` event in origin-evidence mode (`None` otherwise).
    pub fn push_with_origin(
        &mut self,
        line: &str,
    ) -> Result<Option<(StreamEvent, Option<Origin>)>, StreamError> {
        self.line += 1;
        let number = self.line;
        if line.trim().is_empty() {
            return Ok(None);
        }
        match classify(line) {
            LineKind::Header => {
                self.context = None;
                self.skipping = false;
                self.close_origin();
                return Ok(Some(match self.header(line, number) {
                    Ok(header) => {
                        self.context = Some((
                            header.native_session_id.clone(),
                            header.context(self.host, self.source),
                        ));
                        let origin = self.origin.as_mut().map(|open| {
                            let session = OriginSession::open(
                                header.origin_evidence,
                                &header.native_session_id,
                            );
                            let state = session.state();
                            *open = Some(session);
                            Origin::Session(state)
                        });
                        (StreamEvent::Session(Box::new(header)), origin)
                    }
                    Err(malformed) => {
                        self.skipping = true;
                        (*malformed, None)
                    }
                }));
            }
            LineKind::MalformedHeader => {
                self.context = None;
                self.skipping = true;
                self.close_origin();
                return Ok(Some((
                    StreamEvent::MalformedHeader {
                        native_session_id: None,
                        line: number,
                        reason: "session header does not match the shared stream contract",
                    },
                    None,
                )));
            }
            LineKind::Other => {}
        }
        if self.skipping {
            return Ok(None);
        }
        let Some((native, context)) = self.context.as_ref() else {
            return Err(StreamError::RecordBeforeHeader { line: number });
        };
        Ok(match parse_with_context(line, context) {
            Ok(Parsed::Record(mut record)) => {
                if self.origin.is_some() {
                    record.human_adjustment = super::human_input::image_evidence(
                        line,
                        native,
                        context.conversation_id.as_deref().unwrap_or(""),
                        &record.canonical,
                    );
                }
                let origin = self
                    .origin
                    .as_mut()
                    .and_then(Option::as_mut)
                    .map(|session| Origin::Record(session.record(line, &record.canonical)));
                Some((StreamEvent::Record(record), origin))
            }
            Ok(Parsed::Dropped(_)) => {
                if let Some(session) = self.origin.as_mut().and_then(Option::as_mut) {
                    session.unparsed(line);
                }
                Some((StreamEvent::Dropped, None))
            }
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) | Err(_) => {
                let native = native.clone();
                self.context = None;
                self.skipping = true;
                self.close_origin();
                Some((
                    StreamEvent::MalformedRecord {
                        native_session_id: native,
                        line: number,
                        reason: "canonical record does not match the shared stream contract",
                    },
                    None,
                ))
            }
        })
    }

    /// In origin-evidence mode, the claimed record UUIDs of the open session
    /// that lines pushed since the last call disputed (see
    /// [`super::origin`]); empty otherwise. Only record and dropped lines
    /// dispute, and the session stays open across them, so a consumer that
    /// takes the disputes after each such line misses none.
    pub fn take_origin_disputes(&mut self) -> Vec<String> {
        self.origin
            .as_mut()
            .and_then(Option::as_mut)
            .map(OriginSession::take_disputes)
            .unwrap_or_default()
    }

    fn close_origin(&mut self) {
        if let Some(open) = self.origin.as_mut() {
            *open = None;
        }
    }

    fn header(&self, line: &str, number: usize) -> Result<SessionHeader, Box<StreamEvent>> {
        let malformed = || StreamEvent::MalformedHeader {
            native_session_id: None,
            line: number,
            reason: "session header does not match the shared stream contract",
        };
        let header = serde_json::from_str::<SessionHeader>(line).map_err(|_| malformed())?;
        if self.origin.is_none() && header.origin_evidence != OriginMarker::Absent {
            return Err(Box::new(malformed()));
        }
        let native = header.native_session_id.clone();
        let reason = if header.host != self.host.as_str() {
            Some("session header names another host")
        } else if native.trim().is_empty() || header.conversation_id.trim().is_empty() {
            Some("session header lacks an identity")
        } else if header.conversation_id != expected_conversation_id(self.host, &native) {
            Some("session header conversation ID does not derive from its native ID")
        } else if [&header.source_surface, &header.cwd, &header.git_branch]
            .into_iter()
            .flatten()
            .any(|label| label.trim().is_empty())
        {
            // The writer rejects empty identity labels once a record arrives;
            // a header-only session must not pass on a technicality.
            Some("session header carries an empty label")
        } else if header.path.trim().is_empty() {
            Some("session header lacks a source path")
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
            Some(reason) => Err(Box::new(StreamEvent::MalformedHeader {
                native_session_id: Some(native),
                line: number,
                reason,
            })),
            None => Ok(header),
        }
    }
}

pub fn expected_conversation_id(host: Host, native: &str) -> String {
    match host {
        Host::Claude => native.to_owned(),
        Host::Codex => format!("codex-{native}"),
        Host::Cursor => format!("cursor-{native}"),
        Host::Other => format!("other-{native}"),
    }
}
