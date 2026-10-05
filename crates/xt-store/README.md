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
The response-usage view uses this same parser through a deterministic SQLite
comparator. `Store` registers it automatically; direct SQLite consumers must call
`xt_store::timestamp::register_sqlite(&connection)` before querying
`v_response_usage`. Registration supports read-only connections and writes no state.

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
unknown labels. The stored `started_at_ms` is only the host's own start and is
independent of the first imported event. Display reads (`session_list`) show a
Claude session with no stored start as starting at its earliest imported message,
because Claude Code transcripts record no start; nothing writes that back.

## Structural ingestion storage

Migration `0002_ingest.sql` extends the actual 0001 schema. Sessions gain
repository/namespace/branch-set metadata, a user/judge kind and a record count
maintained by canonical batch writes. Records gain nullable hygiene, parent,
agent, subtype and first-seen fields. Hosts, settings and source cursors reserve
storage for their owning adapters; those product workflows are not wired here.
The PR-link tables are written only through `xt_store::pr_link` (below). The normalized usage table and existing source/surface/API-ID
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

`IngestBatch::injected_context` optionally carries, aligned with the inputs, a
native Codex reader's proof that one input is context Codex injected (a selected
skill's instructions). Only a discovered native Codex batch may carry one, and it
is kept in `injected_context_inputs` only when that exact input inserted its
record in this transaction, once, unconflicted, as an eligible human-classified
user text input; otherwise it abstains and the records commit as without it.
Malformed, misbound or ambiguous proofs reject the batch. The outcome reports
one `InjectedContextOutcome` per proof. The Store's confirmation paths refuse an
input a proof holds; SQL itself only refuses a proof for an already confirmed
input, not a later raw confirmation row. No other API writes these rows; see
`docs/acceptance/confirmed-automated-inputs.md`.

`IngestBatch` also keeps, in its own transaction and whatever the retention
mode, a short preview in `record_previews` (see `record_preview.rs`): one line of
at most 280 characters for each accepted input whose whole text the stored
classification calls a person's, and each proven task notification's own
`<summary>`. `IngestBatch::withheld_previews` optionally marks, aligned with the
inputs, those a human-input adjustment says are only partly a person's words;
they keep none. Rows fill once; purge clears their text, and an applied
adjustment removes a person preview. `Store::record_preview` reads one back.

`IngestBatch::task_notifications` optionally carries, aligned with the inputs,
the native line's own marker that Claude Code wrote that input as a task
notification (`origin.kind`). An accepted marked input binds a row in
`task_notification_inputs` to its stored record — new or already held — when
that row is an unconflicted, human-classified Claude user input; otherwise it
abstains. Like a confirmation, the row overrides only that input's effective
human eligibility; the raw record is unchanged.

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
retention is applied. The same preparation path derives `is_human` from explicit
role, metadata/sidechain flags, tool-result presence, and trimmed joined text.
Missing role or content stays unknown unless another known fact excludes a human
message. `text_len` counts Unicode scalar values across text blocks without
separators, including whitespace, and excludes tool payloads. Human typing
consumers use this length only when `is_human` is true.

Existing unknown classifications are never inferred in SQL: they can be
enriched only by replaying sufficient original canonical input. Migration
`0005_human_classification_replay.sql` therefore deletes transcript rows from
`native_checkpoints` once, so the next native scan replays unchanged Claude files
through this preparation path and records new checkpoints. Reader-host
checkpoints, records, receipts, coverage and content are untouched, and later
opens do not repeat the reset. Old receipt masks and
measurement revisions remain immutable; later classification enrichment cannot
retroactively establish capture coverage. Existing raw command/interrupt/reminder
prefix flags retain their original untrimmed semantics.

`measurement::Projection` defines the fixed versioned,
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

## Pull-request evidence

`xt_store::pr_link` persists locally observed pull-request identities, their
session links and typed refresh results over the existing `pull_requests` and
`pr_links` tables. Migration 7 only clears transcript checkpoints so native
witnesses are replayed once. Migration 8 adds two nullable refresh-status
columns to `pull_requests` and changes no existing row or checkpoint.

- `PrIdentity` accepts only `https://github.com/<owner>/<repo>/pull/<number>`,
  at most 256 bytes. Scheme and host compare ASCII case-insensitively. Owners are
  1-39 ASCII letters, digits or hyphens (not leading); repositories are 1-100
  ASCII letters, digits, `.`, `_` or `-`, excluding `.`, `..` and a `.git` suffix.
  Numbers are canonical decimal integers from 1 to `i64::MAX`. Userinfo, ports,
  queries, fragments, trailing slashes, extra or missing segments, percent escapes,
  leading zeroes, controls, whitespace, non-ASCII text, other hosts and schemes,
  and GitHub Enterprise hosts are rejected rather than normalized.
