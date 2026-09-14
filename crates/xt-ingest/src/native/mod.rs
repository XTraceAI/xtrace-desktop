//! Initial import of native session history into the local store.
//!
//! Codex and Cursor history is read by the pinned shared readers from
//! `.plugin-pin`; Claude history is read directly from its canonical JSONL.
//! Every host goes through one path: the shared stream (header + records) is
//! split into sessions and each session is written through the transactional
//! canonical writer under the persisted content policy, which defaults to
//! metadata only. Native files are never written, no plugin needs to be
//! installed and nothing contacts a network. Absent sources, a missing Python
//! runtime, a producer that is not the pinned one, reader failures and
//! malformed output are reported explicitly per host and per session, while
//! every readable session still imports.

pub mod claude_fs;
pub mod readers_cli;
pub mod stream;

use crate::writer::{MAX_BATCH_RECORDS, WriteBatch, write_batch};
use readers_cli::{ReaderDiagnostic, ReaderError};
use serde::Serialize;
use std::{ffi::OsStr, path::Path};
use stream::{BlockOutcome, SessionBlock};
use xt_store::{
    Host, SessionSource, Store,
    batch::{RecordDisposition, SourceCursor},
    ingest::DiscoveredSession,
};

pub struct ImportRequest<'a> {
    /// The home directory whose `.claude`, `.codex` and `.cursor` trees are read.
    pub home: &'a Path,
    pub hosts: &'a [Host],
    /// The pin file naming the producer; required for Codex and Cursor.
    pub pin: &'a Path,
    /// The pinned plugin root (`<checkout>/plugins/memhub`); required for Codex and Cursor.
    pub plugin_root: Option<&'a Path>,
    pub python: Option<&'a OsStr>,
    /// UTC milliseconds recorded as the observation time of this import.
    pub observed_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    /// Every discovered session of this host was read and imported.
    Complete,
    /// The producer or a session reported incomplete coverage; readable
    /// sessions are imported and the gaps are listed.
    Incomplete,
    MissingSource,
    MissingRuntime,
    PinMismatch,
    ReaderFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum SessionOutcome {
    /// Every record of the session was accepted.
    Imported {
        records_new: usize,
        records_enriched: usize,
    },
    /// The session's identity and accepted records are stored, but some records
    /// could not be: they lacked a UUID, or the writer rejected them (a UUID
    /// owned by another session, a type conflict). Coverage is incomplete.
    Partial {
        records_new: usize,
        records_enriched: usize,
        records_dropped: usize,
        rejections: Vec<String>,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionResult {
    pub native_session_id: Option<String>,
    pub conversation_id: Option<String>,
    pub source_surface: Option<String>,
    /// Local source locator; index metadata only, never a cloud or telemetry payload.
    pub path: Option<String>,
    #[serde(flatten)]
    pub outcome: SessionOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostReport {
    pub host: Host,
    pub status: HostStatus,
    pub detail: Option<String>,
    pub diagnostics: Vec<ReaderDiagnostic>,
    pub sessions: Vec<SessionResult>,
}

impl HostReport {
    fn unavailable(host: Host, status: HostStatus, detail: impl Into<String>) -> Self {
        Self {
            host,
            status,
            detail: Some(detail.into()),
            diagnostics: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

fn all_imported(sessions: &[SessionResult]) -> bool {
    sessions
        .iter()
        .all(|session| matches!(session.outcome, SessionOutcome::Imported { .. }))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub hosts: Vec<HostReport>,
}

impl ImportReport {
    /// True only when every requested host imported every record of every session.
    pub fn complete(&self) -> bool {
        self.hosts
            .iter()
            .all(|host| host.status == HostStatus::Complete)
    }
}

/// Import every requested host. Failures are reported, never raised: one host's
/// missing runtime does not stop another host's import.
pub fn import_native(store: &mut Store, request: &ImportRequest<'_>) -> ImportReport {
    let hosts = request
        .hosts
        .iter()
        .map(|host| match host {
            Host::Claude => import_claude(store, request),
            Host::Codex | Host::Cursor => import_reader_host(store, request, *host),
            Host::Other => HostReport::unavailable(
                Host::Other,
                HostStatus::MissingSource,
                "no native reader exists for an unknown host",
            ),
        })
        .collect();
    ImportReport { hosts }
}

fn import_claude(store: &mut Store, request: &ImportRequest<'_>) -> HostReport {
    let projects = request.home.join(".claude").join("projects");
    if !projects.is_dir() {
        return HostReport::unavailable(
            Host::Claude,
            HostStatus::MissingSource,
            "~/.claude/projects is absent",
        );
    }
    let (files, mut diagnostics) = match claude_fs::enumerate(&projects) {
        Ok(found) => found,
        Err(error) => {
            return HostReport::unavailable(
                Host::Claude,
                HostStatus::ReaderFailed,
                format!(
                    "~/.claude/projects could not be enumerated: {}",
                    error.kind()
                ),
            );
        }
    };
    let mut sessions = Vec::new();
    for file in files {
        match claude_fs::import_file(store, &file, request.observed_at) {
            Ok(result) => sessions.push(result),
            Err(error) => {
                diagnostics.push(ReaderDiagnostic {
                    code: "session_unreadable".into(),
                    path: Some(file.path.to_string_lossy().into_owned()),
                });
                sessions.push(SessionResult {
                    native_session_id: Some(file.session_id.clone()),
                    conversation_id: Some(file.session_id.clone()),
                    source_surface: None,
                    path: Some(file.path.to_string_lossy().into_owned()),
                    outcome: SessionOutcome::Skipped {
                        reason: format!("file could not be read: {}", error.kind()),
                    },
                });
            }
        }
    }
    let status = if diagnostics.is_empty() && all_imported(&sessions) {
        HostStatus::Complete
    } else {
        HostStatus::Incomplete
    };
    HostReport {
        host: Host::Claude,
        status,
        detail: None,
        diagnostics,
        sessions,
    }
}

fn reader_sources_present(home: &Path, host: Host) -> bool {
    match host {
        Host::Codex => home.join(".codex").join("sessions").is_dir(),
        Host::Cursor => {
            home.join(".cursor").join("chats").is_dir()
                || home.join(".cursor").join("projects").is_dir()
        }
        Host::Claude | Host::Other => false,
    }
}

fn import_reader_host(store: &mut Store, request: &ImportRequest<'_>, host: Host) -> HostReport {
    if !reader_sources_present(request.home, host) {
        return HostReport::unavailable(
            host,
            HostStatus::MissingSource,
            format!("no {} session directory under the home", host.as_str()),
        );
    }
    let python = match readers_cli::resolve_python(request.python) {
        Ok(python) => python,
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::MissingRuntime, error.to_string());
        }
    };
    let producer = match request
        .plugin_root
        .ok_or_else(|| ReaderError::PinMismatch("no pinned plugin root was supplied".into()))
        .and_then(|root| {
            readers_cli::read_pin(request.pin).and_then(|pin| readers_cli::verify_pin(&pin, root))
        }) {
        Ok(producer) => producer,
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::PinMismatch, error.to_string());
        }
    };
    let mut process = match readers_cli::spawn_reader(&python, &producer, host, request.home) {
        Ok(process) => process,
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::ReaderFailed, error.to_string());
        }
    };
    // Sessions import as their blocks complete, so a large history never sits
    // in memory as a whole and every session read before a later failure stays.
    let mut reader = stream::BlockReader::new(host, SessionSource::ReadersCli);
    let mut sessions = Vec::new();
    let mut stream_failure = None;
    let settle = |store: &mut Store, block: BlockOutcome, sessions: &mut Vec<SessionResult>| {
        let cursor = match &block {
            BlockOutcome::Ready(ready) => Some(SourceCursor {
                source: SessionSource::ReadersCli,
                cursor_key: format!("{}:{}", host.as_str(), ready.header.path),
                // The reader's native update clock in milliseconds; byte offsets
                // belong to the per-file cursors of incremental scanning.
                position: (ready.header.mtime * 1000.0) as i64,
                updated_at: request.observed_at,
            }),
            BlockOutcome::Malformed { .. } => None,
        };
        sessions.push(import_block(
            store,
            host,
            SessionSource::ReadersCli,
            block,
            cursor,
            request.observed_at,
        ));
    };
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::BufRead::read_line(&mut process.stdout, &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                stream_failure = Some("reader stream could not be read".to_owned());
                break;
            }
        }
        let text = line.trim_end_matches(['\n', '\r']);
        match reader.push(text) {
            Ok(Some(block)) => settle(store, block, &mut sessions),
            Ok(None) => {}
            Err(error) => {
                stream_failure = Some(error.to_string());
                break;
            }
        }
        if let Some(queued) = reader.take_queued() {
            settle(store, queued, &mut sessions);
        }
    }
    if stream_failure.is_none()
        && let Some(block) = reader.finish()
    {
        settle(store, block, &mut sessions);
    }
    let outcome = process.finish();
    let (status, detail, diagnostics) = match (stream_failure, outcome) {
        (Some(failure), _) => (HostStatus::ReaderFailed, failure, Vec::new()),
        (None, Err(error)) => (HostStatus::ReaderFailed, error.to_string(), Vec::new()),
        (None, Ok(run)) => {
            let status = if run.complete && run.diagnostics.is_empty() && all_imported(&sessions) {
                HostStatus::Complete
            } else {
                HostStatus::Incomplete
            };
            (
                status,
                format!(
                    "pinned producer {} (memhub {})",
                    producer.commit, producer.plugin_version
                ),
                run.diagnostics,
            )
        }
    };
    HostReport {
        host,
        status,
        detail: Some(detail),
        diagnostics,
        sessions,
    }
}

