# ING-02 parser acceptance

The parser reuses `xt_store::CanonicalRecord` and adds a typed envelope for
native ancestry and independent line/header source observations. It performs
no I/O or persistence and makes no capture-verification or PR-link authority
claim. Every fixture value is synthetic.

## Producer and dataset identity

Native PR-link and stop-hook-summary field names were checked by static
inspection of the signed macOS arm64 Claude Code 2.1.260 executable. Its
`com.anthropic.claude-code` signature verified as Anthropic PBC, and its SHA-256
was `3c269f66801028823e24a63ced9fdd3988cb86cf85fccd9f03f87e463b9d3e3c`.
No binary, proprietary implementation excerpt or real transcript is included.
This is compatibility evidence for that producer version, not a promise of a
stable upstream transcript schema or a live end-to-end ingestion test.

The shared fixture loader reads named `parser` snapshots under F17/F18/F20.
The fixed `canonical-native-v1` dataset has 20 cases with these file hashes:

| File | SHA-256 |
| --- | --- |
| `fixtures/F17/input/parser.json` | `8c934266dc191d836034993c53f78948b320d55ea19406b14715cfd4529e6f17` |
| `fixtures/F18/input/parser.json` | `761ccd524ed77495a025b52359fb00a8e73b2b0e302eceb9754709bdc03f5cf9` |
| `fixtures/F20/input/parser.json` | `774b2ad828578350457a4f6b3fb2bb573af0380028bf4be378319b45a41823e7` |

## Input shapes and expected results

| Synthetic setup | Action and expected result |
| --- | --- |
| Claude cache/sidechain and two UUIDs sharing a response | Parse both records; retain distinct UUIDs, response/request IDs, cache splits, service tier, native ancestry and full timestamp spelling. |
| Cursor missing usage/model/time/cwd and minimal UUID/type-only user | Parse records with absent measurements still unknown; do not invent a message body, model, time or counters. |
| Reader-shaped Codex usage with informational reasoning detail | Parse the supplied output total unchanged; do not add reasoning output again. |
| Native string content, empty string and empty array | Normalize the string to a text block; preserve unknown versus measured-empty content and tool counts. |
| Tool-use blocks and user tool-result carrier | Classify in memory; derived count/carrier facts remain usable after the caller discards transient content. |
| Missing/null/blank UUID user/assistant | Return `Dropped(MissingUuid)`, including rows whose other fields are malformed. |
| Unknown native type or unrelated system subtype | Return `Inert`; unrelated payload fields do not become canonical records or parser errors. |
| `type: pr-link` with sessionId/prNumber/prUrl/prRepository/time and no UUID | Return `PrLink` with the positive number and original observations; do not require a fabricated UUID or create a link. |
| `type: system`, `subtype: stop_hook_summary`, native UUID/time/count/duration | Return one `StructuralEvent`, preserve structural fields and discard commands/errors/context/stop-reason text. |
| New raw entrypoint, distinct explicit surface/native IDs and reader header context | Preserve each independent observation, use only measured native fallbacks, and leave the event timestamp unknown when absent. |
| Invalid JSON, known field types, dates, blocks, counters and PR/hook numbers | Return bounded, payload-free errors; diagnostic display/debug text remains below 128 bytes and never echoes the synthetic sensitive marker. |

These cases are exercised by nine tests in
`crates/xt-ingest/tests/canonical.rs`, including the explicit performance test.

## Measured verification

`cargo test -p xt-ingest canonical --locked --offline` passed all nine tests.

The combined `cargo test -p xt-store -p xt-fixtures -p xt-ingest -p xtask --locked --offline`
run passed 70 tests (38 storage, 18 harness, nine parser, five CLI). Strict Clippy
for those crates, workspace formatting, diff checks and fixture validation also
passed; catalog validation still reports one populated entry and 20 skeletons.

`cargo test -p xt-ingest --release canonical_parse_50k --locked --offline -- --nocapture`
parsed 50,000 lines in **59 ms** on an **Apple M5 Pro**, macOS arm64, with the
pinned Rust 1.94.1 toolchain. Counts were exactly 30,000 records, 5,000 structural
events, 2,500 native PR links, 7,500 inert lines and 5,000 dropped rows. The
measured loop includes parsing and result allocation/drop; fixture loading and
hardware discovery occur before the timer. Release mode enforces the one-second
budget. Debug mode checks the same counts without a timing threshold.

F17/F18/F20 remain product skeletons with empty product expectations; their
earlier schema snapshots remain intact. Complete replay/retention, capture
coverage, metric selection and actual host conformance are later consumers'
acceptance responsibilities.
