# Response token aggregation

`MetricsDb::tokens` returns one report with totals and host, model, host/surface,
and local-day breakdowns. All use the same selected responses and one SQLite read
snapshot. This PR supplies Rust result types; Dashboard commands and UI follow
separately. Cost, favorite-model selection and capture-health policy are separate. The partial-usage
tests verify the nullable inputs to favorite-model selection; its aggregation
assertion remains outstanding in the separate favorite-model implementation.

## Selection and unknowns

Canonical UUID storage removes replays first. For Claude, nonblank API-message
and request IDs identify one response; missing either key falls back to that
record's UUID. Other hosts keep their canonical UUIDs. Keys use separate SQL
columns, including a response/fallback discriminator, so concatenation or null
keys cannot merge unrelated records.

For each response, select one actual usage observation by its precise native
instant, then greatest UUID only for equal instants. UUID provides a deterministic tie-break;
it does not establish chronology. Counters and attribution come from that one
selected row. Only persisted usage observations participate; content-only rows
neither supersede usage nor become phantom responses. Absent usage is unmeasured.
Selection happens before filtering the selected timestamp to the window or local
day. Missing timestamps cannot be assigned to a window. Content/tool records are
never removed. The raw timestamp preserves fractional digits beyond nanoseconds;
normalized offsets and trailing zeroes identify the same instant. The existing
`ts_ms` index restricts candidates with a one-second upper overlap for leap
seconds, while precise comparison checks all successors. Exact native instants
then determine half-open window membership and local-day assignment before any
counter is accumulated; a leap second before midnight stays in the preceding day.

`Store` and `MetricsDb` register the shared timestamp comparator on their connections.
Direct SQLite consumers of `v_response_usage` must call
`xt_store::timestamp::register_sqlite` first; this also works on read-only connections.
Malformed stored timestamps return an error rather than acquiring an invented order.

Each counter is nullable independently. If any selected response omits a
component, that component's aggregate is unknown; the total requires all four
components. Explicit zeros remain zero. Windows or day buckets with no usage observations have zero selected responses
and unknown token counters; they do not establish measured zero. Unknown model/surface labels retain their measured counters.
Measured sessions require complete counters for every selected response in that
group/window. These are usage measurements, not evidence of plugin capture.
Overflow returns an error rather than a wrapped, saturated or rounded value.

The shared canonical projection excludes metadata, judge sessions and the
synthetic model. Sidechain work is included; copied contexts never multiply
canonical work. Codex output already includes reasoning and input excludes cache;
the aggregator adds only the four canonical counters.

## Synthetic evidence

The existing manifest loader exposes named `tokens` snapshots in F5/F6/F17/F18.
They prove this token-only scope while the broader fixtures retain their honest
skeleton status. F1's populated M-04 golden is also exercised.

| Fixture | Expected result                                                                               |
| ------- | --------------------------------------------------------------------------------------------- |
| F1      | 10 selected responses; 750 input + 150 output + 150 cache read + 50 cache creation = 1,100    |
| F5      | 70 fresh input + 40 output + 30 cache read = 140; informational reasoning is not added again  |
| F6      | Repeated generation UUID gives one response and 48 tokens                                     |
| F17     | Selected UUID suffixes 1702, 1705, 1706, 1708; 38 input + 5 output = 43 in the current window |
| F18     | Native then hook, hook then native and replay all converge to one record and 48 tokens        |

F17 uses the canonical writer. Its 1701/1702 pair crosses midnight; only 1702 contributes on September 2 UTC.
The 1703/1704 pair crosses the window end; only 1704 contributes in the next
window. Equal-time 1708/1707 selects 1708 by UUID, independently of insertion order. Missing-ID rows 1705/1706 remain
independent; 1705 retains its unknown model and sidechain work. All eight records
and three tool calls remain stored after usage selection.

`cargo test -p xt-metrics tokens` additionally covers partial/absent/zero usage,
post-open enrichment, metadata/judge/synthetic exclusion, copied contexts,
missing and blank IDs, non-Claude response keys, local days and overflow.

F18 replays both native and enriching sources in each initial arrival order and
checks the full report and stable stored UUID. A separate F17 test reverses input
order and verifies timestamp priority over the deterministic UUID tie-break.
