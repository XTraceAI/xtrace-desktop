# Atomic ingestion storage acceptance evidence

The store now has one fixed transaction boundary for canonical records, supplied
source and receipt facts, and an optional source cursor. It returns indexed
outcomes only after commit. The desktop app and plugin delivery paths are not
wired to this boundary yet.

## Setup, action and expected result

The eight cases below run with `cargo test -p xt-store --test ingest_transaction`.
They use shared synthetic fixture F1 or disposable databases; the implementations
are in `crates/xt-store/tests/ingest_transaction_cases/`.

| Case                                                                            | Setup and action                                                                                                                                                                                               | Expected result, observed passing                                                                                                                                                                                               |
| ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `commits_supplied_facts_and_reopens_with_the_shared_fixture`                    | Apply F1's 25 records with source observations, a supplied receipt for one UUID and cursor 25; reopen, then replay the same sealed receipt ID.                                                                 | All 25 records, 15 usage rows, five tool rows, supplied facts and cursor persist together. Each input has an indexed inserted outcome. Receipt ID reuse fails with all participating tables unchanged.                          |
| `every_late_failure_including_commit_rolls_back_before_retry`                   | Start with an existing record/cursor, attempt enrichment plus a new record and evidence, and inject failures at session source, record source, coverage, cursor and final commit using a deferred foreign key. | All nine participating tables exactly match their pre-batch snapshots after each failure, including old content and receipt absence. Reopen sees the old cursor. Removing each failure allows the entire batch to succeed once. |
| `supplied_evidence_must_match_the_session_and_accepted_submission`              | Submit coverage or source observations for unsubmitted, absent or foreign UUIDs and mismatched parent sessions.                                                                                                | Each batch fails with no partial rows or persisted rejection conflict flag. Existing canonical membership alone cannot justify submitted evidence.                                                                              |
| `mixed_accepted_rejected_occurrences_cannot_supply_uuid_level_evidence`         | Submit the same UUID with accepted and rejected types in both orders, attaching source or receipt evidence separately; then submit accepted known-value conflict enrichment.                                   | Either mixed ordering rejects the evidence and rolls back all changes. Accepted canonical identity with conflicting known usage still preserves old input, fills missing output and commits supplied evidence.                  |
| `per_input_indices_disambiguate_duplicates_enrichment_conflicts_and_rejections` | Apply eight inputs including repeated UUID enrichment, known conflicts, a type rejection, foreign ownership and a missing UUID with otherwise invalid fields.                                                  | Input indices, dispositions, UUIDs and sequential sticky conflict states match exactly. Legacy counters and drop priority remain intact; same-batch cache state prevents overwrites and duplicate children.                     |
| `cursor_scope_monotonicity_and_empty_batches_are_explicit`                      | Update equal keys across distinct sources, retry an equal position with older observation time, attempt negative/regressing positions and submit empty batches.                                                | Sources/keys stay independent; cursor time does not regress; invalid positions roll back canonical changes. Empty metadata/cursor batches return no record outcomes and cannot create receipt coverage.                         |
| `explicit_retention_applies_to_fresh_and_enriched_batch_content`                | Apply retained and metadata-only batches to fresh, sparse and previously retained rows.                                                                                                                        | No new titles, content or tool inputs are acquired with retention off; old content remains and structural counts/usage, including measured zero, survive.                                                                       |

The additional `malformed_rejected_records_roll_back_the_entire_composed_batch`
case submits a valid new record followed by a malformed conflicting UUID, with
a new session title and cursor position in the same batch. Invalid platform,
structural evidence, timestamp and usage values are tested for both type and
ownership conflicts. Every attempt returns an error; all participating tables
and the old cursor remain unchanged. Conflict rejection cannot bypass validation.

The existing 10,000-record write regression additionally runs the composed batch
with SQLite's variable limit lowered to 999. It checks at most 64 SELECTs and
exactly 30,003 rows read for the composed replay, including the session metadata
and retention-policy reads, while an unrelated stored record stays outside the prefetch. The legacy
paths use at most 63 queries. Each transaction reads the retention policy once. This validates batching without a timing gate.

## Verification

Run the focused eight-case suite and the shared storage/parser/fixture tests:

```sh
cargo test -p xt-store --test ingest_transaction --locked --offline
cargo test -p xt-store -p xt-fixtures -p xt-ingest -p xtask --locked --offline
cargo clippy -p xt-store -p xt-fixtures -p xt-ingest -p xtask --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
cargo --offline xtask fixture-validate
```

The catalog remains 21 structurally valid entries, with F1 populated/asserted and
the other 20 skeleton/unimplemented. Full native validation and the exact tested
source and parser dependency are recorded in the pull request.

This is the transaction portion of ingestion. Supplied receipt digests are
synthetic format-valid values. The boundary does not generate digests or masks,
provide idempotent receipt retry/acknowledgement, emit product events, persist
retention policy, purge content or prove verified plugin capture. Those later
writer and retention responsibilities still need their own executable tests;
this storage boundary does not claim full capture acceptance or populate
skeleton product fixtures.
