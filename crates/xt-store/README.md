# Canonical storage

`xt-store` owns the initial SQLite schema and the shared record writer. The
native application does not ingest data or open this store yet.

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
