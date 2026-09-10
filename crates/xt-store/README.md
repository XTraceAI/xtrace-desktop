# Canonical storage

`xt-store` owns the initial SQLite schema and the shared record writer. The
native application opens the store on startup; capture adapters do not yet ingest data into it.

## API

- `Store::open(path)` opens a file with WAL, foreign keys, NORMAL synchronous
  mode and a five-second busy timeout, then applies embedded migrations.
  The parent directory must exist. `Store::open_in_memory()` is for isolated
  single-connection tests and uses SQLite's MEMORY journal.
- `upsert_session(&SessionMeta, keep_content)` creates the canonical conversation
  ID unchanged or fills missing session metadata.
  `SessionMeta::new(id, raw_platform, source)` maps `claude`, `codex` and `cursor` to known hosts;
  every other platform maps to `other` and retains its original value.
- `upsert_records(session_id, &[CanonicalRecord], keep_content)` commits one
  atomic batch. Its result partitions input rows into `inserted`, `enriched`,
  `ignored` and `dropped_no_uuid`. Empty/absent UUIDs are omitted. Other malformed
  input fails the whole transaction. The session must already exist.
- `session(id)`, `records(session_id)` and `counts()` return typed stored data.
  Records, their usage and their tool rows use set-based reads from one snapshot,
  ordered by native timestamp with unknown timestamps last and UUID as a tie
  breaker. Query count remains bounded as the number of session records grows.

Native timestamp ordering, imported first/last bounds and replay conflict checks
preserve every RFC3339 fractional digit, including digits beyond nanoseconds.
Equivalent offsets and trailing fractional zeroes describe the same instant.
`ts_ms` remains a coarse POSIX millisecond projection for indexing; callers must
not use it to break chronological ties. Range writes compare precise native
instants within indexed endpoint buckets, including the leap-second overlap.

Usage counters, model, API/request IDs and timestamps remain nullable. A missing
content array is unknown; an explicit empty array measures zero text and tools.
Text length counts Unicode scalar values in text blocks, excluding tool output.
Both camel-case wire keys (`gitBranch`, `requestId`, `isMeta`, `isSidechain`) and
canonical `git_branch`/`request_id` aliases are supported. Meta/sidechain flags
have the canonical default `false` and remain unchanged on conflicting replay.

## Retention and replay

Pass the same `keep_content` value to both write methods. With `false`, new
session titles, content arrays and tool inputs are discarded; structural counts,
IDs, names and usage remain available. Enrichment cannot acquire content while
this mode is active. Previously saved content remains unchanged. A separately
confirmed purge is not implemented here.

A UUID never moves between sessions or changes record type. Each nullable field
can fill once; later conflicting values preserve the saved value and set a
sticky `has_conflict` flag without storing the conflicting payload. Retained
content arrays and their tool-input projections form one observation: a conflicting
array cannot supply newly retained tool input. Equivalent RFC3339 offsets compare
as the same instant while the original timestamp spelling is retained. Session
`source` keeps its first compatibility summary and is not proof of capture.
Distinct UUIDs sharing API-response IDs remain separate; response-level usage
selection belongs to the metric projection.

`SurfaceEvidence` accepts only `{ "source": "adapter-name", "version": "1.0" }`
with optional version. Both values are bounded structural labels containing
ASCII letters/digits or `._-`; additional fields and path/content values are
rejected. Raw platform and surface values are separate fields and preserve
unknown labels. Native `started_at_ms` is independent of the first imported event.

## Structural ingestion storage

Migration `0002_ingest.sql` extends the actual 0001 schema. Sessions gain
repository/namespace/branch-set metadata, a user/judge kind and a record count
maintained by canonical batch writes. Records gain nullable hygiene, parent,
agent, subtype and first-seen fields. Hosts, settings, source cursors and PR-link
tables reserve storage for their owning adapters; those product workflows are
not wired here. The normalized usage table and existing source/surface/API-ID
columns remain in place.

`xt_store::ingest` exposes typed structural facts through `Store`:

- `observe_session_source` accumulates a first/last observation interval;
  `observe_record_source` accumulates explicitly supplied presence/conflict bits.
  The matching readers keep source observations independent from receipts.
- `insert_capture_receipt(&receipt, &[coverage])` atomically inserts and seals a
  nonempty submitted set. Every covered UUID must exist in the same canonical
  session as the receipt. Duplicate IDs/UUIDs and invalid fields roll back the
  entire operation. `capture_receipts` and `capture_coverage` expose sealed sets.
  Stored sets cannot be edited, appended to or replaced by later observations.
- Coverage stores a nonnegative SQLite integer mask, the exact supplied
  `measurement_revision` (64 lowercase SHA-256 hexadecimal characters) and a
  positive `digest_schema_version`. It never derives missing fields from a richer
  canonical row. Digest generation, delivery retries and capture verification
  belong to ingestion and metric consumers; storage alone makes no verified
  capture claim.
- `observe_discovered_session` preserves native identity and raw surface even
  before a canonical import. Missing values stay unknown; conflicting known
  identity fails, and an older observation cannot replace current completeness.
  `discovered_sessions(host)` does not imply a plugin receipt.
- `insert_tool_event` stores a classified structural hook/event without a
  fabricated canonical record. Its shape contains names and source identity but
  no input, output, command or transcript payload. `tool_events` reads these
  events independently from canonical record tool calls. Optional `timestamp`
  is validated as RFC3339 and stored verbatim in `event_ts`, including its native
  offset and full fractional precision. Missing event time stays unknown even
  when session or receipt times exist. Canonical tool calls use their existing
  record timestamp through the record FK instead.

The tool table is rebuilt transactionally only to allow a missing record FK for
structural events. Existing IDs, block indexes, names and inputs are preserved.
Other tables use additive changes. The store connection remains private; adapters
use these typed methods instead of issuing independent SQL writes.

## Verification

Run `cargo test -p xt-store` and
`cargo clippy -p xt-store --all-targets --locked -- -D warnings`.
Tests use disposable file databases for WAL/readers/migrations/concurrent writes
and isolated memory databases for wire, replay and nullable-field cases. They
inspect each content-bearing column during metadata-only inserts and enrichment.

The [storage acceptance contract](../../docs/acceptance/storage.md) specifies setup,
actions and expected results, including migration/reopen behavior and the
instrumented query-count regressions. Current PR verification records the tested
source/base, commands and actual results.

`cargo test -p xt-store ingest_schema` exercises an actual 0001 file upgrade,
two reopens, row/column/FK preservation and rollback on invalid legacy relations.
F18/F20 named schema snapshots exercise receipt immutability, submitted masks,
discovery without capture and metadata-only writes through the shared fixture
harness. These table-level checks do not mark either product fixture populated.
