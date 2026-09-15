# Local server transport

`xt-server` provides guarded loopback HTTP transport and durable canonical conversation imports for native plugin callers. Metrics and indexing are the default; full-content archival requires an explicitly saved opt-in. The server does not issue cloud credentials, start host watchers or expose the desktop UI. The independent `xtrace-core` binary lives in this crate so it can run without a Tauri runtime.

```sh
cargo run -p xt-server --bin xtrace-core -- serve --db /tmp/xtrace-synthetic.db --port 0
```

Create the database's parent directory first. The command prints `XTRACE_PORT=<actual port>` after binding. Explicit `--port` overrides the persisted `settings` key `server.port`; otherwise the default is 47421. Persisted ports must be integer JSON values from 1 through 65535. Explicit zero requests an ephemeral test port. An occupied configured port fails instead of choosing another. `--bind` accepts only exact `127.0.0.1` or `::1`; invalid addresses fail before opening the database. The library also revalidates a supplied listener.

## Request boundary

Every registered route and fallback requires one literal Host matching `127.0.0.1:<bound port>`, `localhost:<bound port>` or `[::1]:<bound port>`. A present Origin must equal `http://` plus that same authority; absent Origin is allowed for native clients. Foreign authorities, wrong ports, extra origin values, credentials, suffixes and trailing paths fail with 403. The guard does not resolve DNS or trust forwarded-host headers. POST requests require one `application/json` Content-Type (parameters are allowed), or receive 415. The MCP route accepts JSON bodies up to 16 MiB; token routes retain a 64 KiB limit. Oversized bodies receive 413. Duplicate object keys, including nested or escaped equivalents, are rejected before deserialization.

Authorization values do not identify a cloud account. This local compatibility layer accepts any bearer after the request boundary passes. It returns a constant token named `local`; mint echoes supplied label/scopes/expiry metadata, list returns one generic local record, and delete is a stateless no-op. These endpoints are compatibility stubs, not credential-management authority.

| Route                                     | Behavior                                                                    |
| ----------------------------------------- | --------------------------------------------------------------------------- |
| `GET /health`                             | App version and this server's database path.                                |
| `POST /v1/developer/access-tokens`        | REST `{code:0,msg:"ok",data:{secret:"local",access_token:{...}}}` envelope. |
| `GET /v1/developer/access-tokens`         | One generic local token in the same REST envelope.                          |
| `DELETE /v1/developer/access-tokens/{id}` | Successful no-op in the REST envelope.                                      |
| `POST /mcp-server/mcp`                    | JSON-RPC initialize, ping, tools/list and durable conversation imports.     |

MCP request responses use `event: message` / `data: <JSON>` SSE framing with no-store caching. Notifications receive 202 with an empty body. The supported protocol is `2025-06-18`, matching the pinned plugin client; initialize reports that version. Unknown methods/tools and malformed requests receive JSON-RPC errors. `tools/list` exposes `import_conversation`. It requires a caller-owned stable conversation ID, source platform and 1–2,000 canonical messages. Successful imports return a structured acknowledgement only after the shared transaction commits. Retrying the same identity and UUIDs does not duplicate records; usage may enrich existing records. Transaction failure returns `isError:true` without an acknowledgement. Wholly rejected ownership collisions may commit conflict flags on the original owner, but create or enrich no destination session. See the [import wire contract](wire-contract.md) for arguments, receipt accounting, content policy and retry behavior. There is no debug/E2E route in either build profile.

The transport follows the pinned [Axum serve API](https://docs.rs/axum/0.8.9/axum/fn.serve.html) and the plugin's [MCP transport revision](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports). This implementation supports the documented local subset of MCP.

## Verification

```sh
cargo test -p xt-server --locked
cargo clippy -p xt-server --all-targets --locked -- -D warnings
cargo build -p xt-server --bin xtrace-core --release --locked
python3 scripts/conformance/test-plugin-transport.py --plugin-root "$AGENT_PLUGINS_DIR/plugins/memhub" --expected-commit "$(node scripts/ci/plugin-pin.mjs commit)" --binary target/release/xtrace-core
```

The same binary imports native history without the server: `xtrace-core import-native --db PATH --home DIR` reads Claude, Codex and Cursor sources through the pinned readers into the index and prints a JSON report; see [native import](acceptance/native-import.md).
Rescans resume behind proven checkpoints; `--replay` rereads every source. `xtrace-core watch-native --db PATH --home DIR [--once | --for SECONDS]` keeps the index current: it registers a filesystem watcher before the initial scan, reconciles queued changes before printing `ready`, then reconciles live changes, one JSON line per event; see [native import](acceptance/native-import.md).

The conformance command requires the real plugin checkout at the revision declared in `.plugin-pin`; it verifies selected client bytes against Git before importing them. The required local native gate runs the same harness, the Stop-hook import harness and the pinned native reader stream through `scripts/ci/plugin-conformance.sh`, which obtains the pinned checkout itself; see [CI](CI.md#pinned-plugin-conformance). It starts only its own temporary server, uses synthetic credentials/data and terminates that exact child. Missing or mismatched prerequisites fail, with no successful skip. The mandatory local native gate also exercises imports through the real pinned Stop hook, including cursor retry after a failed commit and metadata-only storage. It fails if producer or Python prerequisites are missing. [Transport acceptance](acceptance/local-server.md) and [import acceptance](acceptance/conversation-import.md) record their separate cases and limits. Automatic startup in the native app remains separate integration work.
