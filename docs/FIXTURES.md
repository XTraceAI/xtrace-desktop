# Canonical fixtures

The fixture harness gives development tests one loader for synthetic canonical
records, explicit clocks and file-backed SQLite. `xt-fixtures` is a development
dependency; product crates must not depend on it by default. The desktop does not
load these files.

The catalog contains 21 structurally valid entries: **F1 is populated and has
executable reference assertions; F2–F21 are skeletons with unimplemented rule
acceptance.** A successful structural report does not establish product metric,
privacy, ingestion, probe or rulebook acceptance.

## Commands

Run from the workspace:

```sh
cargo test -p xt-fixtures
cargo test -p xtask
cargo xtask fixture-validate
cargo xtask fixture-db F1 --out /tmp/xtrace-fnd04-F1.sqlite
cargo xtask fixture-export F1
cargo xtask fixture-export F16 --out /tmp/xtrace-F16.json
```

All commands accept `--catalog PATH` for an alternate catalog. IDs are `F1`
through `F21`; `F01` is invalid. Output files must not already exist. Database
output also refuses existing SQLite sidecars, is fully checkpointed before
materialization, and can be opened without its temporary WAL. Remove only your
own previous output before repeating an export.

`fixture-validate` loads all 21 entries, runs the implemented F1 reference checks
and reports every skeleton as `acceptance UNIMPLEMENTED`. A malformed entry or
failed assertion returns a nonzero exit. `fixture-db` refuses skeletons.
`fixture-export` permits structural skeleton exports and preserves their status
and empty expectations.

Exports contain the same canonical Rust records used by the loader and database
builder. Generated IPC DTO parity is a later IPC-boundary responsibility; this
command does not claim that transport contract exists yet.

## Manifest and inputs

Each directory contains `manifest.json`, `expected.json` and every declared
input. No input glob is used. For example, a session may point to
`input/custom/native.events` instead of `input/sessions/session.jsonl`.

```json
{
  "id": "F1",
  "status": "populated",
  "author": "XTrace fixture maintainers",
  "shape": "One synthetic Claude session with revised usage observations.",
  "proves": ["M-02", "M-03", "M-04", "M-05", "M-10"],
  "now": "2026-09-08T00:00:00Z",
  "window_days": 7,
  "sessions": [
    {
      "file": "input/sessions/session.jsonl",
      "session_id": "00000000-0000-4000-8000-000000000001",
      "host": "claude",
      "source_surface": "cli",
      "native_session_id": "00000000-0000-4000-8000-000000000001",
      "started_at": "2026-09-07T12:00:00Z"
    }
  ]
}
```

- `status` is `populated` or `skeleton`. `author`, `shape`, `proves`, `now`,
  positive `window_days` and at least one declared session are required.
- `now` and any supplied `started_at` or record `timestamp` must be RFC3339.
  The clock window is `[now - window_days, now)`; the loader never reads wall time.
  Native record timestamps may be absent, preserving unknown timing.
- `session_id` is a nonempty canonical identity, not a newly generated fixture
  UUID. Host-prefixed canonical IDs are supported. Record UUIDs must parse as
  UUIDs; repeated valid UUIDs remain inputs to the canonical storage boundary.
- `host` is `claude`, `codex`, `cursor` or `other`. Optional `source_platform`
  preserves unknown raw platforms and must agree with the known-host mapping.
  Omission preserves an unknown raw platform independently of the declared host;
  exports and databases never infer a raw observation from the host name.
  `source_surface`, `native_session_id` and `started_at` are optional.
- Each nonblank JSONL line deserializes into `xt_store::CanonicalRecord`.
  The harness passes session metadata and records through `Store` with an
  explicit `keep_content` flag. Store validation and UUID merging still apply.
- Optional `gh` names a JSON stub. Optional `snapshots` maps names to JSON paths;
  F16 reserves `fresh`, `typical`, `connected`, `no-python3` and `env`.
- All paths are relative to the fixture directory. Missing files, traversal,
  absolute paths and symlinks escaping the fixture fail. Diagnostics identify
  the file and JSON/JSONL location, such as
  `input/sessions/session.jsonl:1:uuid: invalid UUID`.

`expected.json` maps defined rule IDs to JSON values. The registry includes
M-01…M-19, M-11a, M-12a, C-01…C-08, O-01…O-13, P-01/P-02, R-01…R-08 and
U-01…U-08: 60 keys. Unknown keys fail in either `proves` or expectations.

For populated inputs, expected keys must exactly match `proves`. A JSON `null`
means an unmeasured value and differs from `0`. Skeletons have empty canonical
input files and `{}` expectations; their `proves` list is planned coverage.
Skeletons cannot use null to stand in for unimplemented assertions.

## F1 oracle and its limits

F1 is entirely synthetic: one Claude session, one deliberately unpriced model,
five human requests, five tool calls and five tool-result carriers. Each human
request has three assistant records but one assistant turn. Twenty-five canonical
rows and all fifteen usage observations remain stored.

Per human request, the first API response has two cumulative observations:

