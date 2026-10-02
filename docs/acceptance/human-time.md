# Human wall-time estimate and agent ratio

`MetricsDb::human_time(window, typing_rate)` reports estimated human wall minutes,
the sum of each session's unioned human minutes, agent minutes, and their ratio.
Both database reads share one read-only snapshot; an existing caller transaction
is reused without being ended. The agent numerator uses `active_spans` directly.
No public interval IDs, new storage, or UI are introduced.

Exact native timestamps select the half-open event window before chronology.
Within each session, timestamps and then UUIDs determine order. Each known human
uses the previous known non-human event, including tool-result carriers; another
human does not reset that previous event. The POSIX millisecond gap is clamped to
zero through 30 minutes. Without a previous in-window agent event, the duration
is text characters divided by the positive typing rate (default 200 per minute).

Intervals are clipped to the selected window before union. The main total unions
all sessions together. The comparison unions each session first and then sums
those durations, so consecutive overlapping humans are not counted twice even in
the comparison. A zero human denominator yields no ratio. Unknown classification
or a needed typing length yields unknown human totals and ratio while preserving
the known agent numerator. A missing length is not needed when a prior agent
establishes the interval. Untimed events cannot be assigned to the window.

Internal coordinates use checked integers scaled by the common typing rate.
Typing durations, clipping and unions remain exact until the final conversion to
minutes, preventing a tiny estimate from rounding away at a large epoch. This
representation is private; it does not change timestamp or storage contracts.

## Synthetic fixture arithmetic

F1 yields 8.135 human minutes: 27 characters / 200 = 0.135 minutes for the first
message, then four two-minute gaps from previous agent events. Its agent total is
23 minutes, giving `23 / 8.135` for the ratio. Both retention modes and duplicate
replay produce the same result.

F2 reuses its existing three-lane timestamp/UUID timeline from the `spans`
snapshot. Selected rows now carry actual human text; timestamps remain unchanged.
Its `human_time` snapshot contains only the expected interval arithmetic.
Offsets below are minutes after the first event:

| Session | Unioned human intervals | Human minutes |
| ------- | ----------------------- | ------------: |
| Lane A  | [0,20], [30,50]         |            40 |
| Lane B  | [10,30]                 |            20 |
| Lane C  | [20,40]                 |            20 |
| Sum     | Per-session unions      |            80 |
| Wall    | [0,50]                  |            50 |

Agent minutes remain 90, so the agent/human ratio is 1.8. Existing concurrency
expectations remain peak 3, occupied wall minutes 50, and weighted mean 1.8.
The raw overlapping human intervals sum to 120 minutes; neither reported human
total uses that sum. Reversed arrival and both retention modes agree. F2 remains
a skeleton for its broader fixture obligations.

F3 has no human duration and no ratio. An added boundary human proves that an
out-of-window agent is not used and that typing intervals clip at window start.
Additional tests cover the 30-minute cap, tool results, consecutive humans,
adjacency, precise ordering beyond nanoseconds, leap-second projection reversal,
end clipping, unknown measurements, configurable rates and tiny typing estimates
at year 9999. Snapshot tests commit a separate writer between the two reads and
verify reuse of an existing transaction.

## Checks

```sh
cargo test -p xt-metrics human_intervals
cargo test -p xt-metrics union
cargo test -p xt-metrics ratio
cargo test -p xt-metrics -p xt-fixtures
cargo clippy -p xt-metrics -p xt-fixtures --all-targets --locked -- -D warnings
cargo fmt --all --check
```

The actual human query is prepared when the read-only reader opens. Missing
projection fields are rejected without reader-side schema repair. Tests use the
shared fixture loader and the production canonical writer; legacy NULL and
concurrent-write tests alter only disposable test databases.
