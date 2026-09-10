# Canonical storage

`xt-store` owns the initial SQLite schema and the shared record writer. The
native application opens the store on startup; capture adapters do not yet ingest data into it.

## API

- `Store::open(path)` opens a file with WAL, foreign keys, FULL synchronous
  mode and a five-second busy timeout, then applies embedded migrations.
  The parent directory must exist. `Store::open_in_memory()` is for isolated
  single-connection tests and uses SQLite's MEMORY journal with NORMAL synchronous mode.
  File connections also enable `fullfsync` on macOS, requesting a storage barrier.
  These connection settings are applied on every open before migrations or writes.
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

In WAL mode, FULL synchronizes the WAL at each transaction commit. NORMAL can
lose a committed transaction after an OS crash or power failure; a capture client
could then have advanced beyond records that were not saved. File-backed writes
therefore use FULL before returning a successful result. Batching amortizes this
extra synchronization across an import. See SQLite's
[synchronous](https://www.sqlite.org/pragma.html#pragma_synchronous) and
[fullfsync](https://www.sqlite.org/pragma.html#pragma_fullfsync) documentation.

## Retention and replay

Pass the same `keep_content` value to both write methods. With `false`, new
session titles, content arrays and tool inputs are discarded; structural counts,
IDs, names and usage remain available. Enrichment cannot acquire content while
this mode is active. Previously saved content remains unchanged.

`retention_mode()` and `set_retention_mode()` share the persisted `content_retention`
setting. An absent setting defaults to metadata-only storage for metrics and indexing.
Full-content storage requires an explicitly saved opt-in. Older databases without
a saved mode also use metadata-only for future writes; their existing content is
left in place. An explicitly saved full-content preference is preserved. Metadata-only mode restricts
all canonical write entry points even if a caller requests content; explicit
`keep_content=false` remains restrictive in full-content mode. Each transaction
reads the policy after obtaining its write lock, so earlier-opened connections
cannot bypass a later setting change. Invalid stored values fail writes without
acquiring content. Changing the mode never calls purge.

`purge_content(&ContentRegistry)` is the separate deletion boundary. The default
inventory clears `sessions.title`, `records.content_json` (including embedded tool
results) and `tool_uses.input_json`. Compiled table owners register additional
nullable content fields through `register(table, columns)`; no SQL callback or
raw connection is exposed. All registrations are validated, then cleared in one
transaction. Only a committed result can request content-view invalidation.
IDs, counts, usage, source facts, receipt evidence and original host files survive.
The caller owns user confirmation; no Settings action is connected by this API.
This is logical database content removal, not erasure of backups or free pages.
Future content-bearing table owners must extend the inventory and their retention
writers; synthetic fire/judge tests demonstrate the registration boundary only.

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

## Atomic ingestion batches

`Store::apply_ingest_batch(&xt_store::batch::IngestBatch)` commits a session,
canonical records, supplied source observations, an optional supplied receipt and
an optional source cursor in one immediate transaction. `IngestBatch::new` takes
an explicit `keep_content` value for both session and record writes. The existing
merger and bounded record prefetch are shared with `upsert_records`. Every failure,
including final commit failure, rolls back all participating facts and returns
an error; the API exposes no connection, callback, event hook or acknowledgement.

Successful results contain one `RecordOutcome` per input with its input index,
UUID, disposition and sticky row conflict state immediately after that input.
Indices distinguish repeated UUIDs. `Inserted`, `Enriched` and `Duplicate` accept
the identity, even when known values conflict. `RejectedOwnership`, `RejectedType`
and `DroppedMissingUuid` do not. A later occurrence can add a conflict, so these
flags are neither the final batch state nor a per-field provenance mask. The
included `WriteStats` retains legacy per-input counters; `ignored` includes
identity/type rejections, and session metadata filling can count as enrichment.

Session observations and receipt parents must match the batch's canonical session.
Record observations and receipt coverage may reference only submitted UUIDs with
no rejected occurrence anywhere in the batch. This restriction avoids assigning
UUID-level evidence to an ambiguous accepted/rejected duplicate. Supplied masks
and digests remain the caller's observation; the store never computes them from
enriched canonical data. The default policy rejects sealed receipt ID reuse.
The ingestion writer may explicitly select exact-match replay and accepted-only
evidence. Matching occurs under the same immediate transaction and requires
identical parent facts and the complete unordered coverage set; mismatches roll
back canonical and cursor changes. Accepted-only mode omits UUIDs rejected by any
occurrence, and empty eligible coverage creates no receipt. Results explicitly
report whether receipt facts committed; the store itself returns no plugin ack.

An optional per-input `RecordIdentity` preserves native ancestry and first-seen
metadata. Command/interrupt/reminder prefix flags are derived before content
retention is applied; `is_human` remains unset for its owning rule. `measurement::Projection` defines the fixed versioned,
content-free field ordering shared with readers. Per-input measurement conflict
bits come from the merger's locked snapshot and only accumulate into source
fields actually reported by that observation. Session metadata changes are also
reported, so filling host/surface identity can invalidate grouped projections.

`source_cursor(source, key)` returns `None` for an unobserved source/key.
`SourceCursor` positions are nonnegative and cannot regress; an equal-position
retry is allowed and cannot lower `updated_at`. Different sources and keys are
independent. A truncated or replaced source generation needs a distinct key;
this API does not reset cursors. Empty batches may update session metadata and
cursors, but cannot manufacture receipt coverage. Effective retention combines the
persisted policy with the caller's restriction. Adapters publish product events
only after the returned transaction outcome.

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

`cargo test -p xt-store --test ingest_transaction` checks composed commits, all late
failure points including final commit, source/receipt eligibility, per-input
outcomes, cursor monotonicity and content retention. The existing 10,000-record
query-count regression also exercises the composed batch under SQLite's lowered
999-variable limit. See `docs/acceptance/atomic-ingestion.md` for setup and expected results.
