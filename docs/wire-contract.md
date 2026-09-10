# Local conversation import

The headless server accepts `POST /mcp-server/mcp` on its bound loopback port.
Send one JSON-RPC 2.0 `tools/call` request with `name: "import_conversation"`.
The existing Host, optional Origin and JSON content-type checks apply before the
handler. Request bodies are limited to 16 MiB on this route; token routes retain
their 64 KiB limit. Duplicate JSON object keys, including nested or escaped
equivalents, are rejected before deserialization can erase them.

## Arguments

| Field                                    | Meaning                                                                                                                                |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `messages`                               | Required array of 1–2,000 canonical message objects. The caller must split larger batches.                                             |
| `conversation_id`                        | Required canonical session ID chosen before the first attempt and reused on retries; echoed unchanged.                                 |
| `source_platform`                        | Required raw host label. Known labels map to Claude, Codex or Cursor; novel labels remain stored and map to `other`.                   |
| `source_surface`, `native_session_id`    | Optional observed identity. Absence stays unknown. Known native IDs must agree with the canonical host prefix.                         |
| `flush`                                  | Optional `auto`, `now` or `defer`. Local persistence completes before any successful response in all three modes.                      |
| `namespace`                              | Optional local session namespace, filled monotonically. A later disagreement preserves the saved value and records a session conflict. |
| `title`                                  | Optional transcript-derived title, subject to the saved content policy.                                                                |
| `provenance`, `agent_brain_id`, `org_id` | Accepted for compatibility. They do not route local data to a cloud account; PR provenance is not acknowledged yet.                    |

Identity labels must be nonblank and at most 512 bytes. Native identity must agree
with the canonical conversation ID. The server rejects an omitted or null
conversation ID before writing; it does not generate an identity that could be
lost with the response. A manual caller can generate its ID before its first
request and retain it for retries.

User/assistant records use the shared parser and transactional writer. Missing
UUIDs are counted as dropped; an entirely empty or ineligible input is an error.
Native structural summaries and PR observations retain their separate consumers.
Malformed accepted-record metadata rejects the import before any database change.

## Result and retry behavior

Replies use `Content-Type: text/event-stream` with `event: message` and one
`data:` JSON-RPC envelope. A successful MCP result contains `isError: false`,
`structuredContent`, and one JSON text block containing the same value:

```json
{
  "conversation_id": "synthetic-session",
  "path": "agentic",
  "messages_received": 1,
  "records_new": 1,
  "records_enriched": 0,
  "records_dropped": 0,
  "ack_through": "synthetic-record",
  "pending": 0,
  "draining": false,
  "provenance_received": { "github_pr_urls": [] },
  "scope": {
    "source": "local",
    "agent_brain_id": null,
    "org_name": "local",
    "workspace_name": "local"
  }
}
```

The acknowledgement is constructed only after the shared database transaction
commits. Replaying an existing UUID inserts no new record. Newly supplied usage
may enrich it, and duplicate-only input can still receive a durable receipt.
Each receipt covers only accepted UUIDs and measurement fields submitted in that
request; it cannot borrow omitted history or richer stored measurements.

A failed transaction returns `isError: true`, no structured acknowledgement and
no change events. A wholly rejected ownership collision can commit a conflict on
the original owner: it returns a tool error with no acknowledgement and creates
or enriches no destination session/source metadata, while committed original-owner
invalidation is still delivered. Type-only rejections preserve that same boundary. Notifications have no request ID
and receive HTTP 202 without invoking an import. Invalid JSON/RPC arguments use
JSON-RPC errors; oversized bodies receive HTTP 413.

An absent retention setting defaults to metrics and indexing (metadata-only).
Full-content archival requires an explicitly saved opt-in; transport arguments
cannot enable it. Existing content and saved preferences remain unchanged.
The metadata-only policy prevents new transcript content, tool input and
titles from being retained, including on enrichment and retry. Namespaces,
structural measurements and content-free receipt evidence remain available.
Change events are published by the commit worker, so a disconnected HTTP caller
cannot cancel them after a successful commit. A missing event subscriber does
not undo a durable import.

## Producer compatibility

The executed reference is the production `flush_turn.py` and `mcp_http.py` from
[`XTraceAI/agent-plugins` at `fae871a5c3385d14e55ea64f6bda7421c52ebebb`](https://github.com/XTraceAI/agent-plugins/tree/fae871a5c3385d14e55ea64f6bda7421c52ebebb/plugins/memhub/scripts).
The harness runs an unchanged source snapshot against disposable local databases
through both `MEMHUB_MCP_BASE_URL` and a copied plugin `.mcp.json`. It observes the
real byte cursor, rather than replacing the hook with a mock client.

This pin implements a single destination. Its plugin config-file route is not
proof of future multi-destination user configuration or independent destination
cursors. It also reads an entire pending tail, so pending tails larger than this
endpoint's message/body limits require producer-side chunking before that path
can be considered complete. Other host hooks and automatic native-app startup
remain separate integrations. These limits must be resolved before enabling
general capture by default.
