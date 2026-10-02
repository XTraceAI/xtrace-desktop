# Hands-off stretches

`MetricsDb::hands_off(Window)` implements M-09 using one indexed query of the
shared work-event view. Timestamp health and stretch selection use the same read
snapshot. Exact native instants determine half-open window membership and session
chronology; equal instants use UUID order. Missing timestamps cannot be assigned
to a window, as in the other event metrics. This is not a source-coverage report.

A segment starts with an explicitly human record and ends at its last explicitly
non-human record before the next human, including tool-result carriers. It must
contain at least one known tool call and a positive duration. Leading agent
records, unanswered humans, no-tool segments and nonpositive durations do not
contribute. Duration uses the stored POSIX millisecond projection; a leap-adjacent
projection reversal is not a positive stretch and never reorders native events.

The result contains nullable `n`, `median_min`, `p90_min`, and an
`excluded_surfaces` list. An unknown in-window classification can change segment
boundaries, so count and percentiles are unknown. A positive-duration segment
with no known tool call and an unknown tool count is also unmeasured; a known
positive tool count establishes eligibility even if other tool counts are
unknown. Empty measured samples have `n: 0` and null percentiles.

Timestamp health is evaluated independently for each `(host, raw surface)`;
missing surface remains a separate null label. Only in-window user/assistant
records participate. A session qualifies with at least five records. It is
degenerate when distinct precise instants multiplied by three do not exceed its
record count. A surface is excluded only with at least three qualifying sessions
and strictly more than 30% degenerate sessions. Equivalent UTC-offset spellings
are the same instant; submillisecond differences remain distinct. The report
names the excluded host/raw surface and qualifying/degenerate counts. Unknown
classifications or tool counts on an excluded surface do not poison retained
healthy surfaces.

## Synthetic evidence

F1's named `hands_off` snapshot pins five endpoints, independently read from the
canonical fixture:

| Human start | Last non-human event | Duration |
| ----------- | -------------------- | -------- |
| 12:00       | 12:03                | 3 min    |
| 12:05       | 12:08                | 3 min    |
| 12:10       | 12:13                | 3 min    |
| 12:15       | 12:18                | 3 min    |
| 12:20       | 12:23                | 3 min    |

All times are 2026-09-07 UTC. Count is five; median and p90 are both three minutes.
Separate cases put a tool-result carrier after the last assistant record and
prove it extends the stretch. Median averages the two middle values for even
samples; p90 selects index `floor(0.9*n)` capped at `n-1`, without interpolation.

F9's named snapshot contains eleven actual canonical sessions. Four have six
records at two distinct instants (the degeneracy equality boundary); seven have
six records at six distinct instants. The test selects these declared subsets:

| Degenerate / qualifying sessions | Excluded? | Reason                    |
| -------------------------------- | --------- | ------------------------- |
| 2 / 2                            | No        | Below three-session floor |
| 3 / 3                            | Yes       | Floor reached; above 30%  |
| 3 / 10                           | No        | Exactly 30%               |
| 4 / 10                           | Yes       | Strictly above 30%        |

The excluded-only cases return a measured empty sample with the exclusion named.
A healthy sibling raw surface remains measured, even when excluded records have
unknown classifications. Additional tests cover the five-record session floor,
null versus literal raw labels, host separation, exact offset equivalence,
submillisecond health, unknown tool evidence, replay and both retention modes.
F9 remains a broader fixture skeleton; named-snapshot checks do not establish UI
or transport conformance.

Run `cargo test -p xt-metrics hands_off`, `cargo test -p xt-metrics stats`, and
`cargo test -p xt-metrics -p xt-fixtures`. Schema checks prepare the actual query,
reject missing fields without read-side repair, and verify writer restoration.
No schema, dependencies, text retention, parser, or stored projection is added.
