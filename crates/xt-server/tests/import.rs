use rusqlite::Connection;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::broadcast,
};
use xt_ingest::writer::ChangeEvent;
use xt_store::{Store, retention::RetentionMode};

struct Server {
    port: u16,
    path: PathBuf,
    events: broadcast::Receiver<ChangeEvent>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    _dir: Arc<tempfile::TempDir>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new() -> Self {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let path = dir.path().join("synthetic.db");
        let store = Store::open(&path).unwrap();
        let listener = xt_server::bind((Ipv4Addr::LOCALHOST, 0).into())
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, events) = broadcast::channel(64);
        let keep = dir.clone();
        let owned_path = path.clone();
        let task = tokio::spawn(async move {
            let _keep = keep;
            xt_server::serve_with_events(
                listener,
                store,
                owned_path,
                Some(sender),
                std::future::pending(),
            )
            .await
        });
        Self {
            port,
            path,
            events,
            task,
            _dir: dir,
        }
    }
    fn sql(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }
    fn store(&self) -> Store {
        Store::open(&self.path).unwrap()
    }
    async fn raw(&self, body: &str) -> Value {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port))
            .await
            .unwrap();
        let headers = format!(
            "POST /mcp-server/mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            self.port,
            body.len()
        );
        stream.write_all(headers.as_bytes()).await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let response = String::from_utf8(bytes).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str(
            response
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap(),
        )
        .unwrap()
    }
    async fn import(&self, args: Value) -> Value {
        self.raw(&json!({"jsonrpc":"2.0","id":"import","method":"tools/call","params":{"name":"import_conversation","arguments":args}}).to_string()).await
    }
    async fn changed(&mut self) -> ChangeEvent {
        tokio::time::timeout(Duration::from_secs(1), self.events.recv())
            .await
            .unwrap()
            .unwrap()
    }
}
fn poor(uuid: &str) -> Value {
    json!({"uuid":uuid,"type":"assistant","message":{}})
}
fn rich(uuid: &str) -> Value {
    json!({"uuid":uuid,"type":"assistant","timestamp":"2026-09-10T01:02:03.123456789012Z","parentUuid":"parent","message":{"role":"assistant","model":"synthetic-model","content":[{"type":"text","text":"Synthetic text"},{"type":"tool_use","name":"Read","input":{"path":"synthetic.txt"}}],"usage":{"input_tokens":7,"output_tokens":3,"cache_read_input_tokens":0}}})
}
fn args(id: &str, rows: Vec<Value>) -> Value {
    json!({"conversation_id":id,"messages":rows,"source_platform":"claude","flush":"auto"})
}
fn success(response: &Value) -> &Value {
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    let output = &response["result"]["structuredContent"];
    assert_eq!(
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        *output
    );
    output
}
fn failure(response: &Value) {
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert!(response["result"].get("structuredContent").is_none());
}
fn count(sql: &Connection, table: &str) -> i64 {
    sql.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn snapshot(sql: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "sessions",
        "records",
        "usage",
        "tool_uses",
        "session_sources",
        "record_sources",
        "capture_receipts",
        "capture_record_coverage",
        "source_cursors",
    ]
    .into_iter()
    .map(|table| {
        let mut stmt = sql
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

#[tokio::test]
async fn import_conversation_is_durable_enriches_duplicates_and_covers_only_submitted_fields() {
    let mut server = Server::new().await;
    let first = server
        .import(args(
            "capture",
            vec![
                poor("first"),
                poor("second"),
                json!({"type":"user","message":{}}),
            ],
        ))
        .await;
    let out = success(&first);
    assert_eq!(out["conversation_id"], "capture");
    assert_eq!(out["ack_through"], "second");
    assert_eq!(
        (
            out["messages_received"].as_u64(),
            out["records_new"].as_u64(),
            out["records_dropped"].as_u64()
        ),
        (Some(3), Some(2), Some(1))
    );
    assert_eq!(out["pending"], 0);
    assert_eq!(out["draining"], false);
    assert_eq!(
        out["scope"],
        json!({"source":"local","agent_brain_id":null,"org_name":"local","workspace_name":"local"})
    );
    assert_eq!(out["provenance_received"], json!({"github_pr_urls":[]}));
    let event = server.changed().await;
    assert_eq!(event.records_new, 2);
    assert!(event.invalidate_measurements);
    let stored = server.store().records("capture").unwrap();
    assert_eq!(stored.len(), 2);
    assert!(
        stored
            .iter()
            .all(|r| r.usage.is_none() && r.model.is_none() && r.ts.is_none())
    );
    let sql = server.sql();
    let first_receipt: String = sql
        .query_row("SELECT receipt_id FROM capture_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    let initial_coverage = server.store().capture_coverage(&first_receipt).unwrap();
    let second = server.import(args("capture", vec![rich("first")])).await;
    let out = success(&second);
    assert_eq!(
        (
            out["records_new"].as_u64(),
            out["records_enriched"].as_u64()
        ),
        (Some(0), Some(1))
    );
    assert_eq!(out["ack_through"], "first");
    assert!(server.changed().await.invalidate_cost);
    assert_eq!(
        server.store().capture_coverage(&first_receipt).unwrap(),
        initial_coverage
    );
    let rich_coverage: (i64, String) = sql.query_row("SELECT metric_field_mask,measurement_revision FROM capture_record_coverage WHERE receipt_id!=?1", [&first_receipt], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    let sparse = server.import(args("capture", vec![poor("first")])).await;
    let out = success(&sparse);
    assert_eq!(
        (
            out["records_new"].as_u64(),
            out["records_enriched"].as_u64()
        ),
        (Some(0), Some(0))
    );
    assert_eq!(out["ack_through"], "first");
    server.changed().await;
    let last: (i64, String) = sql.query_row("SELECT metric_field_mask,measurement_revision FROM capture_record_coverage ORDER BY rowid DESC LIMIT 1", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_ne!(last, rich_coverage);
    assert_eq!(count(&sql, "capture_receipts"), 3);
    assert_eq!(count(&sql, "capture_record_coverage"), 4);
    // The sparse receipt cannot inherit the stored usage or cover omitted second.
    let reopened = server.store();
    let first = &reopened.records("capture").unwrap()[0];
    assert_eq!(first.usage.as_ref().unwrap().output_tokens, Some(3));
    assert_eq!(
        first.ts.as_deref(),
        Some("2026-09-10T01:02:03.123456789012Z")
    );
    assert_eq!(
        sql.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn import_conversation_preserves_unknown_identity_namespace_and_local_routing() {
    let mut server = Server::new().await;
    let mut input = args("novel-session", vec![poor("novel")]);
    input["source_platform"] = json!("future-host");
    input["source_surface"] = json!("Future Desktop");
    input["native_session_id"] = json!("native-id");
    input["namespace"] = json!("synthetic-namespace");
    input["title"] = json!("Synthetic title");
    input["agent_brain_id"] = json!("ignored-cloud-brain");
    input["org_id"] = json!("ignored-cloud-org");
    input["provenance"] = json!({"github_pr_urls":["https://github.com/example/synthetic/pull/1"]});
    success(&server.import(input).await);
    let session = server.store().session("novel-session").unwrap().unwrap();
    assert_eq!(session.meta.host, xt_store::Host::Other);
    assert_eq!(session.meta.source_platform.as_deref(), Some("future-host"));
    assert_eq!(session.meta.surface.as_deref(), Some("Future Desktop"));
    assert_eq!(session.meta.native_session_id.as_deref(), Some("native-id"));
    assert_eq!(session.namespace.as_deref(), Some("synthetic-namespace"));
    assert!(session.meta.title.is_none());
    assert_eq!(server.changed().await.surface, session.meta.surface);
    let mut sparse = args("novel-session", vec![poor("novel")]);
    sparse["source_platform"] = json!("future-host");
    sparse["namespace"] = json!("conflicting-namespace");
    success(&server.import(sparse).await);
    let session = server.store().session("novel-session").unwrap().unwrap();
    assert_eq!(session.namespace.as_deref(), Some("synthetic-namespace"));
    assert!(session.has_conflict);
    assert_eq!(
        server.changed().await.surface.as_deref(),
        Some("Future Desktop")
    );
    let surfaces = server
        .sql()
        .prepare("SELECT surface FROM capture_receipts ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(surfaces, [Some("Future Desktop".into()), None]);
    success(&server.import(args("legacy", vec![poor("legacy")])).await);
    assert!(
        server
            .store()
            .session("legacy")
            .unwrap()
            .unwrap()
            .meta
            .surface
            .is_none()
    );
    let mut native = args("codex-native", vec![poor("codex")]);
    native["source_platform"] = json!("codex");
    native["native_session_id"] = json!("native");
    success(&server.import(native.clone()).await);
    native["conversation_id"] = json!("wrong-id");
    failure(&server.import(native).await);
    assert!(server.store().session("wrong-id").unwrap().is_none());
}

#[tokio::test]
async fn import_conversation_requires_retry_stable_identity_and_recovers_after_an_unread_response()
{
    let mut server = Server::new().await;
    let before = snapshot(&server.sql());
    for input in [
        json!({"messages":[poor("missing")],"source_platform":"claude"}),
        json!({"messages":[poor("missing")],"source_platform":"claude","conversation_id":null}),
    ] {
        failure(&server.import(input).await);
        assert_eq!(snapshot(&server.sql()), before);
    }
    let arguments = args("retry-stable", vec![rich("lost-response")]);
    let body = json!({"jsonrpc":"2.0","id":"lost","method":"tools/call","params":{"name":"import_conversation","arguments":arguments}}).to_string();
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port))
        .await
        .unwrap();
    let headers = format!(
        "POST /mcp-server/mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        server.port,
        body.len()
    );
    stream.write_all(headers.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    assert_eq!(server.changed().await.conversation_id, "retry-stable");
    drop(stream); // The first acknowledgement never reaches the caller.
    let retry = server.import(arguments).await;
    let output = success(&retry);
    assert_eq!(output["conversation_id"], "retry-stable");
    assert_eq!(output["ack_through"], "lost-response");
    assert_eq!(output["records_new"], 0);
    assert_eq!(count(&server.sql(), "sessions"), 1);
    assert_eq!(count(&server.sql(), "records"), 1);
    assert_eq!(count(&server.sql(), "capture_receipts"), 2);
    assert!(
        !server
            .store()
            .session("retry-stable")
            .unwrap()
            .unwrap()
            .has_conflict
    );
    assert!(!server.store().records("retry-stable").unwrap()[0].has_conflict);
    let listing = server
        .raw(&json!({"jsonrpc":"2.0","id":"schema","method":"tools/list"}).to_string())
        .await;
    let schema = &listing["result"]["tools"][0]["inputSchema"];
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("conversation_id"))
    );
}

#[tokio::test]
async fn import_conversation_metadata_policy_applies_to_http_writes_and_retries() {
    let server = Server::new().await;
    assert_eq!(
        server.store().retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    let mut input = args("metadata", vec![poor("metadata-row")]);
    input["title"] = json!("Do not retain this title");
    input["namespace"] = json!("synthetic-metadata");
    success(&server.import(input.clone()).await);
    input["messages"] = json!([rich("metadata-row")]);
    let enriched = server.import(input.clone()).await;
    assert_eq!(success(&enriched)["records_new"], 0);
    assert_eq!(success(&enriched)["records_enriched"], 1);
    success(&server.import(input).await);
    let store = server.store();
    let record = &store.records("metadata").unwrap()[0];
    assert!(record.content_json.is_none());
    assert!(
        record
            .tool_uses
            .iter()
            .all(|tool| tool.input_json.is_none())
    );
    assert_eq!(record.usage.as_ref().unwrap().output_tokens, Some(3));
    assert!(
        store
            .session("metadata")
            .unwrap()
            .unwrap()
            .meta
            .title
            .is_none()
    );
    assert_eq!(
        store
            .session("metadata")
            .unwrap()
            .unwrap()
            .namespace
            .as_deref(),
        Some("synthetic-metadata")
    );
    assert_eq!(record.text_len, Some(14));
    assert_eq!(record.tool_use_count, Some(1));
    assert_eq!(count(&server.sql(), "capture_receipts"), 3);
    for query in [
        "SELECT count(*) FROM sessions WHERE title IS NOT NULL",
        "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
        "SELECT count(*) FROM tool_uses WHERE input_json IS NOT NULL",
        "SELECT count(*) FROM settings WHERE key='content_retention'",
    ] {
        assert_eq!(
            server
                .sql()
                .query_row(query, [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    // The server's already-opened connection observes an explicit opt-in.
    server
        .store()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let mut archive = args("archive", vec![rich("archived-row")]);
    archive["title"] = json!("Explicitly archived title");
    success(&server.import(archive.clone()).await);
    let retained = server.store().records("archive").unwrap().remove(0);
    assert!(retained.content_json.is_some());
    assert!(retained.tool_uses[0].input_json.is_some());
    assert_eq!(
        server
            .store()
            .session("archive")
            .unwrap()
            .unwrap()
            .meta
            .title
            .as_deref(),
        Some("Explicitly archived title")
    );
    server
        .store()
        .set_retention_mode(RetentionMode::MetadataOnly)
        .unwrap();
    archive["messages"] = json!([rich("archived-row"), rich("new-metadata-row")]);
    success(&server.import(archive).await);
    let rows = server.store().records("archive").unwrap();
    assert_eq!(
        rows.iter().find(|r| r.uuid == "archived-row").unwrap(),
        &retained
    );
    let fresh = rows.iter().find(|r| r.uuid == "new-metadata-row").unwrap();
    assert!(fresh.content_json.is_none() && fresh.tool_uses[0].input_json.is_none());
}

#[tokio::test]
async fn import_conversation_validation_and_final_commit_failure_cannot_ack_or_publish() {
    let mut server = Server::new().await;
    let sql = server.sql();
    let before = snapshot(&sql);
    for rows in [
        vec![],
        vec![json!({"type":"assistant"})],
        vec![
            poor("good"),
            json!({"uuid":"bad","type":"assistant","timestamp":"not-a-time"}),
        ],
        vec![poor("good"), json!("not-an-object")],
        vec![poor("same"); 2001],
    ] {
        failure(&server.import(args("invalid", rows)).await);
        assert_eq!(snapshot(&sql), before);
        assert!(server.events.try_recv().is_err());
    }
    // Force failure at COMMIT, after records and receipt coverage have been staged.
    sql.execute_batch("CREATE TABLE test_commit(parent TEXT REFERENCES sessions(session_id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER test_commit_failure AFTER INSERT ON capture_receipts BEGIN INSERT INTO test_commit VALUES('missing-parent'); END;").unwrap();
    let mut input = args("failed", vec![rich("failed")]);
    input["namespace"] = json!("must-roll-back");
    failure(&server.import(input.clone()).await);
    assert_eq!(snapshot(&sql), before);
    assert_eq!(count(&sql, "test_commit"), 0);
    assert!(server.events.try_recv().is_err());
    sql.execute_batch("DROP TRIGGER test_commit_failure")
        .unwrap();
    success(&server.import(input).await);
    assert_eq!(server.changed().await.conversation_id, "failed");
    assert_eq!(
        server
            .store()
            .session("failed")
            .unwrap()
            .unwrap()
            .namespace
            .as_deref(),
        Some("must-roll-back")
    );
}

#[tokio::test]
async fn import_conversation_rejected_owner_conflict_still_publishes_committed_invalidation() {
    let mut server = Server::new().await;
    server
        .store()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    for (id, uuid) in [("owner", "owned"), ("existing", "existing-row")] {
        let mut input = args(id, vec![poor(uuid)]);
        input["source_surface"] = json!("original");
        input["namespace"] = json!("original-namespace");
        input["title"] = json!("Original title");
        success(&server.import(input).await);
        server.changed().await;
    }
    let existing = server.store().session("existing").unwrap();
    let sources = server.store().session_sources("existing").unwrap();
    for destination in ["other", "existing", "owner"] {
        let mut row = poor("owned");
        if destination == "owner" {
            row["type"] = json!("user");
        }
        let mut input = args(destination, vec![row]);
        input["source_surface"] = json!("rejected-surface");
        input["namespace"] = json!("rejected-namespace");
        input["title"] = json!("Rejected title");
        failure(&server.import(input).await);
        let event = server.changed().await;
        assert_eq!(event.conversation_id, "owner");
        assert_eq!(event.surface.as_deref(), Some("original"));
        assert!(event.invalidate_measurements && event.invalidate_cost);
        assert!(server.events.try_recv().is_err());
        assert!(server.store().session("other").unwrap().is_none());
        assert!(server.store().session_sources("other").unwrap().is_empty());
        assert_eq!(server.store().session("existing").unwrap(), existing);
        assert_eq!(server.store().session_sources("existing").unwrap(), sources);
        assert!(server.store().records("owner").unwrap()[0].has_conflict);
        assert_eq!(count(&server.sql(), "capture_receipts"), 2);
        assert_eq!(
            server
                .store()
                .session("owner")
                .unwrap()
                .unwrap()
                .meta
                .title
                .as_deref(),
            Some("Original title")
        );
    }
    // A partly accepted import still commits its destination and acknowledges only accepted rows.
    let result = server
        .import(args("other", vec![poor("owned"), poor("fresh")]))
        .await;
    assert_eq!(success(&result)["ack_through"], "fresh");
    assert_eq!(success(&result)["records_new"], 1);
    assert!(server.store().session("other").unwrap().is_some());
    let events = [server.changed().await, server.changed().await];
    assert!(events.iter().any(|e| e.conversation_id == "other"));
    assert!(events.iter().any(|e| e.conversation_id == "owner"));
    assert_eq!(count(&server.sql(), "capture_receipts"), 3);
    let sql = server.sql();
    let before = snapshot(&sql);
    // The conflict-only path still has to commit before publishing its event.
    sql.execute_batch("CREATE TABLE conflict_commit(parent TEXT REFERENCES sessions(session_id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER conflict_commit_failure AFTER UPDATE OF has_conflict ON records BEGIN INSERT INTO conflict_commit VALUES('absent'); END;").unwrap();
    failure(
        &server
            .import(args("failed-destination", vec![poor("fresh")]))
            .await,
    );
    assert_eq!(snapshot(&sql), before);
    assert_eq!(count(&sql, "conflict_commit"), 0);
    assert!(server.events.try_recv().is_err());
}

#[tokio::test]
async fn import_conversation_duplicate_json_keys_are_rejected_before_erasure() {
    let mut server = Server::new().await;
    let prefix = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"import_conversation","arguments":{"conversation_id":"duplicate","source_platform":"claude","messages":["#;
    for record in [
        r#"{"uuid":"one","uuid":"two","type":"assistant"}"#,
        r#"{"uuid":"one","\u0075uid":"two","type":"assistant"}"#,
        r#"{"uuid":"one","type":"assistant","message":{"usage":{"output_tokens":1,"output_tokens":2}}}"#,
        r#"{"uuid":"one","type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"path":"a","path":"b"}}]}}"#,
    ] {
        let result = server.raw(&format!("{prefix}{record}]}}}}}}")).await;
        assert_eq!(result["error"]["code"], -32700);
        assert_eq!(server.store().counts().unwrap().sessions, 0);
        assert!(server.events.try_recv().is_err());
    }
    // A legitimate canonical body larger than the token route's cap is accepted.
    let mut row = rich("large");
    row["message"]["content"][0]["text"] = json!("x".repeat(80 * 1024));
    success(&server.import(args("large", vec![row])).await);
}

#[tokio::test]
async fn import_conversation_concurrent_duplicates_remain_one_record() {
    let server = Server::new().await;
    let first = args("concurrent", vec![poor("shared")]);
    let second = args("concurrent", vec![rich("shared")]);
    let (a, b) = tokio::join!(server.import(first), server.import(second));
    assert_eq!(
        success(&a)["records_new"].as_u64().unwrap() + success(&b)["records_new"].as_u64().unwrap(),
        1
    );
    assert_eq!(server.store().counts().unwrap().records, 1);
    assert_eq!(
        server.store().records("concurrent").unwrap()[0]
            .usage
            .as_ref()
            .unwrap()
            .output_tokens,
        Some(3)
    );
    assert_eq!(count(&server.sql(), "capture_receipts"), 2);
}

#[tokio::test]
async fn import_conversation_bounds_message_derived_identity_labels() {
    let mut server = Server::new().await;
    for key in [
        "source_surface",
        "entrypoint",
        "native_session_id",
        "sessionId",
    ] {
        for value in ["x".repeat(513), "é".repeat(257)] {
            let mut record = poor("oversized");
            record[key] = json!(value);
            let mut input = args("label-probe", vec![record]);
            input["source_platform"] = json!("future-host");
            let before = snapshot(&server.sql());
            failure(&server.import(input).await);
            assert_eq!(snapshot(&server.sql()), before);
            assert!(server.events.try_recv().is_err());
        }
        let mut record = poor(key);
        record[key] = json!("é".repeat(256));
        let mut input = args(key, vec![record]);
        input["source_platform"] = json!("future-host");
        success(&server.import(input).await);
        let session = server.store().session(key).unwrap().unwrap();
        let saved = if ["source_surface", "entrypoint"].contains(&key) {
            session.meta.surface.unwrap()
        } else {
            session.meta.native_session_id.unwrap()
        };
        assert_eq!(saved.len(), 512);
        server.changed().await;
    }
}