- GitHub resolves owner and repository names case-insensitively, so the canonical
  identity is the ASCII-lowercase `owner/repo` plus the number, with exactly one
  canonical URL. Distinct pull requests never collapse into one identity.
  `PrIdentity::reconcile(url, repository, number)` requires a URL or both other
  parts; every supplied part must name the same canonical identity.
- `PrConfidence` is `exact`, `sha` or `inferred`. A link upgrades only toward
  `exact > sha > inferred` and never downgrades.
- `Store::record_pr_link(&PrLinkObservation)` writes one stub per canonical
  repository/number and one link per session/pull request in one immediate
  transaction. The canonical session must already exist. The link keeps the
  earliest `first_seen_at`, the latest `last_seen_at` and the strongest confidence,
  so duplicate and reversed arrivals converge; an identical replay writes nothing.
  An existing row that matches the repository/number or URL ASCII
  case-insensitively but is not spelled exactly canonically is a conflict. Every failure rolls back the stub and link together.
- A link records a reference only. It never implies a merge: `title`, `state`,
  `merged_at`, `additions`, `deletions`, `head_ref_name`, `refreshed_at`,
  `last_attempted_at` and `refresh_error` stay unknown until a refresh result is
  recorded. Observations carry no title, branch, path, command, tool argument or
  transcript text.
- `Store::record_pr_refresh(&RefreshOutcome)` persists one typed attempt,
  `RefreshSuccess` or `RefreshFailure`, for an identity that already has a
  canonical row, in one immediate transaction. It never creates a stub or link and
  never changes confidence or first/last-seen times; an unknown identity or a row
  the shared canonical lookup reports as a conflict fails. The whole result is
  validated before the transaction: `attempted_at` is UTC milliseconds from 0 to
  `MAX_ATTEMPTED_AT`; title (at most 1024 bytes) and head branch (at most 255
  bytes) are nonblank and free of control characters; `state` is
  `OPEN`/`CLOSED`/`MERGED`; additions and deletions are nonnegative; `MERGED`
  requires an RFC3339 `merged_at` of at most 64 bytes and `OPEN`/`CLOSED` forbid
  one. Every refresh-owned field of a success is required.
- Refresh columns: `refreshed_at` is the last successful refresh,
  `last_attempted_at` the newest applied attempt and `refresh_error` its typed
  failure code (`unavailable`, `timeout`, `cancelled`, `output_too_large`,
  `invalid_response`, `execution_failed`, `not_found`, `unauthorized`,
  `rate_limited`), enforced by a schema CHECK. No message, stderr, response body
  or stored stale flag exists. A newer success replaces `title`, `state`,
  `merged_at`, `additions`, `deletions` and `head_ref_name`, sets
  `refreshed_at` and `last_attempted_at` to the attempt and clears the error. A
  newer failure sets only `last_attempted_at` and the error, keeping the last
  successful metadata and `refreshed_at`. An attempt older than the stored one
  (or than a pre-8 `refreshed_at`) returns `RefreshWrite::Stale` and writes
  nothing. At an equal attempt time an exactly identical result is
  `RefreshWrite::Unchanged`; any differing success or failure is a conflict error
  and nothing is written.
- `pull_request(&identity)`, `all_pull_requests()` (by repository then number),
  `session_pr_links(session)` (by repository then number),
  `pull_request_links(&identity)` (by session) and `all_pr_links()` (by
  repository, number, session) are the read surface. `StoredPullRequest` exposes
  every refresh column and `refresh_status()` derives `NeverAttempted`,
  `Refreshed`, `FailedNeverRefreshed(code)` or `FailedAfterRefresh(code)` without
  any age or staleness policy. The two identity readers
  resolve rows with the writer's candidate lookup: no candidate is absent, one
  exactly canonical row is returned, and a case variant, URL alias, split
  identity or several candidates is an error. A non-canonical stored identity is
  reported rather than re-spelled; reads never rewrite legacy rows.

`IngestBatch::pr_links` carries exact native witnesses into the ingest batch's
immediate transaction, where they commit through the same link write as the
records and checkpoint they sit beside, or fail with them. Only discovered
Claude history may carry them. A link naming another session (a fork's
inherited copy) attaches only to an indexed Claude session and otherwise
nowhere. Scanning belongs to `xt-ingest`; see
[native PR witnesses](../../docs/acceptance/native-pr-witnesses.md). Fetching
refresh results (any GitHub client, subprocess or network call), scheduling,
staleness policy and PR metrics belong to later owners.

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
