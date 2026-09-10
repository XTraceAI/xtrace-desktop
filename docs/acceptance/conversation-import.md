# Conversation import acceptance

This change connects the existing local MCP server to the shared parser, writer
and saved content policy. It does not change desktop screens or start the server
automatically from the native app.

## HTTP behavior

Run `cargo test -p xt-server import_conversation --locked`. Eight tests use real
loopback HTTP connections and disposable SQLite databases:

| Setup and action                                                                                                                     | Expected result                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Import two sparse UUIDs and a missing-UUID row; enrich one UUID; replay it sparsely.                                                 | Counts are 2 new/1 dropped, then 0 new/1 enriched, then 0/0. Every successful response has matching structured/text content and a durable acknowledgement. The first receipt stays immutable; later sparse coverage cannot inherit stored usage or omitted history.                         |
| Submit novel host/surface labels, native IDs, namespace, cloud routing inputs; retry with omitted surface and conflicting namespace. | Raw identity survives, unknown host maps to `other`, local scope remains local, namespace conflict preserves the first value. Receipt surface describes that submission; event surface matches storage. Legacy missing surfaces stay unknown. Known native/canonical contradictions reject. |
| Import and replay rich content with the saved metadata-only policy.                                                                  | Usage and namespace persist; title, transcript content and tool inputs stay absent; both durable receipts exist.                                                                                                                                                                            |
| Submit empty, missing-only, malformed, non-object and oversized record sets; inject a deferred foreign-key error at final COMMIT.    | No acknowledgement or event; all canonical, source, receipt and cursor tables match the prior snapshot. Removing the injected failure lets one retry commit.                                                                                                                                |
| Submit an owned UUID under a second session.                                                                                         | The original owner becomes conflicted and receives committed invalidation, but the rejected import has no receipt or acknowledgement.                                                                                                                                                       |
| Repeat nested/escaped JSON keys and submit a legitimate body larger than 64 KiB.                                                     | Duplicate keys fail before their earlier values disappear; the valid MCP body succeeds under the 16 MiB limit.                                                                                                                                                                              |
| Send concurrent sparse/rich copies of one UUID.                                                                                      | One stored record, one total insertion, measured usage preserved and two committed receipts.                                                                                                                                                                                                |

`cargo test -p xt-server --test transport --locked` additionally checks every
route's loopback/Origin/media-type guards, RPC IDs and errors, notification
non-mutation, the 16 MiB MCP cap, and the separate 64 KiB token-route cap.

## Real producer

Set `AGENT_PLUGINS_DIR` to the `plugins/memhub` directory in a checkout at
`fae871a5c3385d14e55ea64f6bda7421c52ebebb`, then run:

```sh
cargo test -p xt-server --test conformance --locked -- --nocapture
node scripts/ci/run-hook.mjs plugin-conformance
```

The direct Rust test prints an explicit skip reason if the plugin root or Python
is unavailable. The installed local/native gate requires both and fails instead
of skipping. No hosted workflow or job is added.

The harness snapshots pinned production code, supplies a temporary HOME and
synthetic transcript, and restricts the hook's network connections to its test
server. Only the copied `.mcp.json` changes for the config-route run. It does not
read installed credentials, modify installed plugins or import real activity.

Both routing mechanisms must produce identical canonical rows and these actual
cursor observations:

| Action                                   | Before → after byte offset | Durable receipts |
| ---------------------------------------- | -------------------------- | ---------------- |
| First import, including one missing UUID | 0 → 110                    | 1                |
| Richer observation of the same UUID      | 110 → 320                  | 2                |
| Duplicate-only observation               | 320 → 530                  | 3                |
| Forced database COMMIT failure           | 530 → 530                  | 3                |
| Retry after removing the failure         | 530 → 740                  | 4                |

Enrichment and duplicate replay report zero new records. Failure records a
producer error without advancing the cursor; successful retry clears it and
acknowledges the newly durable UUID. The final database passes `quick_check`.

The protocol and remaining producer limits are documented in
[the wire contract](../wire-contract.md). These focused cases do not claim that
multi-destination capture, other host producers, complete session-health fixtures
or production capture-verification consumers are finished.

Identity bounds apply after message metadata is resolved as well as to top-level
arguments. Oversized surface/native-ID labels through all four message aliases
fail without rows, receipts or events; 512-byte multibyte labels round-trip.
