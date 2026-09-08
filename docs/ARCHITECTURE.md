# Architecture and ownership

FND-01 establishes the workspace and a native Tauri 2 shell with a React UI.
The empty Rust libraries are validated by workspace compilation. The app exposes only the scaffold's
`app_info` command; no database, ingestion pipeline, capture server, or host
integration is implemented by this PR.

The following map fixes the destination for later cards. Responsibilities below
are planned ownership, not a list of features already delivered.

| Path                      | Owner and planned responsibility                                                                     |
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

Older planning paths map here: `db/` becomes `xt-store`; `ingest/` and `sources/`
become `xt-ingest`; `server/` and the core command interface belong to `xt-server`;
`hosts/`, `probes/gh.rs`, and `binaries.rs` belong to `xt-probes`; rule and judge
code belongs to `xt-rulebook` and `xt-judge`. All renderer code lives under the
UI path above. The SCA-13 card owns the tray; release cards own distribution.

The intended flow is source observations → canonical ingestion/storage → shared
Rust metrics → native commands and renderer. The future loopback capture adapter
feeds the same canonical writer. JavaScript consumes metric DTOs rather than
reimplementing calculations. The future fixture harness exercises those same
interfaces; it does not introduce a second storage or metric implementation.

Keep each card within its assigned owner and make cross-crate contracts explicit
before dependent implementations begin. FND-03 adds storage, FND-04 adds fixture
behavior, and FND-09 adds the shared shell/data boundary. FND-02 adds CI; the
commands in [CONTRIBUTING.md](../CONTRIBUTING.md) are local checks today.

The monorepo directory layout follows the reference discussed in the project
plan, [Cap](https://github.com/CapSoftware/Cap), as a structural reference only.
No code or assets from that project are included by this scaffold.