/// One session's write sequence: the header registers the discovered
/// identity first, then bounded record batches commit one at a time, and the
/// cursor commits with the last batch only, so it never advances past rows
/// that were not written. Titles are never persisted: they derive from prompts.
pub struct SessionWriter {
    host: Host,
    context: crate::canonical::SourceContext,
    cwd: Option<String>,
    git_branch: Option<String>,
    result: SessionResult,
    new: usize,
    enriched: usize,
    rejected: Vec<String>,
    batches: usize,
}

impl SessionWriter {
    /// Register the header's identity. A conflicting discovered identity is an
    /// explicit skip before anything is written.
    pub fn begin(
        store: &mut Store,
        host: Host,
        source: SessionSource,
        header: &stream::SessionHeader,
        observed_at: i64,
    ) -> Result<Self, Box<SessionResult>> {
        let result = SessionResult {
            native_session_id: Some(header.native_session_id.clone()),
            conversation_id: Some(header.conversation_id.clone()),
            source_surface: header.source_surface.clone(),
            path: Some(header.path.clone()),
            outcome: SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0,
            },
        };
        let discovery = DiscoveredSession {
            host,
            native_session_id: header.native_session_id.clone(),
            conversation_id: Some(header.conversation_id.clone()),
            surface: header.source_surface.clone(),
            started_at_ms: header
                .started_at
                .as_deref()
                .and_then(|value| xt_store::timestamp::parse(value).ok())
                .map(|(_, millis)| millis),
            last_observed_at: observed_at,
            // The identity is fully known from the header; host-level coverage is
            // reported separately.
            discovery_complete: true,
        };
        if let Err(error) = store.observe_discovered_session(&discovery) {
            return Err(Box::new(SessionResult {
                outcome: SessionOutcome::Skipped {
                    reason: format!("discovered identity conflicts with the index: {error}"),
                },
                ..result
            }));
        }
        Ok(Self {
            host,
            context: header.context(host, source),
            cwd: header.cwd.clone(),
            git_branch: header.git_branch.clone(),
            result,
            new: 0,
            enriched: 0,
            rejected: Vec::new(),
            batches: 0,
        })
    }

    /// Records the parser could not identify; they count against coverage.
    pub fn note_dropped(&mut self, count: usize) {
        self.rejected
            .extend(std::iter::repeat_n("missing_uuid".to_owned(), count));
    }

    /// Commit one bounded batch. `cursor` belongs only to the final batch.
    pub fn write(
        &mut self,
        store: &mut Store,
        records: &[crate::canonical::ParsedRecord],
        observed_at: i64,
        cursor: Option<&SourceCursor>,
    ) -> Result<(), Box<SessionResult>> {
        if records.is_empty() {
            return Ok(());
        }
        let batch = WriteBatch {
            context: &self.context,
            declared_host: Some(self.host),
            records,
            title: None,
            cwd: self.cwd.as_deref(),
            git_branch: self.git_branch.as_deref(),
            namespace: None,
            // The persisted policy decides; an unconfigured store keeps metadata only.
            keep_content: true,
            observed_at,
            receipt: None,
            cursor,
        };
        match write_batch(store, &batch) {
            Ok(saved) => {
                self.new += saved.records_new;
                self.enriched += saved.records_enriched;
                // A rejection is a record this import could not store; it must
                // never read as complete coverage.
                self.rejected
                    .extend(saved.dropped_reasons.iter().map(
                        |(_, disposition)| match disposition {
                            RecordDisposition::RejectedOwnership => "rejected_ownership".to_owned(),
                            RecordDisposition::RejectedType => "rejected_type".to_owned(),
                            RecordDisposition::DroppedMissingUuid => "missing_uuid".to_owned(),
                            other => format!("{other:?}").to_lowercase(),
                        },
                    ));
                self.batches += 1;
                Ok(())
            }
            Err(error) => Err(Box::new(self.abandon(format!(
                "session could not be written after {} committed batches: {error}",
                self.batches
            )))),
        }
    }

    /// Stop with an explicit reason; whatever earlier batches committed stays.
    pub fn abandon(&self, reason: String) -> SessionResult {
        SessionResult {
            outcome: SessionOutcome::Skipped { reason },
            ..self.result.clone()
        }
    }

    pub fn finish(self) -> SessionResult {
        let outcome = if self.rejected.is_empty() {
            SessionOutcome::Imported {
                records_new: self.new,
                records_enriched: self.enriched,
            }
        } else {
            SessionOutcome::Partial {
                records_new: self.new,
                records_enriched: self.enriched,
                records_dropped: self.rejected.len(),
                rejections: self.rejected,
            }
        };
        SessionResult {
            outcome,
            ..self.result
        }
    }
}

