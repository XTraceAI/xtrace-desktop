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
are recorded in `docs/acceptance/ING-02.md`.
