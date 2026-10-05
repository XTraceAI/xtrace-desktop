//! The Environment detail read uses indexed UUIDs and recorded locators only.
#![cfg(unix)]
use serde_json::json;
use std::{fs, path::Path};
use xt_ingest::native::readers_cli::CancelToken;
use xt_store::{
    SessionMeta, SessionSource, Store,
    batch::SourceCursor,
    ingest::{ToolEvent, ToolKind},
};
use xtrace_desktop::state::{AppState, StartupOptions};

const SESSION: &str = "00000000-0000-4000-8000-00000000aaaa";
const UUID: &str = "11111111-1111-4111-8111-111111111111";
const TIME: &str = "2026-09-07T12:00:00Z";

#[test]
fn detail_uses_the_displayed_window_and_never_returns_command_text() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = home.join(".claude/projects/-fixture");
    fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{SESSION}.jsonl"));
    fs::write(
        &path,
        format!(
            "{}\n",
            json!({
                "type":"system", "subtype":"stop_hook_summary", "uuid":UUID,
                "timestamp":TIME, "sessionId":SESSION, "hookCount":2,
                "hookInfos":[{"command":"python3 /private/brain_brief.py --secret private"},
                             {"command":"echo private"}]
            })
        ),
    )
    .unwrap();
    let db = root.path().join("data/xtrace.db");
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    let mut store = Store::open(&db).unwrap();
    let mut meta = SessionMeta::new(SESSION, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(SESSION.into());
    store.upsert_session(&meta, false).unwrap();
    store
        .record_native_source_locator(
            &SourceCursor {
                source: SessionSource::Transcript,
                cursor_key: format!("claude:{}", path.display()),
                position: 0,
                updated_at: 1,
            },
            true,
        )
        .unwrap();
    store
        .insert_tool_event(&ToolEvent {
            session_id: SESSION.into(),
            source: SessionSource::Transcript,
            source_event_id: UUID.into(),
            timestamp: Some(TIME.into()),
            name: "stop_hook_summary".into(),
            kind: ToolKind::Hook,
            server: None,
            tool: None,
            skill: None,
        })
        .unwrap();
    drop(store);
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().join("data")),
            native_home: Some(home),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap();
    let end: jiff::Timestamp = "2026-09-08T00:00:00Z".parse().unwrap();
    let result = state
        .environment_hook_names(7, end.as_millisecond(), &CancelToken::new())
        .unwrap();
    assert_eq!(
        (
            result.requested_summaries,
            result.checked_summaries,
            result.unavailable_summaries,
            result.summaries_with_unnamed_commands
        ),
        (1, 1, 0, 1)
    );
    assert_eq!(result.labels[0].script_basename, "brain_brief.py");
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("private"));
    let later: jiff::Timestamp = "2026-09-20T00:00:00Z".parse().unwrap();
    let outside = state
        .environment_hook_names(7, later.as_millisecond(), &CancelToken::new())
        .unwrap();
    assert_eq!(outside.requested_summaries, 0);
    assert_eq!(outside.checked_summaries, 0);
    assert!(Path::new(&path).exists());
}
