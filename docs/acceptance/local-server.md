# Local server transport acceptance

Six Rust integration cases pass against real listeners and the built headless binary. The guard and transport are exercised through the real HTTP server, rather than mocked handlers.

| Case           | Setup/action                                                                                                                                                    | Expected and observed result                                                                                                                             |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Route guard    | All health/MCP/token/fallback paths with foreign Host, wrong port, authority tricks, foreign/mismatched/duplicate Origin, missing/wrong/duplicate Content-Type. | Host/Origin failures return 403; non-JSON POST returns 415. Exact allowed authorities and their matching Origins work; native calls without Origin work. |
| Bind/rebind    | Exact localhost with port 0; occupied port; wildcard/external/alternate 127/IPv4-mapped address; externally bound supplied listener.                            | Actual ports differ, conflicts fail, every unsupported bind is rejected.                                                                                 |
| MCP            | Initialize with numeric/string/null IDs, notifications, tools/list, unknown method/tool, malformed JSON/request and oversized body.                             | IDs preserved, SSE decodes, notifications 202/empty, one descriptor, correct error codes, oversized 413; import stub never acknowledges a write.         |
| Local token    | Mint/list/delete with arbitrary synthetic bearer and label/scopes/expiry.                                                                                       | Constant local ID/secret, correct REST envelope and valid UTC creation time; deletion remains a no-op.                                                   |
| CLI lifecycle  | Start actual binary with ephemeral port, conflict and invalid/duplicate options.                                                                                | Bound port reported promptly; failed startup has no success output and exact test child is cleaned up. Invalid bind never creates the database.          |
| Persisted port | Occupy the port in settings, then override with 0; test invalid persisted JSON values.                                                                          | Configured conflict fails, explicit override wins, invalid settings fail without silent fallback.                                                        |

## Verification

On the current storage migration chain, the combined server/store/fixture/CLI
suite passed 77 tests, including all six real transport and headless cases.
The actual pinned plugin decoder and token client passed against both debug and
release binaries at revision `fae871a5c3385d14e55ea64f6bda7421c52ebebb`.
The release headless binary builds without a Tauri runtime. `pnpm check` passed
78 UI tests, seven script tests, type checking, lint and formatting. Updated
third-party notices passed the existing dependency license policy.

Run `cargo test -p xt-server --locked` for six real-listener and headless-binary cases. Run the explicit pinned-plugin command in [SERVER.md](../SERVER.md) against both debug and release binaries to exercise the actual decoder and token client. Missing or mismatched prerequisites fail. The conformance script uses synthetic inputs and a temporary database, then terminates only its own child process.

The tests exercise the same library used by the binary. The binary is deliberately placed in `xt-server/src/bin` to avoid a Tauri dependency while keeping the `xtrace-core serve` command. Import logic, durable acknowledgements, desktop start/stop/rebind UI and plugin installation remain separate work. The constant-token API provides local compatibility; it does not establish cloud authentication.
