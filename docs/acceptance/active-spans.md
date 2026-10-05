# Active spans and additive agent duration

`MetricsDb::active_spans(Window)` implements M-05 over canonical
`v_session_events`. Human, assistant, tool-result and sidechain events participate;
existing judge, meta, synthetic and global UUID ownership rules remain in the
shared views. Copied contexts do not add activity.

Exact native instants determine half-open window membership before timeline
construction. Per-session events then sort by `(ts_ms, precise instant, UUID)`.
Durations use the stored POSIX millisecond axis, not physical leap-second elapsed
time. For example, leap `23:59:60.9` followed by ordinary `00:00:00.1` projects in
reverse order: the duration axis sorts those endpoints and yields 800 ms. Exact
membership still places the leap event before midnight. A selected leap event's
projected endpoint can therefore exceed the numeric window end; no clipping or
out-of-window activity is inferred.

| Case                                           | Returned durations                               |
| ---------------------------------------------- | ------------------------------------------------ |
| 19m59s gap                                     | 1,199,000 ms                                     |
| Exactly 20m gap                                | 1,200,000 ms                                     |
| 20m01s gap                                     | Two zero-duration spans                          |
| Tied timestamps / singleton                    | Zero                                             |
| F1 baseline                                    | 1,380,000 ms                                     |
| F2 lanes 12:00–12:50, 12:10–12:30, 12:20–12:40 | 3,000,000 + 1,200,000 + 1,200,000 = 5,400,000 ms |
| F3 week boundary                               | Two in-window singleton spans, zero duration     |

F2 includes interior events ten minutes apart. Its 90 additive minutes over a
50-minute union support future peak-three / mean-1.8 concurrency coverage; this
change does not calculate concurrency or human time. F2 stays a product skeleton
with a named `spans` snapshot loaded through the existing fixture loader. It does
not claim completed M-06, M-07 or U-01 coverage.

The integration suite compares returned duration against both the sum of span
endpoints and an independent SQLite `lag` eligible-gap sum after exact window
selection. It checks F1/F2/F3, structural classes, canonical exclusions, copies,
empty inputs, precise submillisecond membership and leap projection reversal.

Run `cargo test -p xt-metrics spans`. No UI, storage migration, pricing or global
span interpolation is included. The DTO exposes session, host and endpoint
milliseconds plus total additive duration for later consumers.
