# Concurrency over active spans

`MetricsDb::concurrency(Window)` implements M-06 by calling `active_spans` once
and sweeping those endpoints. It uses the same exact event-window membership and
POSIX millisecond duration axis as the [active spans](active-spans.md) calculation.
There is no additional query or stored projection.

End events sort before start events at the same millisecond. Touching half-open
spans never overlap, and zero-duration spans contribute no endpoints. The report
contains `max`, `mean`, and `wall_active_ms`. Mean weights each positive-occupancy
interval by its duration; the denominator excludes idle time. With no
positive-duration spans, max and mean serialize as null and wall time is zero.
Integer duration, weighted duration, and occupancy arithmetic is checked before
converting the final ratio to floating point.

F2's existing spans form this independently authored occupancy table:

| Interval    | Occupancy | Wall minutes | Agent minutes |
| ----------- | --------- | ------------ | ------------- |
| 12:00–12:10 | 1         | 10           | 10            |
| 12:10–12:20 | 2         | 10           | 20            |
| 12:20–12:30 | 3         | 10           | 30            |
| 12:30–12:40 | 2         | 10           | 20            |
| 12:40–12:50 | 1         | 10           | 10            |

Total wall time is 50 minutes (3,000,000 ms), weighted activity is 90 minutes,
peak is 3, and mean is 90/50 = 1.8. The named `concurrency` snapshot stores the
occupancy table and expected report; tests load it through the shared fixture
loader and compare every interval against the actual returned lane spans.
Reversing both session and event arrival order yields the same report.

`cargo test -p xt-metrics sweep` also exercises touching lanes, idle gaps, empty
inputs, tied events, singletons, event-window filtering and checked overflow.
F2 remains a broader product skeleton. These are metric and U-01 input assertions;
there is no UI lane-rendering acceptance or human-time implementation in this change.
