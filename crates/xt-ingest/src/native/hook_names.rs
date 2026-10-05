//! Transient, bounded names from indexed Claude stop summaries. Only approved
//! script basenames leave this module; command text is never retained by the
//! store or returned to the view.
use super::{
    readers_cli::CancelToken,
    session_source::{IndexedSource, identifier_error},
    session_titles::{self, Batch, Flow, Proof, TITLE_DEADLINE, TitleLimits},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use xt_store::{
    SessionSource, Store,
    batch::{LocatorRows, escape_like},
    timestamp,
};

pub const MAX_HOOK_SESSIONS: usize = 256;
/// The most saved summaries one request loads from the index. Any beyond it
/// stay in the requested count and are reported unavailable.
pub const MAX_HOOK_SUMMARIES: usize = 2048;
const MAX_FILES_PER_SESSION: usize = 64;
const LABELS: [(&str, &str); 6] = [
    ("turn_flush_prefilter.py", "Turn flush"),
    ("flush_turn.py", "Turn flush"),
    ("brain_brief.py", "Brain brief"),
    ("md_capture_flush.py", "Memory capture"),
    ("rulebook_hook.py", "Rulebook"),
    ("harness_stop.py", "Harness stop"),
];

#[derive(Clone, Debug)]
pub struct SummaryTarget {
    pub uuid: String,
    pub timestamp: String,
}

#[derive(Clone, Debug)]
pub struct SessionTarget {
    pub native_session_id: String,
    pub locators: Vec<IndexedSource>,
    pub summaries: Vec<SummaryTarget>,
}

/// One detail request's cancel and its single deadline, started before the
/// index is read so that preparation and source reading share it. Both are
/// cooperative: they are heard between bounded steps, so they cannot
/// interrupt one SQLite statement or one file read already in progress.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub cancel: Option<&'a CancelToken>,
    pub deadline: Instant,
}

impl<'a> Request<'a> {
    pub fn start(cancel: Option<&'a CancelToken>) -> Self {
        Self {
            cancel,
            deadline: Instant::now() + TITLE_DEADLINE,
        }
    }

    pub fn stopped(&self) -> bool {
        self.cancel.is_some_and(CancelToken::is_cancelled) || Instant::now() >= self.deadline
    }
}

