//! Claude Code keeps its history as canonical JSONL under `~/.claude/projects`:
//! `<project>/<session-id>.jsonl` for the main transcript and
//! `<project>/<session-id>/subagents/**/*.jsonl` for sidechains, which belong to
//! the parent session. Each file is lifted into the shared stream shape (one
//! synthesized session header, then the file's records) and imported through
//! the same per-session writer as the reader-produced streams, one bounded
//! batch at a time, so a large transcript is never held in memory as a whole.
//! Files are only ever read.

use super::checkpoint::{self, FileIdentity, ResumeBasis, TailWindow};
use super::readers_cli::ReaderDiagnostic;
use super::stream::{SessionHeader, expected_conversation_id};
use super::{ScanMode, SessionOutcome, SessionResult, SessionWriter};
use crate::canonical::{Parsed, ParsedRecord, SourceContext, parse_with_context};
use crate::writer::MAX_BATCH_RECORDS;
use sha2::Digest;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use xt_store::{Host, SessionSource, Store, batch::SourceCursor};

#[derive(Clone, Debug)]
pub struct ClaudeFile {
    pub path: PathBuf,
    /// The canonical (and native) session the file's records belong to.
    pub session_id: String,
    pub sidechain: bool,
    pub mtime_ns: u128,
    metadata: fs::Metadata,
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
                        Ok((mtime_ns, metadata)) => files.push(ClaudeFile {
                            mtime_ns,
                            metadata,
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
                Ok((mtime_ns, metadata)) => files.push(ClaudeFile {
                    mtime_ns,
                    metadata,
                    path: entry,
                    session_id: session_id.to_owned(),
                    sidechain: true,
                }),
                Err(_) => diagnostics.push(unreadable(&entry)),
            }
        }
    }
}

