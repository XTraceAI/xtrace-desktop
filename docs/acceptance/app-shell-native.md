# App shell native acceptance

The app owns one SQLite store. Native commands return Rust metadata/count DTOs; the development exporter serializes those same structs and ts-rs generates the TypeScript declarations. The canonical fixture loader creates the synthetic database.

## Startup and isolation

Normal startup uses `XTRACE_DATA_DIR` when provided, otherwise the platform application-data directory, and opens `xtrace.db`. Fixture startup requires a debug build with the optional `fixtures` feature and `XTRACE_FIXTURE=F1` or `--fixture F1`. The CLI wins over the environment; missing, duplicated or invalid fixture arguments fail.

Each fixture launch owns a fresh temporary database. The app explicitly closes the store and removes that directory on the native Exit event; repeated shutdown is harmless and subsequent reads report that the database is closed. An explicit data-directory override supplies only its parent. Fixture mode never resolves the default live directory or reuses an existing database. Capture is off; no probes or watchers start. Unpopulated fixture skeletons fail explicitly.

```sh
XTRACE_FIXTURE=F1 pnpm tauri dev --features fixtures
pnpm fixtures:export
VITE_XTRACE_FIXTURE=F1 pnpm dev
```

Quit an existing instance before changing startup flags; single-instance activation retains that instance's state. The browser export contains only the portable `fixture://F1` path label. Native Settings displays the actual owned directory locally, so native database screenshots are not published.

## Reproducible checks and expectations

| Check                                                                  | Expected result                                                                                                                                           |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test --workspace --all-features --locked`                       | Fixture isolation and cleanup, explicit live override and persistence, argument validation, canonical export parity and exact integer serialization pass. |
| `cargo test -p xtrace-desktop fixture_mode`                            | Default-feature startup rejects fixture selection before resolving live data.                                                                             |
| `bash scripts/ci/check-dto.sh`                                         | Generated files match byte for byte; stale, missing, extra and symlinked declaration inputs fail in its two negative-test groups.                         |
| `cargo check -p xtrace-desktop --release --locked`                     | Normal production native configuration compiles.                                                                                                          |
| `cargo check -p xtrace-desktop --release --features fixtures --locked` | Fails specifically at the intentional compile-time fixture guard.                                                                                         |
| `cargo tree -p xtrace-desktop --edges normal --prefix none --locked`   | The default application tree excludes `xt-fixtures`.                                                                                                      |
| `pnpm check:native --base <reviewed-base-sha> --release`               | Clean combined source/base validation, Rust checks, installed DTO hook, SBOM, licenses, secret scan and debug/production bundle launch pass locally.      |

These count fields are storage smoke counts, not computed product metrics. Serialization accepts integers through 2^53−1 and rejects larger counts in every field. Generated DTO and fixture tests run locally with Rust; ordinary hosted UI checks remain on Ubuntu.

## Native window observations

A debug fixture bundle was launched on macOS 26.5.2 with real native IPC. Settings showed schema 1, 1 session, 25 records and 15 usage rows, with the F1 badge and capture off. Cmd+, opened Settings. Light/dark selection changed the web content and native sidebar material together.

The sidebar uses Tauri's built-in macOS sidebar material and reserves space for the existing system buttons. The main pane stays opaque. Active/inactive appearance, native traffic-light spacing, full-screen entry and return, and the system Reduce Transparency opaque fallback were inspected in the running app. System appearance changes were also observed while the app preference remained System. Both system appearance and accessibility settings were restored after verification. Drag-region attributes have component coverage; the current native drag gesture requires human verification because the automation cannot reliably move the window. Browser previews and other platforms keep an opaque sidebar.

This local run does not establish the macOS 14/Safari 17 platform floor, signing/notarization, or final product-page design acceptance. Those release checks remain separate. [Frontend acceptance](app-shell-ui.md) covers routes, query/event behavior, offline rendering and production fixture exclusion.