/// Write one complete session block through the canonical writer.
pub fn import_block(
    store: &mut Store,
    host: Host,
    source: SessionSource,
    block: BlockOutcome,
    cursor: Option<SourceCursor>,
    observed_at: i64,
) -> SessionResult {
    let SessionBlock {
        header,
        records,
        dropped,
        ..
    } = match block {
        BlockOutcome::Ready(block) => *block,
        BlockOutcome::Malformed {
            native_session_id,
            line,
            reason,
        } => {
            return SessionResult {
                conversation_id: native_session_id
                    .as_deref()
                    .map(|native| stream::expected_conversation_id(host, native)),
                native_session_id,
                source_surface: None,
                path: None,
                outcome: SessionOutcome::Skipped {
                    reason: format!("{reason} (stream line {line})"),
                },
            };
        }
    };
    let mut writer = match SessionWriter::begin(store, host, source, &header, observed_at) {
        Ok(writer) => writer,
        Err(skipped) => return *skipped,
    };
    writer.note_dropped(dropped);
    let chunks: Vec<&[crate::canonical::ParsedRecord]> =
        records.chunks(MAX_BATCH_RECORDS).collect();
    let last = chunks.len().saturating_sub(1);
    for (index, chunk) in chunks.into_iter().enumerate() {
        let cursor = if index == last { cursor.as_ref() } else { None };
        if let Err(skipped) = writer.write(store, chunk, observed_at, cursor) {
            return *skipped;
        }
    }
    writer.finish()
}