/// The sessions one request may read, and how many saved summaries the
/// selected window holds in total, including those not prepared.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    pub requested_summaries: usize,
    pub sessions: Vec<SessionTarget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    pub requested_summaries: usize,
    pub checked_summaries: usize,
    pub unavailable_summaries: usize,
    pub summaries_with_unnamed_commands: usize,
    pub labels: Vec<NameCount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameCount {
    pub script_basename: &'static str,
    pub display_label: &'static str,
    pub summaries_mentioning: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Found {
    labels: BTreeSet<&'static str>,
    unnamed: bool,
    /// Ephemeral proof that two copies have the same complete native record.
    /// The raw line and its commands never leave the streaming callback.
    fingerprint: [u8; 32],
}

/// This detail's capped form of the recorded Claude locator lookup. A
/// saturated result is unavailable; it is never cut to a convenient prefix.
pub fn indexed_hook_sources(store: &Store, native: &str) -> xt_store::Result<Vec<IndexedSource>> {
    if identifier_error(native).is_some() {
        return Ok(Vec::new());
    }
    let escaped = escape_like(native);
    let patterns = [
        format!("claude:%/{escaped}.jsonl"),
        format!("claude:%/{escaped}/subagents/%"),
    ];
    let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
    let LocatorRows::Complete(rows) =
        store.source_cursors_like_bounded(SessionSource::Transcript, &patterns)?
    else {
        return Ok(Vec::new());
    };
    if rows.len() > MAX_FILES_PER_SESSION {
        return Ok(Vec::new());
    }
    rows.into_iter()
        .map(|cursor| {
            Ok(IndexedSource {
                checkpoint: store
                    .native_checkpoint(SessionSource::Transcript, &cursor.cursor_key)?,
                locator: cursor.cursor_key,
            })
        })
        .collect()
}

/// Load at most [`MAX_HOOK_SUMMARIES`] saved summaries of at most
/// [`MAX_HOOK_SESSIONS`] sessions, then their recorded locators, all in one
/// read snapshot. Nothing is read once the request has stopped: `None` before
/// the index was counted, and no prepared session once it was.
pub fn prepare(
    store: &Store,
    start: &str,
    end: &str,
    request: &Request<'_>,
) -> xt_store::Result<Option<Prepared>> {
    prepare_until(store, start, end, &mut || request.stopped())
}

fn prepare_until(
    store: &Store,
    start: &str,
    end: &str,
    stopped: &mut dyn FnMut() -> bool,
) -> xt_store::Result<Option<Prepared>> {
    if stopped() {
        return Ok(None);
    }
    store.in_read_snapshot(|store| {
        let (requested_summaries, rows) =
            store.claude_hook_summaries(start, end, MAX_HOOK_SUMMARIES)?;
        let mut sessions = Vec::<SessionTarget>::new();
        let mut current = None::<String>;
        // Rows arrive in session order, so each session is one run of rows.
        for (event, native) in rows {
            if current.as_deref() != Some(event.session_id.as_str()) {
                if sessions.len() == MAX_HOOK_SESSIONS {
                    break;
                }
                current = Some(event.session_id);
                sessions.push(SessionTarget {
                    native_session_id: native.unwrap_or_default(),
                    locators: Vec::new(),
                    summaries: Vec::new(),
                });
            }
            if let (Some(target), Some(timestamp)) = (sessions.last_mut(), event.timestamp) {
                target.summaries.push(SummaryTarget {
                    uuid: event.source_event_id,
                    timestamp,
                });
            }
        }
        for target in &mut sessions {
            if stopped() {
                return Ok(Some(Prepared {
                    requested_summaries,
                    sessions: Vec::new(),
                }));
            }
            if !target.native_session_id.is_empty() {
                target.locators = indexed_hook_sources(store, &target.native_session_id)?;
            }
        }
        Ok(Some(Prepared {
            requested_summaries,
            sessions,
        }))
    })
}

/// The caller supplies only rows selected from the index. A source can be
/// checked at most once and a failed source contributes no partially read names.
pub fn read(home: &Path, prepared: &Prepared, request: &Request<'_>) -> Names {
    read_with_limits(home, prepared, request, TitleLimits::default())
}

/// `limits.deadline` is replaced by what remains of the request's deadline.
fn read_with_limits(
    home: &Path,
    prepared: &Prepared,
    request: &Request<'_>,
    limits: TitleLimits,
) -> Names {
    let cancel = request.cancel;
    let mut answer = Names {
        requested_summaries: prepared.requested_summaries,
        ..Names::default()
    };
    let mut counts = BTreeMap::<&'static str, usize>::new();
    let limits = TitleLimits {
        deadline: request.deadline.saturating_duration_since(Instant::now()),
        ..limits
    };
    let mut batch = Batch::new(limits, cancel);
    for (position, target) in prepared.sessions.iter().enumerate() {
        if position >= MAX_HOOK_SESSIONS || batch.stopped().is_some() {
            break;
        }
        let Some(found) = read_session(home, target, &mut batch) else {
            continue;
        };
        for summary in &target.summaries {
            if let Some(item) = found.get(&summary.uuid).and_then(|item| item.as_ref()) {
                answer.checked_summaries += 1;
                answer.summaries_with_unnamed_commands += usize::from(item.unnamed);
                for name in &item.labels {
                    *counts.entry(name).or_default() += 1;
                }
            }
        }
    }
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Names {
            requested_summaries: answer.requested_summaries,
            unavailable_summaries: answer.requested_summaries,
            ..Names::default()
        };
    }
    answer.unavailable_summaries = answer
        .requested_summaries
        .saturating_sub(answer.checked_summaries);
    answer.labels = LABELS
        .iter()
        .filter_map(|(basename, label)| {
            let count = counts.get(basename).copied()?;
            Some(NameCount {
                script_basename: basename,
                display_label: label,
                summaries_mentioning: count,
            })
        })
        .collect();
    answer
}

