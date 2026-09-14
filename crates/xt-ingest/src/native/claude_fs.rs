//! Claude Code keeps its history as canonical JSONL under `~/.claude/projects`:
//! `<project>/<session-id>.jsonl` for the main transcript and
//! `<project>/<session-id>/subagents/**/*.jsonl` for sidechains, which belong to
//! the parent session. Each file is lifted into the shared stream shape (one
//! synthesized session header, then the file's records) and imported through
//! the same per-session writer as the reader-produced streams, one bounded
//! batch at a time, so a large transcript is never held in memory as a whole.
//! Files are only ever read.

use super::readers_cli::ReaderDiagnostic;
use super::stream::{SessionHeader, expected_conversation_id};
use super::{SessionOutcome, SessionResult, SessionWriter};
use crate::canonical::{Parsed, ParsedRecord, SourceContext, parse_with_context};
use crate::writer::MAX_BATCH_RECORDS;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use xt_store::{Host, SessionSource, Store, batch::SourceCursor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeFile {
    pub path: PathBuf,
    /// The canonical (and native) session the file's records belong to.
    pub session_id: String,
    pub sidechain: bool,
    pub mtime_ns: u128,
}

fn unreadable(path: &Path) -> ReaderDiagnostic {
    ReaderDiagnostic {
        code: "discovery_incomplete".into(),
        path: Some(path.to_string_lossy().into_owned()),
    }
}

/// Every main transcript and sidechain file under the projects root, newest
/// first, plus a diagnostic for each project or entry that could not be
/// listed; the other projects still enumerate. `memory/` trees and non-JSONL
/// files are ignored; nothing is opened. Only an unreadable root is an error.
pub fn enumerate(projects: &Path) -> std::io::Result<(Vec<ClaudeFile>, Vec<ReaderDiagnostic>)> {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    for project in fs::read_dir(projects)? {
        let Ok(project) = project.map(|entry| entry.path()) else {
            diagnostics.push(unreadable(projects));
            continue;
        };
        match fs::symlink_metadata(&project) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => continue,
            Err(_) => {
                diagnostics.push(unreadable(&project));
                continue;
            }
        }
        let Ok(entries) = fs::read_dir(&project) else {
            diagnostics.push(unreadable(&project));
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry.map(|entry| entry.path()) else {
                diagnostics.push(unreadable(&project));
                continue;
            };
            let name = entry
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = fs::symlink_metadata(&entry).map(|meta| meta.file_type()) else {
                diagnostics.push(unreadable(&entry));
                continue;
            };
            if kind.is_symlink() {
                // An alias is not a discovered transcript: it is reported, not followed.
                diagnostics.push(unreadable(&entry));
                continue;
            }
            if kind.is_file() {
                if let Some(stem) = name.strip_suffix(".jsonl").filter(|stem| !stem.is_empty()) {
                    match mtime_ns(&entry) {
                        Ok(mtime_ns) => files.push(ClaudeFile {
                            mtime_ns,
                            path: entry.clone(),
                            session_id: stem.to_owned(),
                            sidechain: false,
                        }),
                        Err(_) => diagnostics.push(unreadable(&entry)),
                    }
                }
            } else if kind.is_dir() && name != "memory" {
                let subagents = entry.join("subagents");
                match fs::symlink_metadata(&subagents) {
                    Ok(meta) if meta.is_dir() => {
                        collect_jsonl(&subagents, name, &mut files, &mut diagnostics)
                    }
                    Ok(meta) if meta.file_type().is_symlink() => {
                        diagnostics.push(unreadable(&subagents))
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => diagnostics.push(unreadable(&subagents)),
                }
            }
        }
    }
    files.sort_by(|a, b| {
        b.mtime_ns
            .cmp(&a.mtime_ns)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok((files, diagnostics))
}

fn collect_jsonl(
    directory: &Path,
    session_id: &str,
    files: &mut Vec<ClaudeFile>,
    diagnostics: &mut Vec<ReaderDiagnostic>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.push(unreadable(directory));
        return;
    };
    for entry in entries {
        let Ok(entry) = entry.map(|entry| entry.path()) else {
            diagnostics.push(unreadable(directory));
            continue;
        };
        let name = entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = fs::symlink_metadata(&entry).map(|meta| meta.file_type()) else {
            diagnostics.push(unreadable(&entry));
            continue;
        };
        if kind.is_symlink() {
            diagnostics.push(unreadable(&entry));
        } else if kind.is_dir() {
            collect_jsonl(&entry, session_id, files, diagnostics);
        } else if kind.is_file() && name.ends_with(".jsonl") {
            match mtime_ns(&entry) {
                Ok(mtime_ns) => files.push(ClaudeFile {
                    mtime_ns,
                    path: entry,
                    session_id: session_id.to_owned(),
                    sidechain: true,
                }),
                Err(_) => diagnostics.push(unreadable(&entry)),
            }
        }
    }
}

fn mtime_ns(path: &Path) -> std::io::Result<u128> {
    Ok(fs::metadata(path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default())
}

fn context(session: &str) -> SourceContext {
    SourceContext {
        conversation_id: Some(session.to_owned()),
        native_session_id: Some(session.to_owned()),
        source_platform: Some(Host::Claude.as_str().to_owned()),
        source_surface: None,
        started_at: None,
        source: Some(SessionSource::Transcript),
    }
}

fn header(file: &ClaudeFile, first: &[ParsedRecord]) -> SessionHeader {
    SessionHeader {
        kind: "session".into(),
        host: Host::Claude.as_str().into(),
        conversation_id: expected_conversation_id(Host::Claude, &file.session_id),
        native_session_id: file.session_id.clone(),
        source_surface: first.iter().find_map(|r| r.native.entrypoint.clone()),
        started_at: None,
        cwd: first.iter().find_map(|r| r.canonical.cwd.clone()),
        git_branch: first.iter().find_map(|r| r.canonical.git_branch.clone()),
        title: None,
        path: file.path.to_string_lossy().into_owned(),
        mtime: file.mtime_ns as f64 / 1_000_000_000.0,
    }
}

/// Import one file as a stream of complete lines. The synthesized header takes
/// its surface, cwd and branch from the first batch's records; batches commit
/// as they fill, and the file cursor (bytes through the last complete line)
/// commits with the last one. A trailing partial line is left unconsumed. A
/// malformed or non-UTF-8 line stops the file with the committed count named
/// and without advancing the cursor; the parent session's earlier batches stay.
pub fn import_file(
    store: &mut Store,
    file: &ClaudeFile,
    observed_at: i64,
) -> std::io::Result<SessionResult> {
    let mut reader = BufReader::new(fs::File::open(&file.path)?);
    let context = context(&file.session_id);
    let mut writer: Option<SessionWriter> = None;
    let mut batch: Vec<ParsedRecord> = Vec::new();
    let mut dropped = 0;
    let mut complete_bytes: u64 = 0;
    let mut line_number = 0usize;
    let mut buffer = Vec::new();
    let skip = |file: &ClaudeFile, line: usize, reason: &str, batches: Option<&SessionWriter>| {
        SessionResult {
            native_session_id: Some(file.session_id.clone()),
            conversation_id: Some(file.session_id.clone()),
            source_surface: None,
            path: Some(file.path.to_string_lossy().into_owned()),
            outcome: SessionOutcome::Skipped {
                reason: match batches {
                    Some(writer) => format!(
                        "{reason} (stream line {line}); {} earlier batches stay committed",
                        writer.batches
                    ),
                    None => format!("{reason} (stream line {line})"),
                },
            },
        }
    };
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || !buffer.ends_with(b"\n") {
            break; // end of file, or a partial trailing line that is not consumed
        }
        complete_bytes += read as u64;
        line_number += 1;
        let Ok(text) = std::str::from_utf8(&buffer) else {
            return Ok(skip(
                file,
                line_number,
                "transcript is not UTF-8",
                writer.as_ref(),
            ));
        };
        if text.trim().is_empty() {
            continue;
        }
        match parse_with_context(text, &context) {
            Ok(Parsed::Record(record)) => batch.push(*record),
            Ok(Parsed::Dropped(_)) => dropped += 1,
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => {}
            Err(_) => {
                return Ok(skip(
                    file,
                    line_number,
                    "canonical record does not match the shared stream contract",
                    writer.as_ref(),
                ));
            }
        }
        if batch.len() == MAX_BATCH_RECORDS {
            let writer_ref = match writer.as_mut() {
                Some(existing) => existing,
                None => match SessionWriter::begin(
                    store,
                    Host::Claude,
                    SessionSource::Transcript,
                    &header(file, &batch),
                    observed_at,
                ) {
                    Ok(started) => writer.insert(started),
                    Err(skipped) => return Ok(*skipped),
                },
            };
            if let Err(skipped) = writer_ref.write(store, &batch, observed_at, None) {
                return Ok(*skipped);
            }
            batch.clear();
        }
    }
    if writer.is_none() && batch.is_empty() && dropped == 0 {
        // An empty or header-less file: nothing to identify or write.
        return Ok(SessionResult {
            native_session_id: Some(file.session_id.clone()),
            conversation_id: Some(file.session_id.clone()),
            source_surface: None,
            path: Some(file.path.to_string_lossy().into_owned()),
            outcome: SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0,
            },
        });
    }
    let mut writer_value = match writer {
        Some(existing) => existing,
        None => match SessionWriter::begin(
            store,
            Host::Claude,
            SessionSource::Transcript,
            &header(file, &batch),
            observed_at,
        ) {
            Ok(started) => started,
            Err(skipped) => return Ok(*skipped),
        },
    };
    writer_value.note_dropped(dropped);
    let cursor = SourceCursor {
        source: SessionSource::Transcript,
        cursor_key: format!("claude:{}", file.path.display()),
        position: i64::try_from(complete_bytes).unwrap_or(i64::MAX),
        updated_at: observed_at,
    };
    if let Err(skipped) = writer_value.write(store, &batch, observed_at, Some(&cursor)) {
        return Ok(*skipped);
    }
    Ok(writer_value.finish())
}
