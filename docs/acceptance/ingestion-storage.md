# Ingestion storage acceptance

The change extends the existing canonical SQLite store with structural source
observations, immutable submitted receipt coverage, native discovery and tool
events. Tests use synthetic fixtures and disposable files; the desktop app does
not import or display these facts yet.

## Setup, action and expected result

These cases run under `cargo test -p xt-store ingest_schema`. Their implementations
are in `crates/xt-store/tests/ingest_schema_cases/`.

| Case                                                                     | Setup and action                                                                                                                                          | Expected result                                                                                                                                                                                         |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `upgrade_real_base_schema`                                               | Create a real file with the unchanged 0001 SQL and F1-derived canonical rows, usage splits and tool IDs 41/99; upgrade and reopen twice.                  | Original columns and values remain, including a legal empty tool name and nullable input; exactly two migration versions, valid FKs and WAL; tool IDs and normalized usage round-trip.                  |
| `failed_upgrade_preserves_original_schema_and_tool_rows`                 | Add an invalid legacy orphan tool row, then open through the migration runner.                                                                            | Upgrade fails atomically; old tool rows/schema and version 1 remain, with no partial 0002 tables or columns.                                                                                            |
| `receipt_schema_keeps_distinct_facts`                                    | Load F18's named schema input, persist source observations and a receipt covering only one of two canonical records.                                      | Observations alone create no receipt. Native/canonical identities round-trip; coverage contains exactly the supplied UUID, one-bit mask, digest and version even when stored usage is richer.           |
| `invalid_or_duplicate_coverage_rolls_back_the_whole_receipt`             | Try empty/duplicate coverage, malformed digests (including embedded NULs), invalid masks/versions, missing UUIDs and a UUID belonging to another session. | Each attempt fails without leaving parent or child rows. A valid submission succeeds once; duplicate receipt identity cannot replace it.                                                                |
| `sealed_receipts_cannot_change_append_or_replace`                        | Seal a receipt, attempt SQL update/delete/replace/append and presealed insertion, then enrich its canonical record.                                       | Invalid mutations fail. The original receipt and submitted coverage remain unchanged after canonical enrichment.                                                                                        |
| `sql_digest_types_and_unsealed_visibility_are_enforced`                  | Stage an unsealed parent and try sealing without children or inserting fractional masks/versions and a binary digest.                                     | Invalid values fail; unsealed facts never appear through typed receipt readers.                                                                                                                         |
| `metadata_mode_keeps_structural_facts_without_new_content`               | Insert/replay F18 with retention disabled, add a hook event and submitted receipt, then inspect persistent columns.                                       | No new title/content/tool input is stored; counts and structural facts remain. The hook has no fabricated canonical row, duplicate identity fails, and structural DTOs reject extra raw-content fields. |
| `structural_event_time_is_native_nullable_and_separate_from_record_time` | Write an offset timestamp with 12 fractional digits, an unknown event time and malformed timestamps alongside canonical records.                          | Exact native spelling survives; unknown stays unknown despite known session start; invalid dates write nothing; canonical child rows keep their original record timestamps.                             |
| `f20_discovery_without_capture_stays_observable`                         | Load F20's named schema discovery inputs for CLI, Desktop, a new surface and unknown identity/time evidence.                                              | Four discoveries round-trip with honest unknown/incomplete fields, while canonical sessions, records and receipts remain empty.                                                                         |
| `discovery_preserves_identity_and_latest_completeness`                   | Fill absent discovery metadata, submit stale observations, a conflicting identity and a newer incomplete observation.                                     | Missing identity fills, conflicts fail, stale status cannot replace current status, and a newer probe can report incompleteness.                                                                        |
| `source_observations_accumulate_without_manufacturing_receipts`          | Replay overlapping observation intervals and presence/conflict masks; attempt invalid references and values.                                              | Intervals widen and supplied bits accumulate; invalid facts fail; no receipt is manufactured.                                                                                                           |
| `supplemental_schema_constraints_and_record_counts_survive_replay`       | Inspect supplemental tables, attempt invalid settings/cursors/host/PR/hygiene values, replay canonical rows and fail a mixed batch.                       | Constraints hold; replay and rollback preserve record counts; a later successful new record increments the count once.                                                                                  |

## Verification

Run the shared storage/fixture tests and the complete native validation on the
PR's exact source and base:

- `cargo test -p xt-store -p xt-fixtures -p xtask --locked --offline`
- `cargo test -p xt-store ingest_schema --locked --offline`: all 12 cases above
  execute, with no filtered-out acceptance cases.
- `cargo clippy -p xt-store -p xt-fixtures -p xtask --all-targets --locked --offline -- -D warnings`
- `cargo fmt --all -- --check` and `git diff --check`.
- `cargo --offline xtask fixture-validate`: expect 21 structurally valid entries;
  F1 is populated and asserted, and 20 remain skeleton/unimplemented.
- `pnpm fixtures:export` regenerates the shell fixture from the real store;
  its schema version is 2 and canonical counts remain 1 session / 25 records /
  15 usage rows. The committed export and native app startup tests must agree.
- `pnpm check:native --base FULL_REVIEWED_BASE_SHA`: workspace tests, strict
  Clippy, DTO parity, dependency checks and an isolated native app launch pass.

Record actual test counts, source/base identities and the native OS in the PR.
Use disposable databases; these checks do not require importing personal sessions.

F18/F20's named schema snapshots provide table-level evidence while their product
expectations remain empty. They do not claim complete ingestion, digest
generation, retry arbitration, capture verification, daily tool metrics or purge
acceptance. Those consumers still need executable product assertions. Receipt
digests here are synthetic format-valid values supplied to storage; the store
never computes coverage from enriched canonical data.
