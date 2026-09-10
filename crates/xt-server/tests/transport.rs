use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use xt_store::Store;

struct Server {
    port: u16,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    _dir: Arc<tempfile::TempDir>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server() -> Server {
    let dir = Arc::new(tempfile::TempDir::new().unwrap());
    let path = dir.path().join("synthetic.db");
    let listener = xt_server::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let store = Store::open(&path).unwrap();
    let keep = dir.clone();
    let task = tokio::spawn(async move {
        let _keep = keep;
        xt_server::serve(listener, store, path, std::future::pending()).await
    });
    Server {
        port,
        task,
        _dir: dir,
    }
}
async fn request(server: &Server, method: &str, path: &str, headers: &str, body: &str) -> String {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port))
        .await
        .unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\n{headers}Connection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stream.read_to_end(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    String::from_utf8(bytes).unwrap()
}
fn status(response: &str) -> u16 {
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}
fn json_body(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}
fn rpc(response: &str) -> serde_json::Value {
    assert!(response.contains("text/event-stream"));
    serde_json::from_str(
        response
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn loopback_guard_covers_every_route_origin_host_port_and_media_type() {
    let server = server().await;
    let routes = [
        ("GET", "/health", ""),
        (
            "POST",
            "/mcp-server/mcp",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ),
        ("GET", "/v1/developer/access-tokens", ""),
        (
            "POST",
            "/v1/developer/access-tokens",
            r#"{"label":"Synthetic","scopes":[]}"#,
        ),
        ("DELETE", "/v1/developer/access-tokens/local", ""),
        ("GET", "/missing", ""),
        ("GET", "/__e2e", ""),
    ];
    for (method, path, body) in routes {
        for host in [
            "example.invalid".into(),
            format!("127.0.0.1:{}", server.port.wrapping_add(1)),
            "127.1:1".into(),
            format!("localhost.example.invalid:{}", server.port),
            format!("user@localhost:{}", server.port),
            format!("localhost.:{}", server.port),
        ] {
            assert_eq!(
                status(
                    &request(
                        &server,
                        method,
                        path,
                        &format!("Host: {host}\r\nContent-Type: application/json\r\n"),
                        body
                    )
                    .await
                ),
                403,
                "{method} {path} {host}"
            );
        }
        for origin in [
            "null".into(),
            "https://example.invalid".into(),
            format!("https://127.0.0.1:{}", server.port),
            format!("http://127.0.0.1:{}/", server.port),
            format!("http://localhost:{}", server.port),
            format!("http://127.0.0.1:{}", server.port.wrapping_add(1)),
        ] {
            assert_eq!(status(&request(&server,method,path,&format!("Host: 127.0.0.1:{}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\n",server.port),body).await),403,"{path} {origin}");
        }
        let headers = format!(
            "Host: 127.0.0.1:{}\r\nOrigin: http://127.0.0.1:{}\r\nContent-Type: application/json; charset=utf-8\r\nAuthorization: Bearer synthetic\r\n",
            server.port, server.port
        );
        assert_eq!(
            status(&request(&server, method, path, &headers, body).await),
            if ["/missing", "/__e2e"].contains(&path) {
                404
            } else {
                200
            }
        );
        if method == "POST" {
            for content in [
                "",
                "Content-Type: text/plain\r\n",
                "Content-Type: application/json\r\nContent-Type: text/plain\r\n",
            ] {
                assert_eq!(
                    status(
                        &request(
                            &server,
                            method,
                            path,
                            &format!("Host: localhost:{}\r\n{content}", server.port),
                            body
                        )
                        .await
                    ),
                    415
                );
            }
        }
    }
    for host in [
        format!("localhost:{}", server.port),
        format!("[::1]:{}", server.port),
    ] {
        assert_eq!(
            status(
                &request(
                    &server,
                    "GET",
                    "/health",
                    &format!("Host: {host}\r\nOrigin: http://{host}\r\n"),
                    ""
                )
                .await
            ),
            200
        );
    }
    let duplicate = format!(
        "Host: localhost:{}\r\nOrigin: http://localhost:{}\r\nOrigin: http://localhost:{}\r\n",
        server.port, server.port, server.port
    );
    assert_eq!(
        status(&request(&server, "GET", "/health", &duplicate, "").await),
        403
    );
}

#[tokio::test]
async fn loopback_bind_refuses_external_addresses_and_port_conflicts() {
    for ip in [
        "0.0.0.0",
        "::",
        "192.0.2.1",
        "127.0.0.2",
        "::ffff:127.0.0.1",
    ] {
        assert!(
            xt_server::bind(SocketAddr::new(ip.parse().unwrap(), 0))
                .await
                .is_err()
        );
    }
    let first = server().await;
    assert!(
        xt_server::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), first.port))
            .await
            .is_err()
    );
    let second = server().await;
    assert_ne!(first.port, second.port);
    for server in [&first, &second] {
        assert_eq!(
            status(
                &request(
                    server,
                    "GET",
                    "/health",
                    &format!("Host: localhost:{}\r\n", server.port),
                    ""
                )
                .await
            ),
            200
        );
    }
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    assert!(
        xt_server::serve(
            listener,
            Store::open_in_memory().unwrap(),
            "synthetic.db".into(),
            std::future::pending()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn mcp_transport_preserves_ids_sse_errors_notifications_and_import_validation() {
    let server = server().await;
    let headers = format!(
        "Host: localhost:{}\r\nContent-Type: application/json\r\n",
        server.port
    );
    for id in [
        serde_json::json!(7),
        serde_json::json!("synthetic"),
        serde_json::json!(null),
        serde_json::json!(1.5),
    ] {
        let body = serde_json::json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Synthetic","version":"1"}}});
        let result = rpc(&request(
            &server,
            "POST",
            "/mcp-server/mcp",
            &headers,
            &body.to_string(),
        )
        .await);
        assert_eq!(result["id"], id);
        assert_eq!(result["result"]["protocolVersion"], "2025-06-18");
    }
    let response = request(
        &server,
        "POST",
        "/mcp-server/mcp",
        &headers,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
    assert_eq!(status(&response), 202);
    assert_eq!(response.split("\r\n\r\n").nth(1), Some(""));
    let notification = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"import_conversation","arguments":{"conversation_id":"notification","source_platform":"claude","messages":[{"uuid":"notification-row","type":"assistant"}]}}}"#;
    assert_eq!(
        status(&request(&server, "POST", "/mcp-server/mcp", &headers, notification).await),
        202
    );
    assert_eq!(
        Store::open(server._dir.path().join("synthetic.db"))
            .unwrap()
            .counts()
            .unwrap()
            .sessions,
        0
    );
    for (method, code) in [("unknown", -32601), ("tools/call", -32602)] {
        let result = rpc(&request(
            &server,
            "POST",
            "/mcp-server/mcp",
            &headers,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":method}).to_string(),
        )
        .await);
        assert_eq!(result["error"]["code"], code);
    }
    let result = rpc(&request(
        &server,
        "POST",
        "/mcp-server/mcp",
        &headers,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    )
    .await);
    assert_eq!(result["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(result["result"]["tools"][0]["name"], "import_conversation");
    let result=rpc(&request(&server,"POST","/mcp-server/mcp",&headers,r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"import_conversation","arguments":{}}}"#).await);
    assert_eq!(result["error"]["code"], -32602);
    assert!(result.get("result").is_none());
    for (body, code) in [
        ("{", -32700),
        ("[]", -32600),
        (r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#, -32600),
    ] {
        assert_eq!(
            rpc(&request(&server, "POST", "/mcp-server/mcp", &headers, body).await)["error"]["code"],
            code
        );
    }
    let huge = serde_json::json!({"payload":"x".repeat(xt_server::MAX_MCP_BYTES)}).to_string();
    assert_eq!(
        status(&request(&server, "POST", "/mcp-server/mcp", &headers, &huge).await),
        413
    );
    let token_body = serde_json::json!({"label":"x".repeat(65*1024),"scopes":[]}).to_string();
    assert_eq!(
        status(
            &request(
                &server,
                "POST",
                "/v1/developer/access-tokens",
                &headers,
                &token_body
            )
            .await
        ),
        413
    );
}

#[tokio::test]
async fn conformance_token_envelope_is_constant_and_not_cloud_identity() {
    let server = server().await;
    let headers = format!(
        "Host: localhost:{}\r\nContent-Type: application/json\r\nAuthorization: Bearer any-synthetic-value\r\n",
        server.port
    );
    let minted = json_body(
        &request(
            &server,
            "POST",
            "/v1/developer/access-tokens",
            &headers,
            r#"{"label":"Synthetic","scopes":["memory:read"],"expires_at":"2099-01-01T00:00:00Z"}"#,
        )
        .await,
    );
    assert_eq!(minted["code"], 0);
    assert_eq!(minted["data"]["secret"], "local");
    assert_eq!(minted["data"]["access_token"]["id"], "local");
    assert_eq!(minted["data"]["access_token"]["label"], "Synthetic");
    assert_eq!(
        minted["data"]["access_token"]["expires_at"],
        "2099-01-01T00:00:00Z"
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(
            minted["data"]["access_token"]["created_at"]
                .as_str()
                .unwrap()
        )
        .is_ok()
    );
    assert_eq!(
        json_body(&request(&server, "GET", "/v1/developer/access-tokens", &headers, "").await)["data"]
            [0]["id"],
        "local"
    );
    for id in ["local", "unknown"] {
        assert_eq!(
            status(
                &request(
                    &server,
                    "DELETE",
                    &format!("/v1/developer/access-tokens/{id}"),
                    &headers,
                    ""
                )
                .await
            ),
            200
        );
    }
    assert_eq!(json_body(&request(&server,"GET","/v1/developer/access-tokens",&headers,"").await)["data"].as_array().unwrap().len(),1);
}