fn mtime_ns(path: &Path) -> std::io::Result<(u128, fs::Metadata)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("source is not a regular file"));
    }
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Ok((mtime, metadata))
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
    snapshot: &mut fs::File,
    context: &SourceContext,
) -> std::io::Result<Result<Option<String>, (usize, &'static str)>> {
    snapshot.rewind()?;
    let mut reader = BufReader::new(snapshot);
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
        let named = match parse_with_context(text, context) {
            Ok(Parsed::Record(record)) => match record.canonical.source_surface.as_ref() {
                Some(surface) if surface.trim().is_empty() => {
                    return Ok(Err((line_number, "record has an unusable surface")));
                }
                surface => surface.cloned(),
            },
            Ok(Parsed::Dropped(_)) => {
                let raw: serde_json::Value = serde_json::from_str(text)?;
                let value = raw
                    .get("source_surface")
                    .filter(|value| !value.is_null())
                    .or_else(|| raw.get("entrypoint"))
                    .filter(|value| !value.is_null());
                match value {
                    None => None,
                    Some(value) => match value.as_str().filter(|value| !value.trim().is_empty()) {
                        Some(surface) => Some(surface.to_owned()),
                        None => {
                            return Ok(Err((
                                line_number,
                                "dropped record has an unusable surface",
                            )));
                        }
                    },
                }
            }
            Ok(_) => continue,
            Err(_) => return Ok(Ok(surface)),
        };
        if let Some(named) = named {
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
/// the session, so its discovered identity is registered before anything is
/// written. In `Resume` mode the file's checkpoint is proven first (same
/// inode, position still held, trailing bytes unchanged): a proven file is
/// read only behind its position, and an unchanged one touches the index not
/// at all; a replaced, truncated or rewritten file is read whole again. The
/// surface is settled over the bytes to be read before any row is written;
/// each batch fills in whichever labels are still unknown, and those labels
/// persist only with a committed batch. Batches commit as they fill, each
/// carrying the checkpoint through its last complete line, which the store
/// records only if every record of the batch is stored; a scan that dropped
/// or rejected any record advances no checkpoint, and a zero-position source
/// locator is recorded after the last batch only for a file with no rejected
/// or dropped record. A trailing partial line is left unconsumed. A malformed
/// or non-UTF-8 line stops the file with the committed count named; the
/// checkpoints of earlier batches stay.
pub fn import_file(
    store: &mut Store,
    file: &ClaudeFile,
    observed_at: i64,
    mode: ScanMode,
) -> std::io::Result<SessionResult> {
    let key = checkpoint::file_key(&file.path);
    let context = context(&file.session_id);
    let known = match mode {
        ScanMode::Resume => match store.native_checkpoint(SessionSource::Transcript, &key) {
            Ok(known) => known,
            Err(error) => {
                return Ok(skipped(
                    file,
                    format!("checkpoint could not be read: {error}"),
                ));
            }
        },
        ScanMode::Replay => None,
    };
    let opened = open_source(&file.path, &file.metadata).and_then(|(mut source, identity)| {
        let resume = checkpoint::resume_point(known.as_ref(), &mut source, &identity)?;
        Ok((source, identity, resume))
    });
    let (mut source, identity, resume) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            // The file name still identifies the session: a transcript that
            // vanished or became unreadable since enumeration is a known session
            // whose read failure the caller diagnoses.
            if let Err(skipped) = SessionWriter::begin(
                store,
                Host::Claude,
                SessionSource::Transcript,
                &header(file, &[]),
                observed_at,
            ) {
                return Ok(*skipped);
            }
            return Err(error);
        }
    };
    if resume.basis == ResumeBasis::Unchanged {
        // Proven unchanged by digesting the whole prefix (its change time had
        // moved): the checkpoint takes the current identity, so the next
        // proof is the cheap one again instead of another full digest.
        if resume.refresh
            && let Err(error) = store.record_native_checkpoint(&checkpoint::file_checkpoint(
                &key,
                &identity,
                resume.start,
                &resume.prefix,
                &TailWindow::seeded(resume.tail.clone()),
                resume.lines,
                observed_at,
            ))
        {
            return Ok(skipped(
                file,
                format!("checkpoint could not be refreshed: {error}"),
            ));
        }
        // The report names the surface the index already holds for the
        // session, so an unchanged file reads the same as when it was imported.
        let source_surface = store
            .session(&file.session_id)
            .ok()
            .flatten()
            .and_then(|stored| stored.meta.surface);
        return Ok(SessionResult {
            native_session_id: Some(file.session_id.clone()),
            conversation_id: Some(file.session_id.clone()),
            source_surface,
            path: Some(file.path.to_string_lossy().into_owned()),
            outcome: SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0,
            },
        });
    }
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
    let stop = |writer: &SessionWriter, line: u64, reason: &str| {
        writer.abandon(format!(
            "{reason} (stream line {line}); {} earlier batches stay committed",
            writer.batches
        ))
    };
    // Only the bytes behind the resume point are read, into one bounded
    // anonymous snapshot shared by validation and import.
    source.seek(SeekFrom::Start(resume.start))?;
    let mut snapshot = copy_snapshot(source, identity.len - resume.start)?;
    // The bytes to be read must agree on their surface before the first batch
    // can commit that label to the canonical row and the fill-once discovered
    // identity; the check stays per line below in case the file grows.
    let mut surface = match file_surface(&mut snapshot, &context)? {
        Ok(surface) => surface,
        Err((line, reason)) => return Ok(stop(&writer, resume.lines + line as u64, reason)),
    };
    snapshot.rewind()?;
    let mut reader = BufReader::new(snapshot);
    let mut batch: Vec<ParsedRecord> = Vec::new();
    let mut dropped = 0;
    let mut lines = resume.lines;
    let mut consumed: u64 = 0;
    let mut window = TailWindow::seeded(resume.tail);
    let mut prefix = resume.prefix;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || !buffer.ends_with(b"\n") {
            break; // end of file, or a partial trailing line that is not consumed
        }
        lines += 1;
        let Ok(text) = std::str::from_utf8(&buffer) else {
            return Ok(stop(&writer, lines, "transcript is not UTF-8"));
        };
        if !text.trim().is_empty() {
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
                            lines,
                            "record identity disagrees with the file name",
                        ));
                    }
                    if let Some(named) = label(record.canonical.source_surface.as_ref()) {
                        match &surface {
                            Some(known) if *known != named => {
                                return Ok(stop(
                                    &writer,
                                    lines,
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
                        lines,
                        "canonical record does not match the shared stream contract",
                    ));
                }
            }
        }
        // The line is consumed only once it parsed: progress never covers a
        // line the scan could not read.
        consumed += read as u64;
        window.push(&buffer);
        prefix.update(&buffer);
        if batch.len() == MAX_BATCH_RECORDS {
            if !writer.labels_complete() {
                writer.enrich(&header(file, &batch));
            }
            let progress = (dropped == 0 && writer.gapless()).then(|| {
                checkpoint::file_checkpoint(
                    &key,
                    &identity,
                    resume.start + consumed,
                    &prefix,
                    &window,
                    lines,
                    observed_at,
                )
            });
            if let Err(skipped) =
                writer.write_with_checkpoint(store, &batch, observed_at, progress.as_ref())
            {
                return Ok(*skipped);
            }
            batch.clear();
        }
    }
    writer.note_dropped(dropped);
    if !writer.labels_complete() {
        writer.enrich(&header(file, &batch));
    }
    let position = resume.start + consumed;
    let progress = (dropped == 0 && writer.gapless()).then(|| {
        checkpoint::file_checkpoint(
            &key,
            &identity,
            position,
            &prefix,
            &window,
            lines,
            observed_at,
        )
    });
    // A file without a storable record (empty, or inert lines only) keeps its
    // discovered identity and nothing else: an empty batch commits no row,
    // and no cursor is recorded without a committed batch.
    let carried = !batch.is_empty();
    if let Err(skipped) =
        writer.write_with_checkpoint(store, &batch, observed_at, progress.as_ref())
    {
        return Ok(*skipped);
    }
    let gapless = dropped == 0 && writer.gapless();
    let cursor = SourceCursor {
        source: SessionSource::Transcript,
        cursor_key: key.clone(),
        position: 0,
        updated_at: observed_at,
    };
    let result = writer.complete(store, Some(&cursor));
    if !gapless || !matches!(result.outcome, SessionOutcome::Imported { .. }) {
        return Ok(result);
    }
    // Progress no batch carried is recorded on its own, after the rows: the
    // last complete lines were inert, or the scan consumed nothing under a
    // new generation, whose stale checkpoint is then forgotten.
    let recorded = if position == 0 {
        store.clear_native_checkpoint(SessionSource::Transcript, &key)
    } else if carried {
        Ok(())
    } else {
        match &progress {
            Some(progress) => store.record_native_checkpoint(progress),
            None => Ok(()),
        }
    };
    Ok(match recorded {
        Ok(()) => result,
        Err(error) => SessionResult {
            outcome: SessionOutcome::Skipped {
                reason: format!("checkpoint could not be recorded after the rows: {error}"),
            },
            ..result
        },
    })
}