fn read_session(
    home: &Path,
    target: &SessionTarget,
    batch: &mut Batch<'_>,
) -> Option<BTreeMap<String, Option<Found>>> {
    if target.locators.is_empty() || target.locators.len() > MAX_FILES_PER_SESSION {
        return None;
    }
    let roots = [home.join(".claude/projects")];
    let native = target.native_session_id.as_str();
    let (files, outside) = session_titles::gather(
        target
            .locators
            .iter()
            .filter_map(|source| Some((source.claude_path()?, source))),
        native,
        &roots,
        |path, roots| contained(path, roots, native),
    )
    .ok()?;
    // A missing recorded file might carry a conflicting copy of a UUID.
    // Even two locators deduplicated to one file are conservatively unavailable.
    if outside || files.len() != target.locators.len() {
        return None;
    }
    let primary = format!("{native}.jsonl");
    if files
        .iter()
        .filter(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name == primary.as_str())
        })
        .count()
        > 1
    {
        return None;
    }
    let wanted: BTreeMap<&str, &str> = target
        .summaries
        .iter()
        .map(|item| (item.uuid.as_str(), item.timestamp.as_str()))
        .collect();
    let mut found = BTreeMap::<String, Option<Found>>::new();
    for file in files {
        if batch.stopped().is_some() {
            return None;
        }
        let mut in_file = BTreeMap::<String, Option<Found>>::new();
        batch
            .stream(&file, Proof::Checkpoint, &mut |line| {
                // The exact JSON fields below decide acceptance. This byte check
                // only saves parsing unrelated records; it is never a name search.
                if !line
                    .windows(b"stop_hook_summary".len())
                    .any(|part| part == b"stop_hook_summary")
                {
                    return Flow::Continue;
                }
                let Some((uuid, detail)) = summary(line, native, &wanted) else {
                    return Flow::Continue;
                };
                merge(&mut in_file, uuid, detail);
                Flow::Continue
            })
            .ok()?;
        for (uuid, detail) in in_file {
            merge(&mut found, uuid, detail);
        }
    }
    Some(found)
}

fn merge(found: &mut BTreeMap<String, Option<Found>>, uuid: String, detail: Option<Found>) {
    match found.entry(uuid) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(detail);
        }
        std::collections::btree_map::Entry::Occupied(mut slot) => {
            if slot.get() != &detail {
                slot.insert(None);
            }
        }
    }
}

fn contained(path: &Path, roots: &[PathBuf], native: &str) -> bool {
    if session_titles::claude_contained(path, roots, native) {
        return true;
    }
    let Some(subagents) = path.parent() else {
        return false;
    };
    let Some(session) = subagents.parent() else {
        return false;
    };
    let Some(project) = session.parent() else {
        return false;
    };
    let ordinary = |p: &Path| {
        fs::symlink_metadata(p).is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
    };
    path.is_absolute()
        && path.components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        && path.extension().is_some_and(|value| value == "jsonl")
        && subagents
            .file_name()
            .is_some_and(|value| value == "subagents")
        && session.file_name().is_some_and(|value| value == native)
        && project
            .parent()
            .is_some_and(|root| roots.iter().any(|allowed| allowed == root))
        && ordinary(subagents)
        && ordinary(session)
        && ordinary(project)
}

fn summary(
    line: &[u8],
    native: &str,
    wanted: &BTreeMap<&str, &str>,
) -> Option<(String, Option<Found>)> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let object = value.as_object()?;
    if object.get("type")?.as_str()? != "system"
        || object.get("subtype")?.as_str()? != "stop_hook_summary"
    {
        return None;
    }
    let uuid = object.get("uuid")?.as_str()?;
    let saved = *wanted.get(uuid)?;
    let time = object.get("timestamp").and_then(Value::as_str);
    let identity_agrees = object.get("sessionId").and_then(Value::as_str) == Some(native)
        && time
            .and_then(|actual| timestamp::parse(actual).ok())
            .zip(timestamp::parse(saved).ok())
            .is_some_and(|(actual, saved)| actual.0 == saved.0);
    if !identity_agrees {
        return Some((uuid.to_owned(), None));
    }
    let mut labels = BTreeSet::new();
    let mut unnamed = false;
    if let Some(infos) = object.get("hookInfos").and_then(Value::as_array) {
        unnamed |= infos.is_empty()
            || object
                .get("hookCount")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > infos.len() as u64);
        for info in infos {
            let known = info
                .get("command")
                .and_then(Value::as_str)
                .and_then(script_names);
            match known {
                Some(names) => {
                    labels.extend(names);
                }
                None => unnamed = true,
            }
        }
    } else {
        unnamed = true;
    }
    Some((
        uuid.to_owned(),
        Some(Found {
            labels,
            unnamed,
            fingerprint: Sha256::digest(line).into(),
        }),
    ))
}

