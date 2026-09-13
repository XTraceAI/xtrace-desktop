//! Claude Code keeps its history as canonical JSONL under `~/.claude/projects`:
//! `<project>/<session-id>.jsonl` for the main transcript and
//! `<project>/<session-id>/subagents/**/*.jsonl` for sidechains, which belong to
//! the parent session. This lifts each file into the shared stream shape (one
//! synthesized session header, then the file's records) so Claude imports take
//! the same path as the reader-produced streams. Files are only ever read.

use super::stream::{BlockOutcome, SessionBlock, SessionHeader, expected_conversation_id};
use crate::canonical::{Parsed, SourceContext, parse_with_context};
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use xt_store::{Host, SessionSource};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeFile {
    pub path: PathBuf,
    /// The canonical (and native) session the file's records belong to.
    pub session_id: String,
    pub sidechain: bool,
    pub mtime_ns: u128,
}

/// Every main transcript and sidechain file under the projects root, newest
/// first. `memory/` trees and non-JSONL files are ignored; nothing is opened.
pub fn enumerate(projects: &Path) -> std::io::Result<Vec<ClaudeFile>> {
    let mut files = Vec::new();
    for project in fs::read_dir(projects)? {
        let project = project?.path();
        if !project.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&project)? {
            let entry = entry?.path();
            let name = entry
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            if entry.is_file() {
                if let Some(stem) = name.strip_suffix(".jsonl").filter(|stem| !stem.is_empty()) {
                    files.push(ClaudeFile {
                        mtime_ns: mtime_ns(&entry)?,
                        path: entry.clone(),
                        session_id: stem.to_owned(),
                        sidechain: false,
                    });
                }
            } else if entry.is_dir() && name != "memory" {
                let subagents = entry.join("subagents");
                if subagents.is_dir() {
                    collect_jsonl(&subagents, name, &mut files)?;
                }
            }
        }
    }
    files.sort_by(|a, b| {
        b.mtime_ns
            .cmp(&a.mtime_ns)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(files)
}

fn collect_jsonl(
    directory: &Path,
    session_id: &str,
    files: &mut Vec<ClaudeFile>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?.path();
        let name = entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if entry.is_dir() {
            collect_jsonl(&entry, session_id, files)?;
        } else if entry.is_file() && name.ends_with(".jsonl") {
            files.push(ClaudeFile {
                mtime_ns: mtime_ns(&entry)?,
                path: entry,
                session_id: session_id.to_owned(),
                sidechain: true,
            });
        }
    }
    Ok(())
}

fn mtime_ns(path: &Path) -> std::io::Result<u128> {
    Ok(fs::metadata(path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default())
}

#[derive(Debug)]
pub struct ClaudeRead {
    pub outcome: BlockOutcome,
    /// Bytes covered by complete lines; a trailing partial line is not consumed.
    pub complete_bytes: u64,
}

/// Lift one file into a session block. Only complete lines are parsed; the
/// header is synthesized from the file name and the records' own native fields.
pub fn read_file(file: &ClaudeFile) -> std::io::Result<ClaudeRead> {
    let bytes = fs::read(&file.path)?;
    let complete_bytes = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |index| index as u64 + 1);
    let session = file.session_id.clone();
    let Ok(text) = std::str::from_utf8(&bytes[..complete_bytes as usize]) else {
        return Ok(ClaudeRead {
            outcome: BlockOutcome::Malformed {
                native_session_id: Some(session),
                line: 0,
                reason: "transcript is not UTF-8",
            },
            complete_bytes,
        });
    };
    let context = SourceContext {
        conversation_id: Some(session.clone()),
        native_session_id: Some(session.clone()),
        source_platform: Some(Host::Claude.as_str().to_owned()),
        source_surface: None,
        started_at: None,
        source: Some(SessionSource::Transcript),
    };
    let mut records = Vec::new();
    let (mut inert, mut dropped) = (0, 0);
    let mut surface = None;
    let (mut cwd, mut git_branch) = (None, None);
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_with_context(line, &context) {
            Ok(Parsed::Record(record)) => {
                if surface.is_none() {
                    surface = record.native.entrypoint.clone();
                }
                if cwd.is_none() {
                    cwd = record.canonical.cwd.clone();
                }
                if git_branch.is_none() {
                    git_branch = record.canonical.git_branch.clone();
                }
                records.push(*record);
            }
            Ok(Parsed::Dropped(_)) => dropped += 1,
            Ok(Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_)) => inert += 1,
            Err(_) => {
                return Ok(ClaudeRead {
                    outcome: BlockOutcome::Malformed {
                        native_session_id: Some(session),
                        line: index + 1,
                        reason: "canonical record does not match the shared stream contract",
                    },
                    complete_bytes,
                });
            }
        }
    }
    let header = SessionHeader {
        kind: "session".into(),
        host: Host::Claude.as_str().into(),
        conversation_id: expected_conversation_id(Host::Claude, &session),
        native_session_id: session,
        source_surface: surface,
        started_at: None,
        cwd,
        git_branch,
        title: None,
        path: file.path.to_string_lossy().into_owned(),
        mtime: file.mtime_ns as f64 / 1_000_000_000.0,
    };
    Ok(ClaudeRead {
        outcome: BlockOutcome::Ready(Box::new(SessionBlock {
            header,
            records,
            inert,
            dropped,
        })),
        complete_bytes,
    })
}
