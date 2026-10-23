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
event; see [native import](acceptance/native-import.md#app-integration).
`xt_ingest::native::session_source` reads one session's original local source
on demand for the detail view. Claude files are read directly, with descriptor
and path checks before and after the read. Codex and Cursor JSONL sessions use
the verified pinned reader's `--exact-detail` mode and a Python 3.10+
interpreter resolved for that open. Reads are bounded and cancellable, hold
records in memory, and write no transcript cache or staged source copy. Reader
setup and execution failures return a closed unavailable reason; Cursor SQLite
stores remain unsupported. See [session source](acceptance/session-source.md).
The session page shows the M-09
hands-off stretches that session contributed in the selected window, read from
`xt-metrics` as metadata only, and reveals a stretch's first tool call only when
exactly one tool call in the loaded transcript sits at the position M-09 named.
Each stretch also carries M-20's repeat measurement of the same stretch, read in
the same snapshot, with no comparison key or argument on the wire;
see [session stretches](acceptance/session-stretches.md) and
[session timeline](acceptance/session-timeline.md). The app
exposes `app_info` and `db_counts` as well, but does not start the capture server
automatically. Dashboard, Sessions and Pull requests query production Rust
metrics through typed native commands.

`xt-judge` runs a grading skill (the bundled, unchanged `vendor/claude-md-doctor`,
or a skill the user installed and added to Scores because its folder has a valid
rubric) headless with the user's Claude Code on one project folder, without the user's
personal Claude Code settings, inside Claude Code's sandbox (the project
read-only, writes only in the run's grade folder, the network always off,
well-known secret locations unreadable, no hooks or MCP servers, no saved transcript; Claude Code 2.1.296 or newer). A final step of the same run judges the skill's output against the
skill's rubric, `xtrace-rubric.md` ([format](GRADING-RUBRIC.md)), and writes
`result.json`, which one strict protocol v1 reader checks. The score rule
belongs to the rubric; XTrace never recomputes it. The app stores grades and
shows them on the Scores page; `xtrace-core grade` runs one end to end for
developers. `xt_judge::installed` lists the skills installed under the native
home (`~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills` and enabled,
user-scoped Claude Code plugins), read-only and bounded, never following a link
out of a folder. `xt_judge::maker` makes a rubric for an installed skill: the
bundled helper skill `crates/xt-judge/rubric-maker` (shipped as an app
resource) runs headless on a staged copy of the skill with exactly a grading
run's command line and containment, and only XTrace writes the result after
the rubric reader accepts it. One rule, `xt_judge::rubric_store::applicable`,
decides which rubric applies to a skill (the folder's own `xtrace-rubric.md`,
else one XTrace keeps in `<data>/rubrics/` for a plugin's skill or an
unwritable folder) for discovery, opening an added skill and staging a run.
`xt_judge::refusals` keeps the rubric maker's refusals across restarts in
`<data>/rubric-refusals.json`, and one rule (`refusals::in_force`) says a kept
refusal applies only while the skill's fingerprint is unchanged.
`xt_judge::skill_link` handles "Add from a link" without any network code:
one reading of a public GitHub link, unpacking a downloaded archive within
bounds (no absolute paths, `..`, devices, hard links or links out of the
repository), finding the skills it holds, one rule for whether each can be
kept, keeping one in `~/.agents/skills` (never over an existing entry) with
a `~/.claude/skills` link when that name is free, and the source records in
`<data>/linked-skills.json`. The app's `skill_link` service is the only
place it uses the network: on the user's Look up it asks GitHub's API
(unauthenticated) for the link's commit and downloads that commit's archive
from `codeload.github.com` with the reqwest client the updater already uses.
Nothing downloaded is run.

The following table names each subsystem's responsibility. A reserved directory
or compiling placeholder does not establish implemented behavior.

| Path                      | Subsystem responsibility                                                                           |
| ------------------------- | -------------------------------------------------------------------------------------------------- |
| `crates/xt-store/`        | Storage, canonical schema, migrations, and transactional writes.                                   |
| `crates/xt-ingest/`       | Normalization, parsers, native source import through the pinned readers, and the canonical writer. |
| `crates/xt-server/`       | Guarded loopback HTTP/MCP endpoints and the headless core interface.                               |
| `crates/xt-metrics/`      | Metric definitions, SQL views, aggregation, and shared result DTOs.                                |
| `crates/xt-probes/`       | GitHub pull-request probes, shared executable lookup and bounded process-group runs.               |
| `crates/xt-rulebook/`     | Rule lifecycle, local proposals, verification, and fire history.                                   |
| `crates/xt-judge/`        | Headless grading runs by Claude Code, the rubric reader and the strict protocol v1 result reader.  |
| `crates/xt-fixtures/`     | Development-only fixture loading, database construction, and expected-result assertions.           |
| `xtask/`                  | Developer commands for shared fixture validation, database creation, and export.                   |
| `apps/desktop/src-tauri/` | Native lifecycle, Tauri commands, window chrome, tray, updater, and telemetry integration.         |
| `apps/desktop/ui/src/`    | React shell, UI components, screens, and the typed data-access boundary.                           |

All renderer code lives under the UI path above. Native lifecycle and window
behavior belong to the Tauri application.

The data flow is source observations → canonical ingestion/storage → shared
Rust metrics → native commands and renderer. Native history imports and the
optional loopback import adapter feed the shared canonical writer. JavaScript
consumes metric DTOs rather than reimplementing calculations. The fixture harness uses the canonical storage API;
its bounded baseline reference assertions do not implement the production metric
engine or establish whole-rule coverage.

## Design principles

### One rule per question (single source of truth)

Every question the app answers — "is this week ahead of even use?", "is this
session live?", "how long did this session run?", "how is this time shown?" —
is decided by exactly one function. Everything that shows or acts on the answer
(a label, a color, a warning, a chart, a sort order, a second screen) reads that
one result instead of working it out again with its own formula or threshold.
Derive, don't duplicate.

- Before adding a calculation, threshold, label rule or formatter, search for an
  existing one that answers the same question and reuse it. If it does not quite
  fit, change it in place rather than writing a near copy.
- Product decisions and metrics live in Rust and reach the renderer as DTO
  fields. The renderer formats and arranges them; it does not re-derive them.
  If the screen needs a new decision, add it to the DTO.
- When two signals describe the same state (for example a bar color and a
  warning line), derive both from the same comparison on the same unrounded
  numbers, and round only for display.
- A shared constant (a window length, a timeout, a cut-off) is defined once and
  imported where it is needed.
- Test the agreement, not only each part: a test should show that every signal
  derived from one answer changes together.

Example of the failure this prevents: the usage widget once colored its bar by
comparing use with even use over the whole week, while its "Runs out" warning
extended the last day's rate. A week below even use could show a green bar and
a run-out warning at the same time. Both now come from one comparison.

Keep subsystem contracts explicit at crate boundaries. Local checks are in
[CONTRIBUTING.md](../CONTRIBUTING.md); [CI.md](CI.md) describes automated
validation and its limits.

The monorepo directory layout uses [Cap](https://github.com/CapSoftware/Cap)
as a structural reference only.
No code or assets from that project are included in this workspace.
