# Architecture

The workspace contains a native Tauri 2 shell with a React UI.
`xt-store` implements canonical SQLite storage, migrations, replay-safe writes
and typed reads. Its [API contract](../crates/xt-store/README.md) and
[acceptance cases](acceptance/storage.md) describe the available behavior.
The other subsystem libraries remain scaffolds validated by compilation.
The app exposes only `app_info`; storage is not connected to the desktop UI,
ingestion pipeline or host integrations yet.

The following subsystem boundaries describe the scaffold directories. Empty
libraries reserve these responsibilities; compilation does not establish
implemented behavior.

| Path                      | Subsystem responsibility                                                                             |
| ------------------------- | ---------------------------------------------------------------------------------------------------- |
| `crates/xt-store/`        | Storage, canonical schema, migrations, and transactional writes.                                     |
| `crates/xt-ingest/`       | Normalization, parsers, source cursors, and backfill into the canonical writer.                      |
| `crates/xt-server/`       | Guarded loopback HTTP/MCP endpoints and the headless core interface.                                 |
| `crates/xt-metrics/`      | Metric definitions, SQL views, aggregation, and shared result DTOs.                                  |
| `crates/xt-probes/`       | Host/plugin discovery, executable environment resolution, and `gh`/Git probes.                       |
| `crates/xt-rulebook/`     | Rule lifecycle, local proposals, verification, and fire history.                                     |
| `crates/xt-judge/`        | Isolated configured judge CLI routes and separate judge-usage accounting.                            |
| `crates/xt-fixtures/`     | Development-only fixture loading, database construction, and expected-result assertions.             |
| `xtask/`                  | Developer commands for shared fixture validation, database creation, and export.                     |
| `apps/desktop/src-tauri/` | Native lifecycle, Tauri commands, window chrome, and later tray, updater, and telemetry integration. |
| `apps/desktop/ui/src/`    | React shell, UI components, screens, and the typed data-access boundary.                             |

All renderer code lives under the UI path above. Native lifecycle and window
behavior belong to the Tauri application.

The intended flow is source observations → canonical ingestion/storage → shared
Rust metrics → native commands and renderer. The future loopback capture adapter
feeds the same canonical writer. JavaScript consumes metric DTOs rather than
reimplementing calculations. The future fixture harness exercises those same
interfaces; it does not introduce a second storage or metric implementation.

Keep subsystem contracts explicit at crate boundaries. Local checks are in
[CONTRIBUTING.md](../CONTRIBUTING.md); [CI.md](CI.md) describes automated
validation and its limits.

The monorepo directory layout uses [Cap](https://github.com/CapSoftware/Cap)
as a structural reference only.
No code or assets from that project are included by this scaffold.