| Observation                                | Input | Output | Cache read | Cache creation | Total |
| ------------------------------------------ | ----: | -----: | ---------: | -------------: | ----: |
| Initial text                               |   100 |      5 |         20 |             10 |   135 |
| Later tool call, same response/request IDs |   100 |     10 |         20 |             10 |   140 |
| Second API response                        |    50 |     20 |         10 |              0 |    80 |

Select the later observation of the first response, then add the second:
`5 × (140 + 80) = 1,100` tokens. Counter totals are 750 input, 150 output,
150 cache read and 50 cache creation across ten selected responses. Summing all
fifteen stored observations would incorrectly produce 1,775 tokens.

The first event is 12:00 and the last 12:23. No gap exceeds twenty minutes, so
activity is 23 minutes (1,380,000 milliseconds). The sole model has 150 selected
output tokens and is the favorite. Literal goldens encode these independent
calculations; reference assertions derive their observations from stored rows.
Tests alter both an input counter and a golden total to prove each mismatch fails.

These checks exercise F1's baseline shapes for M-02/M-03/M-04/M-05/M-10. They do
not implement the production metric engine or establish whole-rule coverage.
Prices, unknown counters, tie-breaking by native sequence, other host adapters,
window boundaries and other edge cases require their owning fixtures and product
tests. The bounded reference checker rejects unsupported F1 shapes.

## Ownership and test loop

`xt-fixtures` owns the sole `Fixture::load/all` loader, typed canonical inputs,
fixed clocks, auxiliary stubs, exact expectation comparison and database owner.
`xtask` invokes that API. Reader/server/metric tests must reuse it instead of
opening JSON with their own fixture loader.

```rust
use xt_fixtures::Fixture;

fn example() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::load("fixtures/F1")?;
    let database = fixture.build_db(true)?;
    let reader = xt_store::Store::open(database.path())?;
    assert_eq!(reader.counts()?.records, 25);
    // Keep `database` alive until all readers, servers and writers have stopped.
    drop(reader);
    drop(database);
    Ok(())
}
```

`TempDb` owns both a real SQLite file and its Store/TempDir. `store_mut()` supports
reader/replay test setup without bypassing the canonical write API. Build with
`false` to test metadata retention explicitly; this changes future fixture writes
and is not a purge operation. WAL tests keep a reader snapshot open during a
writer commit, then verify a new read sees the committed row.

`TempDb::empty()` provides the same file owner for schema/discovery tests before
their product fixture is populated. F18 and F20 have named `schema` snapshots
loaded through `Fixture::snapshots()` for `cargo test -p xt-store ingest_schema`.
F18 checks canonical/native identity, exact immutable submitted coverage and
metadata-only structural writes. F20 checks discovery rows without canonical
imports or receipts, including new and unknown surfaces. Their synthetic digest
values test storage constraints, not digest generation or capture verification.
Both entries remain skeletons with empty product expectations; the owning
ingestion and metric PRs must still provide their executable product assertions.

The harness owns catalog structure. The first subsystem PR populating a fixture
owns its inputs; later PRs coordinate additions to that fixture's expected keys
or named variants rather than replacing earlier evidence. Start with synthetic
input and independently calculated expectations, write a failing assertion
against the owning product API, implement the behavior, then run its tests and
catalog validation. Add executable reference dispatch before marking another
entry populated for `fixture-validate`; fail honestly while that dispatch is
missing. Never copy live-machine transcripts or totals into a fixture.

| ID  | Current status                | Planned input responsibility                                    |
| --- | ----------------------------- | --------------------------------------------------------------- |
| F1  | Populated, reference asserted | Baseline human/assistant turns, response usage, activity, model |
| F2  | Skeleton                      | Overlapping activity and concurrency                            |
| F3  | Skeleton                      | Event window boundaries                                         |
| F4  | Skeleton                      | Exact pull-request links and merged-PR statistics               |
| F5  | Skeleton                      | Codex cumulative usage reader                                   |
| F6  | Skeleton                      | Cursor IDE hook usage attachment                                |
| F7  | Skeleton                      | Cursor Agent CLI unmeasured usage                               |
| F8  | Skeleton                      | Human-message exclusions                                        |
| F9  | Skeleton                      | Batched timestamp latency                                       |
| F10 | Skeleton                      | Aborted turn without counters                                   |
| F11 | Skeleton                      | Weekly model comparisons                                        |
| F12 | Skeleton                      | Inferred branch links                                           |
| F13 | Skeleton                      | Directive coverage                                              |
| F14 | Skeleton                      | Rule-fire deduplication                                         |
| F15 | Skeleton                      | Enforcement downgrade                                           |
| F16 | Skeleton                      | Named filesystem/environment probe snapshots                    |
| F17 | Skeleton                      | Claude response revisions, copies and missing IDs               |
| F18 | Skeleton                      | Source replay, enrichment, retention and purge variants         |
| F19 | Skeleton                      | Overlapping pull-request attribution                            |
| F20 | Skeleton                      | Surface identity and capture coverage                           |
| F21 | Skeleton                      | Initial scan/tail handoff race                                  |

Purge variants must eventually cover previously stored transcript/tool/fire/judge
content, confirmed removal, unchanged metadata/counts and source files, and a
metadata-only retry that cannot refill removed content. F18's skeleton does not
claim those storage/ingestion/rulebook operations are implemented.