fn skipped(file: &ClaudeFile, reason: String) -> SessionResult {
    SessionResult {
        native_session_id: Some(file.session_id.clone()),
        conversation_id: Some(file.session_id.clone()),
        source_surface: None,
        path: Some(file.path.to_string_lossy().into_owned()),
        outcome: SessionOutcome::Skipped { reason },
    }
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

/// One anonymous temporary file shared by validation and import. Later source
/// appends are outside this scan; no named transcript copy remains on disk.
#[cfg(test)]
fn snapshot_source(path: &Path, expected: &fs::Metadata) -> std::io::Result<fs::File> {
    let (source, identity) = open_source(path, expected)?;
    copy_snapshot(source, identity.len)
}

/// Open the enumerated transcript without following aliases and confirm it is
/// still the file that was enumerated; its identity fixes the length this scan
/// may read, so later appends stay outside it.
fn open_source(path: &Path, expected: &fs::Metadata) -> std::io::Result<(fs::File, FileIdentity)> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let source = options.open(path)?;
    let opened = source.metadata()?;
    if !opened.is_file() {
        return Err(std::io::Error::other("source is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != expected.dev() || opened.ino() != expected.ino() {
            return Err(std::io::Error::other("source changed since discovery"));
        }
    }
    let identity = FileIdentity::of(&source.metadata()?);
    Ok((source, identity))
}

fn copy_snapshot(source: impl Read, length: u64) -> std::io::Result<fs::File> {
    let mut snapshot = tempfile::tempfile()?;
    if std::io::copy(&mut source.take(length), &mut snapshot)? != length {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "source shortened while copying",
        ));
    }
    snapshot.rewind()?;
    Ok(snapshot)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn short_snapshot_copy_is_rejected_before_import() {
        assert_eq!(
            copy_snapshot(&b"short"[..], 10).unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        let mut snapshot = copy_snapshot(&b"exact-plus-append"[..], 5).unwrap();
        let mut bytes = String::new();
        snapshot.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, "exact");
    }

    #[test]
    fn append_between_validation_and_import_cannot_change_snapshot() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("session.jsonl");
        let original = "{\"type\":\"assistant\",\"entrypoint\":\"cli\"}\n";
        fs::write(&source, original).unwrap();
        let mut snapshot = snapshot_source(&source, &fs::metadata(&source).unwrap()).unwrap();
        assert_eq!(
            file_surface(&mut snapshot, &context("session"))
                .unwrap()
                .unwrap()
                .as_deref(),
            Some("cli")
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"{\"type\":\"assistant\",\"entrypoint\":\"sdk\"}\n")
            .unwrap();
        snapshot.rewind().unwrap();
        let mut imported_bytes = String::new();
        snapshot.read_to_string(&mut imported_bytes).unwrap();
        assert_eq!(imported_bytes, original);
        let mut next = snapshot_source(&source, &fs::metadata(&source).unwrap()).unwrap();
        assert!(
            file_surface(&mut next, &context("session"))
                .unwrap()
                .is_err()
        );
    }
}
