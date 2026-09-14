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
use super::{SessionResult, SessionWriter};
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
            Ok(meta) if meta.file_type().is_symlink() => {
                diagnostics.push(unreadable(&project));
                continue;
            }
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
            let Some(name) = entry_name(&entry, &mut diagnostics) else {
                continue;
            };
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
                if let Some(stem) = name.strip_suffix(".jsonl") {
                    if stem.trim().is_empty() {
                        diagnostics.push(unreadable(&entry));
                        continue;
                    }
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
                if name.trim().is_empty() {
                    diagnostics.push(unreadable(&entry));
                    continue;
                }
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
        let Some(name) = entry_name(&entry, diagnostics) else {
            continue;
        };
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

/// A record-derived label; a blank value is no label. The writer rejects a
/// record whose identity label is blank, so it must never reach the
/// discovered identity first, where a fill-once value would outlive the skip.
fn label(value: Option<&String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty()).cloned()
}

fn header(file: &ClaudeFile, first: &[ParsedRecord]) -> SessionHeader {
    SessionHeader {
        kind: "session".into(),
        host: Host::Claude.as_str().into(),
        conversation_id: expected_conversation_id(Host::Claude, &file.session_id),
        native_session_id: file.session_id.clone(),
        source_surface: first
            .iter()
            .find_map(|r| label(r.canonical.source_surface.as_ref())),
        started_at: None,
        cwd: first.iter().find_map(|r| label(r.canonical.cwd.as_ref())),
        git_branch: first
            .iter()
            .find_map(|r| label(r.canonical.git_branch.as_ref())),
        title: None,
        path: file.path.to_string_lossy().into_owned(),
        mtime: file.mtime_ns as f64 / 1_000_000_000.0,
    }
}

/// The surface the whole file agrees on, settled before any row is written:
/// the first non-blank surface a record names, which every later record must
/// repeat. One streaming pass, nothing retained; a line the parser rejects
/// ends the pass early, and the import pass reports it precisely.
fn file_surface(
    file: &ClaudeFile,
    context: &SourceContext,
) -> std::io::Result<Result<Option<String>, (usize, &'static str)>> {
    let mut reader = BufReader::new(fs::File::open(&file.path)?);
    let mut surface: Option<String> = None;
    let mut line_number = 0usize;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || !buffer.ends_with(b"\n") {
            return Ok(Ok(surface));
        }
        line_number += 1;
        let Ok(text) = std::str::from_utf8(&buffer) else {
            return Ok(Ok(surface));
        };
        if text.trim().is_empty() {
            continue;
        }
        let record = match parse_with_context(text, context) {
            Ok(Parsed::Record(record)) => record,
            Ok(_) => continue,
            Err(_) => return Ok(Ok(surface)),
        };
        if let Some(named) = label(record.canonical.source_surface.as_ref()) {
            match &surface {
                Some(known) if *known != named => {
                    return Ok(Err((
                        line_number,
                        "record surface disagrees with the file's surface",
                    )));
                }
                Some(_) => {}
                None => surface = Some(named),
            }
        }
    }
}

