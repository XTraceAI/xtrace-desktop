# Architecture

The workspace contains a native Tauri 2 shell with a React UI.
`xt-store` implements canonical SQLite storage, migrations, replay-safe writes
and typed reads. Its [API contract](../crates/xt-store/README.md) and
[acceptance cases](acceptance/storage.md) describe the available behavior.
`xt-fixtures` and `xtask` provide a shared synthetic catalog, disposable SQLite
databases and fixture validation/export commands. Their [acceptance cases](acceptance/fixtures.md)
distinguish the populated baseline from unimplemented skeletons.
`xt-ingest` provides canonical parsing, identity resolution and the transactional
writer used by `xt-server` for durable local imports. The server acknowledges
only committed records and receipt evidence; see the [server guide](SERVER.md)
and [import contract](wire-contract.md). Metrics and indexing are the default,
with full-content archival available only through a saved opt-in.
`xt-ingest::native` performs the initial import of native history: Claude JSONL
is read directly, Codex and Cursor through the shared readers pinned by
`.plugin-pin`, all as one header-plus-records stream into the same writer; see
[native import](acceptance/native-import.md). `xt_ingest::native::checkpoint`
defines generation-aware resume checkpoints (proven before any input is skipped,
committed only with the rows they cover) and `xt_ingest::native::watch` tails the
native roots with `notify`, reconciling changes queued during the initial scan
before it reports ready, and cancels a scan on shutdown (a reader still running
is killed and reaped). The native app starts that tailer at launch over the
user's home against its own database, running the Codex/Cursor readers from the
bundled copy of the pinned sources (`vendor/agent-plugins`, verified by object
identity without Git) with a discovered Python 3.10+ interpreter, and exposes the
result as the typed `native_index_status` command and `native-index://status`
event; see [native import](acceptance/native-import.md#app-integration). The app
exposes `app_info` and `db_counts` as well, but does not start the capture server
automatically. Production metric calculation, product data queries and the
remaining subsystem behavior are separate work.

The following table names each subsystem's responsibility. A reserved directory
or compiling placeholder does not establish implemented behavior.

| Path                      | Subsystem responsibility                                                                             |
| ------------------------- | ---------------------------------------------------------------------------------------------------- |
| `crates/xt-store/`        | Storage, canonical schema, migrations, and transactional writes.                                     |
| `crates/xt-ingest/`       | Normalization, parsers, native source import through the pinned readers, and the canonical writer.   |
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
Rust metrics → native commands and renderer. The loopback import adapter
already feeds the shared canonical writer; the metric and renderer integration
remain separate work. JavaScript consumes metric DTOs rather than
reimplementing calculations. The fixture harness uses the canonical storage API;
its bounded baseline reference assertions do not implement the production metric
engine or establish whole-rule coverage.

Keep subsystem contracts explicit at crate boundaries. Local checks are in
[CONTRIBUTING.md](../CONTRIBUTING.md); [CI.md](CI.md) describes automated
validation and its limits.

The monorepo directory layout uses [Cap](https://github.com/CapSoftware/Cap)
as a structural reference only.
No code or assets from that project are included in this workspace.