/// A deliberately small command grammar. A filename found in an argument,
/// echo, variable, shell expansion or unrelated text is not evidence.
fn script_names(command: &str) -> Option<BTreeSet<&'static str>> {
    if let Some(names) = guarded_stop_template(command) {
        return Some(names);
    }
    let command = command.trim();
    let command = command
        .strip_prefix("bash -lc '")
        .and_then(|inner| inner.strip_suffix('\''))
        .or_else(|| {
            command
                .strip_prefix("sh -c '")
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(command);
    let command = command.strip_prefix("exec ").unwrap_or(command);
    let command = ["python3 ", "python ", "uv run python3 ", "uv run python "]
        .into_iter()
        .find_map(|prefix| command.strip_prefix(prefix))?;
    let command = command.strip_prefix("-u ").unwrap_or(command);
    // The script and every argument after it must be plain words separated by
    // spaces. Any operator, quote, expansion, redirect, comment or line break
    // could start another command, so the whole command stays unnamed.
    if command
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || b"/._-=,:@+ ".contains(&byte)))
    {
        return None;
    }
    let script = command.split(' ').next()?;
    if !script
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return None;
    }
    let basename = script.rsplit('/').next()?;
    LABELS
        .iter()
        .find_map(|(known, _)| (*known == basename).then(|| BTreeSet::from([*known])))
}

