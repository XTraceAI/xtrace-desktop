# Canonical parsing

`canonical::parse_line(&str)` and `parse_with_context(&str, &SourceContext)` are
pure functions. They do not open files, read native databases, write SQLite,
create PR links or generate capture receipts.

`Parsed` distinguishes canonical records, native stop-hook summaries, native
PR-link observations, inert unknown types and countable missing-UUID drops.
Recognized malformed inputs return fixed `ParseError` categories that contain no
source text, identifier or arbitrary JSON value.

Records contain the shared `xt_store::CanonicalRecord` inside a typed envelope:

- `native` preserves `sessionId`, `agentId`, `parentUuid`, subtype and entrypoint.
- `source` preserves identity fields supplied on the line; `context` separately
  preserves caller-observed import/header metadata. A conflict remains visible
  for the later writer instead of silently overwriting either observation.
- Native string content becomes one canonical text block. Missing message
  content/usage/model remains unknown, and an empty string remains measured.
  Missing user/assistant UUIDs are dropped before validating other row fields.
- Missing `api_message_id` can use the observed `message.id`; missing raw surface
  can use the observed entrypoint; missing native session ID can use the observed
  native `sessionId`. Existing explicitly supplied values remain unchanged.
  Header start time is never substituted for an event timestamp.
- Tool-use count and tool-result-carrier presence are derived while content is
  in memory. Other hygiene and tool-kind classification remain later work. The
  parser itself retains canonical content for that subsequent classification;
  the retention policy belongs to the writer.

Stop-hook summaries preserve a structural subset, including native identity,
timestamp and supplied counts/duration. Raw hook commands, errors, stop reason
and additional context are discarded. One summary is one event regardless of
its `hookCount`; this parser does not calculate daily hook metrics.

Native `pr-link` observations have no UUID. Their positive PR number, raw URL,
repository, session and optional time are parsed independently from canonical
records. URL normalization, link validation and persistence remain the owning
PR-link consumer's responsibilities.

Run `cargo test -p xt-ingest canonical` and the explicit release performance gate
`cargo test -p xt-ingest --release canonical_parse_50k -- --nocapture`.
Producer provenance, input variants, exact dataset hashes and measured evidence
are recorded in `docs/acceptance/canonical-parser.md`.

## Ingestion writer

`writer::write_batch(&mut Store, &WriteBatch)` accepts the existing parsed record
envelopes, an authoritative source context, explicit content retention and optional
native cursor. It resolves canonical/native identity and preserves a known adapter
host independently from an absent raw platform. Conflicting known identities fail.
Native ancestry and structural prefix facts survive content discard.

Plugin batches require stable receipt parent facts; other sources cannot supply
receipts. The writer computes a versioned SHA-256 measurement revision and field
mask from incoming records before merging. Compatible duplicate UUID observations
union only their submitted measurements; contradictory measurements cannot define
one receipt. No title, transcript text or tool input/output enters the projection.
Timestamp encoding preserves full precision and equivalent offsets using tuples;
it is independent of JSON object-map feature ordering.

The store's single transaction filters rejected identities from evidence, inserts
or exactly matches immutable receipt facts, and advances an optional cursor.
Receipt ID retries must reuse the original parent time and complete submitted set.
`BatchOutcome` and its typed change events are returned only after commit.
Acknowledgement exists only when receipt facts committed and names the last
accepted eligible input UUID. Duplicate-only imports can acknowledge; empty or
fully rejected sets cannot manufacture receipt coverage.

New/enriched counters count each accepted UUID once, with insertion taking
precedence over enrichment within one call. Dropped counts include each missing
UUID input and each wholly rejected UUID once; indexed reasons retain individual
rejections. A batch is bounded to 2,000 records; native adapters submit explicit
cursor positions with successive chunks. Event values describe invalidation and
backfill progress; adapters remain responsible for actual publication.

`coverage` and `matches_current` provide a record-level measurement comparison.
They are not a complete session verification policy: identity, conflicts and
selected response representatives still require their owning consumer. Parser
stop-hook/PR-link variants retain their dedicated later consumers; this method
accepts `ParsedRecord`, not arbitrary raw event payloads. Namespace/repository
session extensions, full tool-kind classification and named native event routing
belong to their adapter and classification consumers.

Run `cargo test -p xt-ingest --test writer --locked --offline`. Exact test evidence
and remaining work are recorded in `docs/acceptance/ingestion-writer.md`.