/// Import one file as a stream of complete lines. The file name identifies
/// the session, so its discovered identity is registered before any line is
/// parsed. The surface is settled over the whole file before any row is
/// written, so a label committed with the first batch is never contradicted
/// by a later record; each batch then fills in whichever of the surface, cwd
/// and branch is still unknown, and those labels persist only with a
/// committed batch. Batches commit as they fill, and the file cursor (bytes
/// through the last complete line) is recorded after the last one, only for
/// a file with no rejected or dropped record. A trailing partial line is left
/// unconsumed. A malformed or non-UTF-8 line stops the file with the
/// committed count named and without advancing the cursor; the parent
/// session's earlier batches stay.
pub fn import_file(
    store: &mut Store,
    file: &ClaudeFile,
    observed_at: i64,
) -> std::io::Result<SessionResult> {
    // The identity is registered before the open, so a transcript that
    // vanished or became unreadable since enumeration is still a known session.
    let mut writer = match SessionWriter::begin(
        store,
        Host::Claude,
        SessionSource::Transcript,
        &header(file, &[]),
        observed_at,
    ) {
        Ok(started) => started,
        Err(skipped) => return Ok(*skipped),
    };
    let context = context(&file.session_id);
    let stop = |writer: &SessionWriter, line: usize, reason: &str| {
        writer.abandon(format!(
            "{reason} (stream line {line}); {} earlier batches stay committed",
            writer.batches
        ))
    };
    // The whole file must agree on its surface before the first batch can
    // commit that label to the canonical row and the fill-once discovered
    // identity; the check stays per line below in case the file grows.
    let mut surface = match file_surface(file, &context)? {
        Ok(surface) => surface,
        Err((line, reason)) => return Ok(stop(&writer, line, reason)),
    };
    let mut reader = BufReader::new(fs::File::open(&file.path)?);
    let mut batch: Vec<ParsedRecord> = Vec::new();
    let mut dropped = 0;
    let mut complete_bytes: u64 = 0;
    let mut line_number = 0usize;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || !buffer.ends_with(b"\n") {
            break; // end of file, or a partial trailing line that is not consumed
        }
        complete_bytes += read as u64;
        line_number += 1;
        let Ok(text) = std::str::from_utf8(&buffer) else {
            return Ok(stop(&writer, line_number, "transcript is not UTF-8"));
        };
        if text.trim().is_empty() {
            continue;
        }
        match parse_with_context(text, &context) {
            Ok(Parsed::Record(record)) => {
                // The file name is this session's identity; a record that
                // names another session stops the file before its metadata
                // could enrich the registered identity.
                if record
                    .native
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id != file.session_id)
                {
                    return Ok(stop(
                        &writer,
                        line_number,
                        "record identity disagrees with the file name",
                    ));
                }
                if let Some(named) = label(record.canonical.source_surface.as_ref()) {
                    match &surface {
                        Some(known) if *known != named => {
                            return Ok(stop(
                                &writer,
                                line_number,
                                "record surface disagrees with the file's surface",
                            ));
                        }
                        Some(_) => {}
                        None => surface = Some(named),
                    }
                }
                batch.push(*record);
            }
            Ok(Parsed::Dropped(_)) => dropped += 1,
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => {}
            Err(_) => {
                return Ok(stop(
                    &writer,
                    line_number,
                    "canonical record does not match the shared stream contract",
                ));
            }
        }
        if batch.len() == MAX_BATCH_RECORDS {
            if !writer.labels_complete() {
                writer.enrich(&header(file, &batch));
            }
            if let Err(skipped) = writer.write(store, &batch, observed_at) {
                return Ok(*skipped);
            }
            batch.clear();
        }
    }
    writer.note_dropped(dropped);
    if !writer.labels_complete() {
        writer.enrich(&header(file, &batch));
    }
    // A file without a storable record (empty, or inert lines only) keeps its
    // discovered identity and nothing else: an empty batch commits no row,
    // and no cursor is recorded without a committed batch.
    if let Err(skipped) = writer.write(store, &batch, observed_at) {
        return Ok(*skipped);
    }
    let cursor = SourceCursor {
        source: SessionSource::Transcript,
        cursor_key: format!("claude:{}", file.path.display()),
        position: i64::try_from(complete_bytes).unwrap_or(i64::MAX),
        updated_at: observed_at,
    };
    Ok(writer.complete(store, Some(&cursor)))
}

fn entry_name<'a>(path: &'a Path, diagnostics: &mut Vec<ReaderDiagnostic>) -> Option<&'a str> {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => Some(name),
        None => {
            diagnostics.push(unreadable(path));
            None
        }
    }
}

#[cfg(all(test, unix))]
mod name_tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    #[test]
    fn undecodable_name_adds_a_discovery_diagnostic() {
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"invalid-\xff.jsonl".to_vec()));
        let mut diagnostics = Vec::new();
        assert!(entry_name(&path, &mut diagnostics).is_none());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "discovery_incomplete");
        assert_eq!(
            entry_name(Path::new("valid.jsonl"), &mut diagnostics),
            Some("valid.jsonl")
        );
        assert_eq!(diagnostics.len(), 1);
    }
}
