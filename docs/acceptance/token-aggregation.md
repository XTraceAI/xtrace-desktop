# Response token aggregation

`MetricsDb::tokens` returns one report with totals and host, model, host/surface,
and local-day breakdowns. All use the same selected responses and one SQLite read
snapshot. This PR supplies Rust result types; Dashboard commands and UI follow
separately. Cost, favorite-model selection and capture-health policy are separate.

## Selection and unknowns

Canonical UUID storage removes replays first. For Claude, nonblank API-message
and request IDs identify one response; missing either key falls back to that
record's UUID. Other hosts keep their canonical UUIDs. Keys use separate SQL
columns, including a response/fallback discriminator, so concatenation or null
keys cannot merge unrelated records.

For each response, select the latest usage observation by stored `ts_ms`, then
stable UUID for equal milliseconds. Canonical storage has no native sequence
column; none is fabricated. Only persisted usage observations participate; content-only rows neither
supersede usage nor become phantom responses. Absent usage is unmeasured. Selection happens before
filtering the selected timestamp to the window or local day. Missing timestamps
cannot be assigned to a window. Content/tool records are never removed.

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
| F17     | Selected UUID suffixes 1702, 1705, 1706, 1708; 39 input + 5 output = 44 in the current window |
| F18     | Native then hook, hook then native and replay all converge to one record and 48 tokens        |

F17's 1701/1702 pair crosses midnight; only 1702 contributes on September 2 UTC.
The 1703/1704 pair crosses the window end; only 1704 contributes in the next
window. Equal-time 1707/1708 selects 1708. Missing-ID rows 1705/1706 remain
independent; 1705 retains its unknown model and sidechain work. All eight records
and three tool calls remain stored after usage selection.

`cargo test -p xt-metrics tokens` additionally covers partial/absent/zero usage,
post-open enrichment, metadata/judge/synthetic exclusion, copied contexts,
missing and blank IDs, non-Claude response keys, local days and overflow.
