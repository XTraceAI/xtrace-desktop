# Local server transport

`xt-server` provides a headless HTTP scaffold for native plugin callers. It does not import records, issue cloud credentials, start host watchers or expose the desktop UI. The independent `xtrace-core` binary lives in this crate so it can run without a Tauri runtime.

```sh
cargo run -p xt-server --bin xtrace-core -- serve --db /tmp/xtrace-synthetic.db --port 0
```

Create the database's parent directory first. The command prints `XTRACE_PORT=<actual port>` after binding. Explicit `--port` overrides the persisted `settings` key `server.port`; otherwise the default is 47421. Persisted ports must be integer JSON values from 1 through 65535. Explicit zero requests an ephemeral test port. An occupied configured port fails instead of choosing another. `--bind` accepts only exact `127.0.0.1` or `::1`; invalid addresses fail before opening the database. The library also revalidates a supplied listener.

## Request boundary

Every registered route and fallback requires one literal Host matching `127.0.0.1:<bound port>`, `localhost:<bound port>` or `[::1]:<bound port>`. A present Origin must equal `http://` plus that same authority; absent Origin is allowed for native clients. Foreign authorities, wrong ports, extra origin values, credentials, suffixes and trailing paths fail with 403. The guard does not resolve DNS or trust forwarded-host headers. POST requests require one `application/json` Content-Type (parameters are allowed), or receive 415. JSON bodies are limited to 64 KiB in this scaffold; oversized bodies receive 413. The future import owner must choose and test its own payload bound.

Authorization values do not identify a cloud account. This local compatibility layer accepts any bearer after the request boundary passes. It returns a constant token named `local`; mint echoes supplied label/scopes/expiry metadata, list returns one generic local record, and delete is a stateless no-op. These endpoints are compatibility stubs, not credential-management authority.

| Route                                     | Behavior                                                                    |
| ----------------------------------------- | --------------------------------------------------------------------------- |
| `GET /health`                             | App version and this server's database path.                                |
| `POST /v1/developer/access-tokens`        | REST `{code:0,msg:"ok",data:{secret:"local",access_token:{...}}}` envelope. |
| `GET /v1/developer/access-tokens`         | One generic local token in the same REST envelope.                          |
| `DELETE /v1/developer/access-tokens/{id}` | Successful no-op in the REST envelope.                                      |
| `POST /mcp-server/mcp`                    | JSON-RPC initialize, ping, tools/list and explicit tools/call errors.       |

MCP request responses use `event: message` / `data: <JSON>` SSE framing with no-store caching. Notifications receive 202 with an empty body. The supported protocol is `2025-06-18`, matching the pinned plugin client; initialize reports that version. Unknown methods/tools and malformed requests receive JSON-RPC errors. `tools/list` exposes one `import_conversation` descriptor; calling it returns `isError:true` with no acknowledgement or fabricated record count. There is no debug/E2E route in either build profile.

The transport follows the pinned [Axum serve API](https://docs.rs/axum/0.8.9/axum/fn.serve.html) and the plugin's [MCP transport revision](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports). This limited scaffold is not a claim of full MCP capability coverage.

## Verification

```sh
cargo test -p xt-server --locked
cargo clippy -p xt-server --all-targets --locked -- -D warnings
cargo build -p xt-server --bin xtrace-core --release --locked
python3 scripts/conformance/test-plugin-transport.py --plugin-root "$AGENT_PLUGINS_DIR/plugins/memhub" --expected-commit fae871a5c3385d14e55ea64f6bda7421c52ebebb --binary target/release/xtrace-core
```

The conformance command requires the real plugin checkout at the named revision; it verifies selected client bytes against Git before importing them. It starts only its own temporary server, uses synthetic credentials/data and terminates that exact child. Missing or mismatched prerequisites fail, with no successful skip. This manual contract evidence does not install the later repository-wide plugin conformance gate. [Acceptance evidence](acceptance/ING-06.md) records observed behavior and limits.
