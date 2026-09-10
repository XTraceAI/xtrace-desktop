# Ingestion writer acceptance

This local implementation composes parsed canonical records, incoming measurement
coverage and receipt acknowledgement through one storage transaction. The tests below cover the typed writer; adapters connect it to live imports.

## Setup, action and expected result

`cargo test -p xt-ingest --test writer --locked --offline` runs fourteen tests using
synthetic records and shared disposable fixture databases.

| Case                    | Setup and action                                                                                                                              | Expected result                                                                                                                                                                                                                                                  |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source order            | Import sparse plugin and rich native observations in both orders.                                                                             | One identical canonical record, native ancestry, measured usage and content-free fields; enrichment inserts no new UUID.                                                                                                                                         |
| Late/full receipt       | Backfill two UUIDs, capture only the late UUID, retry, enrich its usage, then capture the current full set.                                   | Partial receipt stays immutable, duplicate-only ack advances, old evidence fails current-field comparison, full submitted evidence matches. Independently read usage/model inputs change and both invalidation flags fire.                                       |
| Transaction failures    | Reuse an ID with mismatched evidence, and separately inject a deferred-FK failure at final commit during enrichment/new record/cursor writes. | No outcome/ack escapes; prior rows/receipt/cursor remain; removing the injected failure permits one successful retry.                                                                                                                                            |
| Unique outcomes         | Submit repeated compatible UUIDs, a known usage conflict and a foreign UUID beside a valid UUID.                                              | New/enriched counters count each UUID once; known usage stays, its exact conflict bit is recorded; foreign UUID gains no receipt evidence and ack names the valid input.                                                                                         |
| Projection              | Compare equal-length changed text/tool input, equivalent offsets, fractional tails, missing usage and measured zero.                          | Raw text and tool inputs are not serialized or hashed; derived lengths/prefix facts can change the digest. Equivalent instants match; precise time/measurement changes differ. The sparse projection has mask 199 and an independently specified SHA-256 golden. |
| Identity/chunk boundary | Resolve a native ID, preserve an unknown host/surface, reject conflicting identities and more than 2,000 records.                             | Known canonical prefix is applied once; unknowns stay unknown; successful native cursor progress is durable and has no plugin ack.                                                                                                                               |
| Concurrent retry        | Two independent stores start the same receipt concurrently.                                                                                   | One canonical insert and one immutable receipt; both successful calls acknowledge the same durable UUID.                                                                                                                                                         |
| Empty/rejected input    | Submit empty, foreign/type-rejected, contradictory duplicate and missing-UUID rows.                                                           | No empty/rejected coverage or ack; contradictory duplicate measurements fail; missing UUID is counted before invalid row metadata.                                                                                                                               |
| Shared F1/chunks        | Replay all 25 shared F1 rows, then submit 2,501 native rows in two bounded chunks.                                                            | Replay inserts zero; role-sensitive human classification stays unset; raw platform remains absent independently of declared host; final cursor is 2,501.                                                                                                         |
| Hygiene                 | Import human, command, interrupted, reminder, empty and unknown-content inputs with retention disabled.                                       | Structural flags match their inputs; transcript content stays absent and human classification stays unset even with absent or mismatched roles.                                                                                                                  |

The additional session-conflict regression replays the same native measurement
with a different known working directory. The stored directory remains unchanged,
its session conflict flag becomes true, and both invalidation flags are returned
with zero new/enriched records. Duplicate-only committed receipts also invalidate
capture measurements, including exact retries, without requiring row changes.

The prior 10,000-record query-bound test remains unchanged and passes. The SHA-256
schema uses `[version, fixed-order-values]`, with null for absent values and a
normalized `[UTC second, leap-second flag, decimal fraction]` timestamp. The sparse
28-field golden is `0c299a8f1b8c32e9d61ebc26bedf0da9ac99dad88e889cce513f15698afcc795`.
All strings in tests are newly synthetic; no native transcript or live user store
is read. F1 remains the only populated product fixture.

## Commit and replay boundaries

The original-owner regression first creates a plugin receipt and warms a test
consumer from separately read stored rows. A foreign session then submits that
UUID twice. An injected cursor failure rolls back the owner conflict and incoming
session, returning no events. After removing the failure, the original owner gets
one post-commit invalidation with its own surface and no foreign cursor. Refreshing
the consumer rejects the formerly matching receipt; the receipt itself is intact.

Receipt replay checks bind the original timestamp, session, surface, complete UUID
set and measurement revisions. Changing any one fails without altering rows or
receipt coverage. Reusing a sealed ID with empty, missing-UUID or wholly rejected input also
rolls back session changes, conflict flags and cursor advancement.
Reordering the identical set succeeds and acknowledges the last
accepted UUID in that request's order. Concurrency is covered with two independent
store connections delivering one exact receipt simultaneously.

## Scope

The entry point accepts parsed record envelopes. Countable parser-drop variants,
structural hook events and PR observations keep their separate adapter consumers.
Namespace/repository import metadata, tool-kind classification and named native
event publication are connected by those consumers. Returned change values cover
post-commit invalidation and backfill progress, including original record owners.

Shared F1 is replayed here. The broader F2/F18/F20 catalog entries remain skeletons
until their full concurrency, surface-health and capture-verification consumers
exist; focused synthetic writer cases do not declare those catalog contracts done.

No HTTP/native adapter, background reader, persistent retention policy, content
purge or production metric/cost engine is included. Record-field comparison alone
does not establish complete plugin capture verification: consumers must also check
stored session identity/conflicts and selected usage representatives.