/// The five Stop command shapes in the static MemHub hook template. Each
/// delimiter, guard and command position must agree; variables are never
/// expanded and a filename in an echo or argument cannot enter this result.
fn guarded_stop_template(command: &str) -> Option<BTreeSet<&'static str>> {
    let body = command.strip_prefix("IN=$(cat); ")?;
    let (harness, body) = match body.strip_prefix(
        "case \"${MEMHUB_HARNESS_EXTRACT:-}\" in 1|[Oo][Nn]|[Tt][Rr][Uu][Ee]|[Yy][Ee][Ss]) ;; *) exit 0 ;; esac; "
    ) { Some(body) => (true, body), None => (false, body) };
    let body = body.strip_prefix(
        "if [ -n \"${CLAUDE_PLUGIN_ROOT:-}\" ] && printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/claude_hook_guard.py\" "
    )?;
    let (prefilter, body) = if let Some(body) = body.strip_prefix(
        "capture Stop && printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/turn_flush_prefilter.py\"; then printf %s \"$IN\" | "
    ) { (true, body) } else { (false, body.strip_prefix(
        "ignore Stop; then printf %s \"$IN\" | "
    )?) };
    let main = if prefilter {
        (body == "python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/flush_turn.py\"; fi")
            .then_some("flush_turn.py")
    } else {
        [
            ("python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/brain_brief.py\" refresh; fi", "brain_brief.py"),
            ("uv run --with 'mcp<2' python \"${CLAUDE_PLUGIN_ROOT}/scripts/md_capture_flush.py\"; fi", "md_capture_flush.py"),
            ("python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/rulebook_hook.py\" flush; fi", "rulebook_hook.py"),
            ("python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/harness_stop.py\" stop; fi", "harness_stop.py"),
        ].into_iter().find_map(|(shape, basename)| (body == shape).then_some(basename))
    }?;
    if harness != (main == "harness_stop.py") {
        return None;
    }
    let mut names = BTreeSet::from([main]);
    if prefilter {
        names.insert("turn_flush_prefilter.py");
    }
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xt_store::{
        SessionMeta,
        batch::SourceCursor,
        ingest::{ToolEvent, ToolKind},
    };
    const SESSION: &str = "00000000-0000-4000-8000-00000000aaaa";
    const UUID: &str = "11111111-1111-4111-8111-111111111111";
    const TIME: &str = "2026-09-07T12:00:00Z";

    fn fixture(lines: &[String]) -> (tempfile::TempDir, SessionTarget) {
        let root = tempfile::TempDir::new().unwrap();
        let project = root.path().join(".claude/projects/-fixture");
        fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{SESSION}.jsonl"));
        fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        let target = SessionTarget {
            native_session_id: SESSION.into(),
            locators: vec![IndexedSource {
                locator: format!("claude:{}", path.display()),
                checkpoint: None,
            }],
            summaries: vec![SummaryTarget {
                uuid: UUID.into(),
                timestamp: TIME.into(),
            }],
        };
        (root, target)
    }
    fn prepared(targets: &[SessionTarget]) -> Prepared {
        Prepared {
            requested_summaries: targets.iter().map(|target| target.summaries.len()).sum(),
            sessions: targets.to_vec(),
        }
    }
    fn check(home: &Path, targets: &[SessionTarget], cancel: Option<&CancelToken>) -> Names {
        read(home, &prepared(targets), &Request::start(cancel))
    }
    fn check_with_limits(home: &Path, targets: &[SessionTarget], limits: TitleLimits) -> Names {
        read_with_limits(home, &prepared(targets), &Request::start(None), limits)
    }
    fn line(uuid: &str, session: &str, commands: &[&str]) -> String {
        json!({"type":"system","subtype":"stop_hook_summary","uuid":uuid,
            "timestamp":TIME,"sessionId":session,"hookCount":commands.len(),
            "hookInfos":commands.iter().map(|command| json!({"command":command})).collect::<Vec<_>>()}).to_string()
    }

    #[test]
    fn command_shapes_are_conservative() {
        assert_eq!(
            script_names("python3 /safe/brain_brief.py --arg secret"),
            Some(BTreeSet::from(["brain_brief.py"]))
        );
        assert_eq!(
            script_names("bash -lc 'python3 /safe/rulebook_hook.py --arg secret'"),
            Some(BTreeSet::from(["rulebook_hook.py"]))
        );
        for command in [
            "echo brain_brief.py",
            "python3 $ROOT/brain_brief.py",
            "bash -lc 'echo brain_brief.py'",
            "python3 /safe/unknown.py",
            "python3 -c 'brain_brief.py'",
            "python3 /safe/brain_brief.py; echo x",
            "python3 /safe/brain_brief.py ; python3 /safe/unknown.py",
            "python3 /safe/brain_brief.py && python3 /safe/unknown.py",
            "python3 /safe/brain_brief.py || python3 /safe/unknown.py",
            "python3 /safe/brain_brief.py | python3 /safe/unknown.py",
            "python3 /safe/brain_brief.py & python3 /safe/unknown.py",
            "python3 /safe/brain_brief.py\npython3 /safe/unknown.py",
            "python3 /safe/brain_brief.py\tpython3",
            "python3 /safe/brain_brief.py $(python3 /safe/unknown.py)",
            "python3 /safe/brain_brief.py `python3 /safe/unknown.py`",
            "python3 /safe/brain_brief.py --arg $SECRET",
            "python3 /safe/brain_brief.py > /tmp/out",
            "python3 /safe/brain_brief.py # python3 /safe/unknown.py",
            "bash -lc 'python3 /safe/brain_brief.py ; python3 /safe/unknown.py'",
            "sh -c 'python3 /safe/brain_brief.py && python3 /safe/unknown.py'",
        ] {
            assert_eq!(script_names(command), None, "{command}");
        }
        for (command, basename) in [
            (
                "sh -c 'exec python3 -u /safe/harness_stop.py stop'",
                "harness_stop.py",
            ),
            (
                "uv run python3 /safe/md_capture_flush.py --mode=flush",
                "md_capture_flush.py",
            ),
            (
                "exec python /safe/flush_turn.py --turn 3,4",
                "flush_turn.py",
            ),
        ] {
            assert_eq!(
                script_names(command),
                Some(BTreeSet::from([basename])),
                "{command}"
            );
        }
    }

    #[test]
    fn a_known_script_followed_by_another_command_leaves_the_summary_unnamed() {
        let (root, target) = fixture(&[line(
            UUID,
            SESSION,
            &["python3 /safe/brain_brief.py ; python3 /safe/unknown.py"],
        )]);
        let answer = check(root.path(), &[target], None);
        assert_eq!(
            (
                answer.checked_summaries,
                answer.summaries_with_unnamed_commands
            ),
            (1, 1)
        );
        assert!(answer.labels.is_empty());
    }

    const IN_WINDOW: (&str, &str) = ("2026-09-01T00:00:00Z", "2026-09-08T00:00:00Z");

    /// `sessions` Claude sessions of `each` in-window summaries, every one
    /// with one recorded primary locator.
    fn indexed(sessions: usize, each: usize) -> Store {
        let mut store = Store::open_in_memory().unwrap();
        for session in 0..sessions {
            let id = format!("00000000-0000-4000-8000-{session:012}");
            let mut meta = SessionMeta::new(&id, "claude", SessionSource::Transcript);
            meta.native_session_id = Some(id.clone());
            store.upsert_session(&meta, false).unwrap();
            store
                .record_native_source_locator(
                    &SourceCursor {
                        source: SessionSource::Transcript,
                        cursor_key: format!("claude:/tmp/-p/{id}.jsonl"),
                        position: 0,
                        updated_at: 1,
                    },
                    true,
                )
                .unwrap();
            for summary in 0..each {
                store
                    .insert_tool_event(&ToolEvent {
                        session_id: id.clone(),
                        source: SessionSource::Transcript,
                        source_event_id: format!("11111111-1111-4111-{session:04}-{summary:012}"),
                        timestamp: Some(TIME.into()),
                        name: "stop_hook_summary".into(),
                        kind: ToolKind::Hook,
                        server: None,
                        tool: None,
                        skill: None,
                    })
                    .unwrap();
            }
        }
        store
    }

    #[test]
    fn preparation_loads_at_most_the_session_cap_and_keeps_the_true_count() {
        let store = indexed(MAX_HOOK_SESSIONS + 2, 1);
        let (start, end) = IN_WINDOW;
        let prepared = prepare(&store, start, end, &Request::start(None))
            .unwrap()
            .unwrap();
        assert_eq!(prepared.requested_summaries, MAX_HOOK_SESSIONS + 2);
        assert_eq!(prepared.sessions.len(), MAX_HOOK_SESSIONS);
        assert!(
            prepared
                .sessions
                .iter()
                .all(|target| target.locators.len() == 1)
        );
        assert_eq!(
            prepared.sessions.last().unwrap().native_session_id,
            format!("00000000-0000-4000-8000-{:012}", MAX_HOOK_SESSIONS - 1)
        );
        // Nothing in the fixture home exists, so every summary is unavailable,
        // including the two never prepared.
        let root = tempfile::TempDir::new().unwrap();
        let answer = read(root.path(), &prepared, &Request::start(None));
        assert_eq!(
            (
                answer.requested_summaries,
                answer.checked_summaries,
                answer.unavailable_summaries
            ),
            (MAX_HOOK_SESSIONS + 2, 0, MAX_HOOK_SESSIONS + 2)
        );
    }

    #[test]
    fn preparation_loads_at_most_the_summary_cap() {
        let store = indexed(1, MAX_HOOK_SUMMARIES + 3);
        let (start, end) = IN_WINDOW;
        let prepared = prepare(&store, start, end, &Request::start(None))
            .unwrap()
            .unwrap();
        assert_eq!(prepared.requested_summaries, MAX_HOOK_SUMMARIES + 3);
        assert_eq!(prepared.sessions.len(), 1);
        assert_eq!(prepared.sessions[0].summaries.len(), MAX_HOOK_SUMMARIES);
    }

    #[test]
    fn summaries_left_out_of_preparation_are_unavailable() {
        let (root, target) = fixture(&[line(UUID, SESSION, &["python3 /safe/brain_brief.py"])]);
        let prepared = Prepared {
            requested_summaries: 3,
            sessions: vec![target],
        };
        let answer = read(root.path(), &prepared, &Request::start(None));
        assert_eq!(
            (
                answer.requested_summaries,
                answer.checked_summaries,
                answer.unavailable_summaries
            ),
            (3, 1, 2)
        );
        assert_eq!(answer.labels[0].summaries_mentioning, 1);
    }

    #[test]
    fn a_stopped_request_prepares_nothing_more() {
        let store = indexed(5, 1);
        let (start, end) = IN_WINDOW;
        let token = CancelToken::new();
        token.cancel();
        assert!(
            prepare(&store, start, end, &Request::start(Some(&token)))
                .unwrap()
                .is_none()
        );
        let over = Request {
            cancel: None,
            deadline: Instant::now(),
        };
        assert!(prepare(&store, start, end, &over).unwrap().is_none());
        // A cancel heard before the second locator lookup ends preparation
        // there: no later session is looked up and none is returned.
        let mut checks = 0;
        let prepared = prepare_until(&store, start, end, &mut || {
            checks += 1;
            checks == 3
        })
        .unwrap()
        .unwrap();
        assert_eq!(checks, 3);
        assert_eq!(prepared.requested_summaries, 5);
        assert!(prepared.sessions.is_empty());
    }

    #[test]
    fn recorded_locator_lookup_refuses_more_than_the_file_cap() {
        let mut store = Store::open_in_memory().unwrap();
        for index in 0..=MAX_FILES_PER_SESSION {
            store
                .record_native_source_locator(
                    &SourceCursor {
                        source: SessionSource::Transcript,
                        cursor_key: format!(
                            "claude:/tmp/{SESSION}/subagents/agent-{index:03}.jsonl"
                        ),
                        position: 0,
                        updated_at: 1,
                    },
                    true,
                )
                .unwrap();
        }
        assert!(indexed_hook_sources(&store, SESSION).unwrap().is_empty());
    }

    #[test]
    fn guarded_stop_shapes_name_only_explicit_script_positions() {
        let first = "IN=$(cat); if [ -n \"${CLAUDE_PLUGIN_ROOT:-}\" ] && printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/claude_hook_guard.py\" ";
        let cases = [
            (
                "capture Stop && printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/turn_flush_prefilter.py\"; then printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/flush_turn.py\"; fi",
                BTreeSet::from(["turn_flush_prefilter.py", "flush_turn.py"]),
            ),
            (
                "ignore Stop; then printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/brain_brief.py\" refresh; fi",
                BTreeSet::from(["brain_brief.py"]),
            ),
            (
                "ignore Stop; then printf %s \"$IN\" | uv run --with 'mcp<2' python \"${CLAUDE_PLUGIN_ROOT}/scripts/md_capture_flush.py\"; fi",
                BTreeSet::from(["md_capture_flush.py"]),
            ),
            (
                "ignore Stop; then printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/rulebook_hook.py\" flush; fi",
                BTreeSet::from(["rulebook_hook.py"]),
            ),
        ];
        for (suffix, expected) in cases {
            assert_eq!(script_names(&(first.to_owned() + suffix)), Some(expected));
        }
        let harness = "IN=$(cat); case \"${MEMHUB_HARNESS_EXTRACT:-}\" in 1|[Oo][Nn]|[Tt][Rr][Uu][Ee]|[Yy][Ee][Ss]) ;; *) exit 0 ;; esac; ";
        let harness = format!(
            "{harness}{}{}",
            first.strip_prefix("IN=$(cat); ").unwrap(),
            "ignore Stop; then printf %s \"$IN\" | python3 \"${CLAUDE_PLUGIN_ROOT}/scripts/harness_stop.py\" stop; fi"
        );
        assert_eq!(
            script_names(&harness),
            Some(BTreeSet::from(["harness_stop.py"]))
        );
        assert_eq!(script_names(&(harness + " echo brain_brief.py")), None);
    }

    #[test]
    fn matches_only_saved_uuid_session_and_time_and_deduplicates_labels() {
        let (root, target) = fixture(&[
            line("other", SESSION, &["python3 /safe/harness_stop.py"]),
            line(
                UUID,
                SESSION,
                &[
                    "python3 /safe/brain_brief.py --secret x",
                    "python3 /safe/brain_brief.py --secret y",
                    "echo unknown",
                ],
            ),
        ]);
        let answer = check(root.path(), &[target], None);
        assert_eq!(
            (
                answer.requested_summaries,
                answer.checked_summaries,
                answer.unavailable_summaries,
                answer.summaries_with_unnamed_commands
            ),
            (1, 1, 0, 1)
        );
        assert_eq!(answer.labels.len(), 1);
        assert_eq!(answer.labels[0].script_basename, "brain_brief.py");
        assert_eq!(answer.labels[0].summaries_mentioning, 1);
        let wire = serde_json::to_string(
            &answer
                .labels
                .iter()
                .map(|name| name.display_label)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(!wire.contains("secret"));
    }

    #[test]
    fn copied_uuid_or_conflicting_duplicate_is_unavailable() {
        let (root, target) = fixture(&[line(
            UUID,
            "copied-session",
            &["python3 /safe/brain_brief.py"],
        )]);
        let answer = check(root.path(), &[target], None);
        assert_eq!(
            (answer.checked_summaries, answer.unavailable_summaries),
            (0, 1)
        );
        let (root, target) = fixture(&[
            line(UUID, SESSION, &["python3 /safe/brain_brief.py"]),
            line(UUID, SESSION, &["python3 /safe/harness_stop.py"]),
        ]);
        assert_eq!(check(root.path(), &[target], None).unavailable_summaries, 1);
        let (root, target) = fixture(&[
            line(UUID, SESSION, &["python3 /safe/brain_brief.py --one"]),
            line(UUID, SESSION, &["python3 /safe/brain_brief.py --two"]),
        ]);
        assert_eq!(check(root.path(), &[target], None).unavailable_summaries, 1);
    }

    #[test]
    fn two_recorded_primary_files_do_not_claim_one_session() {
        let (root, mut target) = fixture(&[line(UUID, SESSION, &["python3 /safe/brain_brief.py"])]);
        let other_project = root.path().join(".claude/projects/-other");
        fs::create_dir_all(&other_project).unwrap();
        let other = other_project.join(format!("{SESSION}.jsonl"));
        fs::write(
            &other,
            line(UUID, SESSION, &["python3 /safe/brain_brief.py"]) + "\n",
        )
        .unwrap();
        target.locators.push(IndexedSource {
            locator: format!("claude:{}", other.display()),
            checkpoint: None,
        });
        let answer = check(root.path(), &[target], None);
        assert_eq!(
            (answer.checked_summaries, answer.unavailable_summaries),
            (0, 1)
        );
    }

    #[test]
    fn missing_changed_over_limit_and_cancelled_sources_give_no_partial_names() {
        let (root, mut target) = fixture(&[line(UUID, SESSION, &["python3 /safe/brain_brief.py"])]);
        let path = PathBuf::from(target.locators[0].locator.strip_prefix("claude:").unwrap());
        let missing = IndexedSource {
            locator: format!("claude:{}", path.with_file_name("missing.jsonl").display()),
            checkpoint: None,
        };
        let mut incomplete = target.clone();
        incomplete.locators.push(missing);
        assert_eq!(
            check(root.path(), &[incomplete], None).unavailable_summaries,
            1
        );
        let limits = TitleLimits {
            max_file_bytes: 1,
            ..TitleLimits::default()
        };
        assert_eq!(
            check_with_limits(root.path(), &[target.clone()], limits).unavailable_summaries,
            1
        );
        let limits = TitleLimits {
            max_line_bytes: 1,
            ..TitleLimits::default()
        };
        assert_eq!(
            check_with_limits(root.path(), &[target.clone()], limits).unavailable_summaries,
            1
        );
        let token = CancelToken::new();
        token.cancel();
        assert_eq!(
            check(root.path(), &[target.clone()], Some(&token)).unavailable_summaries,
            1
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            check(root.path(), &[target.clone()], None).unavailable_summaries,
            1
        );
        fs::write(
            &path,
            line(UUID, SESSION, &["python3 /safe/brain_brief.py"]) + "\n",
        )
        .unwrap();
        target.summaries[0].timestamp = "2026-09-07T12:00:01Z".into();
        assert_eq!(
            check(root.path(), &[target.clone()], None).unavailable_summaries,
            1
        );
        let over = Request {
            cancel: None,
            deadline: Instant::now(),
        };
        assert_eq!(
            read(root.path(), &prepared(&[target]), &over).unavailable_summaries,
            1
        );
    }

    #[test]
    fn absent_command_details_are_checked_but_unnamed() {
        let row = json!({"type":"system","subtype":"stop_hook_summary","uuid":UUID,
            "timestamp":TIME,"sessionId":SESSION})
        .to_string();
        let (root, target) = fixture(&[row]);
        let answer = check(root.path(), &[target], None);
        assert_eq!(
            (
                answer.checked_summaries,
                answer.summaries_with_unnamed_commands
            ),
            (1, 1)
        );
        assert!(answer.labels.is_empty());
    }
}
