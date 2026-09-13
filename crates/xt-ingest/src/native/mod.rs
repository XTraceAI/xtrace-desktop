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
use xt_store::{Host, SessionSource, Store, batch::SourceCursor, ingest::DiscoveredSession};

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
    Imported {
        records_new: usize,
        records_enriched: usize,
        records_dropped: usize,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub hosts: Vec<HostReport>,
}

impl ImportReport {
    /// True only when every requested host imported all of its sessions.
    pub fn complete(&self) -> bool {
        self.hosts.iter().all(|host| {
            host.status == HostStatus::Complete
                && host
                    .sessions
                    .iter()
                    .all(|session| matches!(session.outcome, SessionOutcome::Imported { .. }))
        })
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
    let files = match claude_fs::enumerate(&projects) {
        Ok(files) => files,
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
    let mut diagnostics = Vec::new();
    for file in files {
        match claude_fs::read_file(&file) {
            Ok(read) => {
                let cursor = SourceCursor {
                    source: SessionSource::Transcript,
                    cursor_key: format!("claude:{}", file.path.display()),
                    position: i64::try_from(read.complete_bytes).unwrap_or(i64::MAX),
                    updated_at: request.observed_at,
                };
                sessions.push(import_block(
                    store,
                    Host::Claude,
                    SessionSource::Transcript,
                    read.outcome,
                    Some(cursor),
                    request.observed_at,
                    true,
                ));
            }
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
    let status = if diagnostics.is_empty()
        && sessions
            .iter()
            .all(|s| matches!(s.outcome, SessionOutcome::Imported { .. }))
    {
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
    let run = match readers_cli::run_reader(&python, &producer, host, request.home) {
        Ok(run) => run,
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::ReaderFailed, error.to_string());
        }
    };
    let blocks = match stream::parse_stream(&run.lines, host, SessionSource::ReadersCli) {
        Ok(blocks) => blocks,
        Err(error) => {
            return HostReport::unavailable(host, HostStatus::ReaderFailed, error.to_string());
        }
    };
    let sessions = blocks
        .into_iter()
        .map(|block| {
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
            import_block(
                store,
                host,
                SessionSource::ReadersCli,
                block,
                cursor,
                request.observed_at,
                run.complete,
            )
        })
        .collect::<Vec<_>>();
    let status = if run.complete
        && run.diagnostics.is_empty()
        && sessions
            .iter()
            .all(|s| matches!(s.outcome, SessionOutcome::Imported { .. }))
    {
        HostStatus::Complete
    } else {
        HostStatus::Incomplete
    };
    HostReport {
        host,
        status,
        detail: Some(format!(
            "pinned producer {} (memhub {})",
            producer.commit, producer.plugin_version
        )),
        diagnostics: run.diagnostics,
        sessions,
    }
}

/// Write one session block through the canonical writer in bounded batches.
/// The cursor commits with the final batch, so it never advances past rows
/// that were not written. Titles are never persisted: they derive from prompts.
pub fn import_block(
    store: &mut Store,
    host: Host,
    source: SessionSource,
    block: BlockOutcome,
    cursor: Option<SourceCursor>,
    observed_at: i64,
    discovery_complete: bool,
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
    let context = header.context(host, source);
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
        discovery_complete,
    };
    let mut result = SessionResult {
        native_session_id: Some(header.native_session_id.clone()),
        conversation_id: Some(header.conversation_id.clone()),
        source_surface: header.source_surface.clone(),
        path: Some(header.path.clone()),
        outcome: SessionOutcome::Imported {
            records_new: 0,
            records_enriched: 0,
            records_dropped: dropped,
        },
    };
    if let Err(error) = store.observe_discovered_session(&discovery) {
        result.outcome = SessionOutcome::Skipped {
            reason: format!("discovered identity conflicts with the index: {error}"),
        };
        return result;
    }
    if records.is_empty() {
        // A metadata-only stream or an empty file: identity is discovered,
        // nothing is written, the cursor stays where it was.
        return result;
    }
    let chunks: Vec<&[crate::canonical::ParsedRecord]> =
        records.chunks(MAX_BATCH_RECORDS).collect();
    let last = chunks.len() - 1;
    for (index, chunk) in chunks.into_iter().enumerate() {
        let batch = WriteBatch {
            context: &context,
            declared_host: Some(host),
            records: chunk,
            title: None,
            cwd: header.cwd.as_deref(),
            git_branch: header.git_branch.as_deref(),
            namespace: None,
            // The persisted policy decides; an unconfigured store keeps metadata only.
            keep_content: true,
            observed_at,
            receipt: None,
            cursor: if index == last { cursor.as_ref() } else { None },
        };
        match write_batch(store, &batch) {
            Ok(saved) => {
                if let SessionOutcome::Imported {
                    records_new,
                    records_enriched,
                    records_dropped,
                } = &mut result.outcome
                {
                    *records_new += saved.records_new;
                    *records_enriched += saved.records_enriched;
                    *records_dropped += saved.records_dropped;
                }
            }
            Err(error) => {
                result.outcome = SessionOutcome::Skipped {
                    reason: format!(
                        "session could not be written after {index} committed batches: {error}"
                    ),
                };
                return result;
            }
        }
    }
    result
}
