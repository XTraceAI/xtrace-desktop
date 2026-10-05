//! Settings retention and purge commands over synthetic disposable databases
//! only; no test opens an installed app database or a real host history.
use serde_json::json;
use std::{cell::Cell, path::Path};
use xt_ingest::{
    canonical::{Parsed, ParsedRecord, SourceContext, parse_line},
    writer::{WriteBatch, write_batch},
};
use xt_store::{SessionSource, Store};
use xtrace_desktop::{
    privacy::ContentRetention,
    state::{AppState, StartupOptions, StateError},
};

fn app(root: &Path) -> AppState {
    std::fs::create_dir_all(root.join("home")).unwrap();
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            ..Default::default()
        },
        || panic!("the default data directory must not be resolved"),
        || Ok(root.join("home")),
    )
    .unwrap()
}

fn record(uuid: &str) -> ParsedRecord {
    let line = json!({"uuid":uuid,"type":"assistant","parentUuid":"parent","timestamp":"2026-09-10T00:00:00Z","message":{"role":"assistant","model":"synthetic-model","content":[{"type":"text","text":"Synthetic transcript"},{"type":"tool_use","name":"Read","input":{"path":"synthetic.txt"}}],"usage":{"input_tokens":7,"output_tokens":3}}});
    let Parsed::Record(record) = parse_line(&line.to_string()).unwrap() else {
        panic!("synthetic record");
    };
    *record
}

/// Writes one synthetic session through the canonical writer, requesting content.
fn import(db: &Path, uuid: &str) {
    let context = SourceContext {
        conversation_id: Some("synthetic".into()),
        source: Some(SessionSource::Transcript),
        ..Default::default()
    };
    let records = [record(uuid)];
    let batch = WriteBatch {
        namespace: None,
        context: &context,
        declared_host: None,
        records: &records,
        hook_summaries: &[],
        pr_witnesses: &[],
        title: Some("Synthetic title"),
        cwd: None,
        git_branch: None,
        keep_content: true,
        observed_at: 10,
        receipt: None,
        cursor: None,
        discovery: None,
        checkpoint: None,
    };
    write_batch(&mut Store::open(db).unwrap(), &batch).unwrap();
}

/// (title, message content, tool input) present per saved record.
fn content(db: &Path) -> (bool, Vec<(bool, bool)>) {
    let store = Store::open(db).unwrap();
    let title = store
        .session("synthetic")
        .unwrap()
        .unwrap()
        .meta
        .title
        .is_some();
    let records = store
        .records("synthetic")
        .unwrap()
        .into_iter()
        .map(|record| {
            (
                record.content_json.is_some(),
                record
                    .tool_uses
                    .iter()
                    .any(|tool| tool.input_json.is_some()),
            )
        })
        .collect();
    (title, records)
}

#[test]
fn retention_defaults_to_metadata_only_and_opt_in_persists() {
    let root = tempfile::TempDir::new().unwrap();
    let state = app(root.path());
    let db = root.path().join("data/xtrace.db");
    assert_eq!(
        state.content_retention().unwrap(),
        ContentRetention::MetadataOnly
    );
    // The default drops requested content from a fresh import.
    import(&db, "default");
    assert_eq!(content(&db), (false, vec![(false, false)]));
    assert_eq!(
        state
            .set_content_retention(ContentRetention::FullContent)
            .unwrap(),
        ContentRetention::FullContent
    );
    state.shutdown();
    let reopened = app(root.path());
    assert_eq!(
        reopened.content_retention().unwrap(),
        ContentRetention::FullContent
    );
}

#[test]
fn opting_out_changes_future_imports_and_keeps_saved_content() {
    let root = tempfile::TempDir::new().unwrap();
    let state = app(root.path());
    let db = root.path().join("data/xtrace.db");
    state
        .set_content_retention(ContentRetention::FullContent)
        .unwrap();
    import(&db, "archived");
    assert_eq!(content(&db), (true, vec![(true, true)]));
    assert_eq!(
        state
            .set_content_retention(ContentRetention::MetadataOnly)
            .unwrap(),
        ContentRetention::MetadataOnly
    );
    import(&db, "later");
    // The archived record keeps its content; the later one acquires none.
    let (title, mut records) = content(&db);
    records.sort();
    assert!(title);
    assert_eq!(records, vec![(false, false), (true, true)]);
}

#[test]
fn purge_clears_content_after_commit_and_preserves_everything_else() {
    let root = tempfile::TempDir::new().unwrap();
    let state = app(root.path());
    let db = root.path().join("data/xtrace.db");
    state
        .set_content_retention(ContentRetention::FullContent)
        .unwrap();
    import(&db, "archived");
    let host = root.path().join("home/synthetic-host.jsonl");
    let host_bytes = b"{\"synthetic\":\"original host file\"}\n";
    std::fs::write(&host, host_bytes).unwrap();
    let counts = serde_json::to_value(state.db_counts().unwrap()).unwrap();
    let before = Store::open(&db).unwrap().records("synthetic").unwrap();

    let published = Cell::new(0);
    let outcome = state
        .purge_stored_content(|| {
            // Another connection already sees the cleared fields: published after commit.
            assert_eq!(content(&db), (false, vec![(false, false)]));
            published.set(published.get() + 1);
        })
        .unwrap();
    assert_eq!(published.get(), 1);
    assert!(outcome.invalidate_content);
    assert_eq!(
        outcome.tables.iter().map(|table| table.rows).sum::<u64>(),
        3
    );

    let mut expected = before;
    for record in &mut expected {
        record.content_json = None;
        for tool in &mut record.tool_uses {
            tool.input_json = None;
        }
    }
    assert_eq!(
        Store::open(&db).unwrap().records("synthetic").unwrap(),
        expected
    );
    assert_eq!(
        serde_json::to_value(state.db_counts().unwrap()).unwrap(),
        counts
    );
    assert_eq!(std::fs::read(&host).unwrap(), host_bytes);
    // Purge never changes the mode.
    assert_eq!(
        state.content_retention().unwrap(),
        ContentRetention::FullContent
    );

    // Nothing left to clear: committed, but no invalidation.
    let again = state.purge_stored_content(|| published.set(99)).unwrap();
    assert!(!again.invalidate_content);
    assert_eq!(published.get(), 1);
}

#[test]
fn unavailable_database_fails_without_publishing() {
    let root = tempfile::TempDir::new().unwrap();
    let state = app(root.path());
    let db = root.path().join("data/xtrace.db");
    state
        .set_content_retention(ContentRetention::FullContent)
        .unwrap();
    import(&db, "archived");
    state.shutdown();
    let published = Cell::new(false);
    assert!(matches!(
        state.purge_stored_content(|| published.set(true)),
        Err(StateError::Closed)
    ));
    assert!(matches!(
        state.set_content_retention(ContentRetention::MetadataOnly),
        Err(StateError::Closed)
    ));
    assert!(!published.get());
    assert_eq!(content(&db), (true, vec![(true, true)]));
    assert_eq!(
        Store::open(&db).unwrap().retention_mode().unwrap(),
        xt_store::retention::RetentionMode::FullContent
    );
}
