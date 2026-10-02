# Model and session summaries

`favorite_model(window)` ranks measured output observations from the existing
canonical response-usage projection. Response deduplication still happens before
event-window filtering. Missing output excludes only that observation from the
ranking; other measured output for the same model remains usable. Input-only
usage cannot create a favorite. Explicitly measured zero is distinct from unknown.

Unknown-model measured output is retained as an uncertainty bound. A unique named
leader is reported only when its measured output exceeds the largest named rival
plus all unknown-model output. This allows unknown output to reinforce a rival,
not merely to form a new model. For example:

| Named A | Named B | Unknown output | Result                                      |
| ------: | ------: | -------------: | ------------------------------------------- |
|     100 |      80 |             50 | Unknown                                     |
|     130 |      80 |             50 | Unknown; a tie remains possible             |
|     140 |      80 |             50 | A                                           |
|     100 |      80 |              0 | A                                           |
|       0 |       0 |              0 | Unknown if any measured identity is unknown |

Positive named-output ties use responding turns, attributed to each segment's
first assistant record. Exact native timestamps and UUIDs determine chronology,
using only events in the selected window. Missing classification, role or first
assistant model needed for attribution produces an explicit unknown reason.
Leading records, later assistant blocks and trailing unanswered humans do not
manufacture attributed turns. A known human resets earlier boundary uncertainty.
Only fully measured turn ties use lexical model order. A provable unique output
winner skips turn attribution entirely. This is a conservative direct fold, not
an enumeration of possible missing histories.

`favorite_comparison(window)` returns current and previous categories in one
read-only snapshot, using the equal-elapsed `Window::previous()`. It does not
calculate a percentage for a model name. `favorite_iso_week(anchor_ms, zone)`
uses a complete Monday-to-Monday local ISO week with an explicit anchor and zone.
Calendar arithmetic handles DST and year boundaries; this weekly lookup is
separate from equal-elapsed comparisons. Existing caller transactions are reused.

`sessions_per_day(window, zone)` counts each session once, on its first exact
in-window event. Native session start times are irrelevant. It reports all local
date buckets, including zeros; max and mean use those same buckets. The mean is
total sessions divided by reported bucket count. Partial days count as one bucket,
and a 23-hour or 25-hour local day also counts as one. A 71-hour spring-DST window
from Saturday noon through Tuesday noon therefore reports four date buckets.

`Delta::new` takes current/previous values and the caller's metric-specific sample
counts. Both counts are preserved. Percentages are suppressed when either sample
is below five, either value is unknown/nonfinite, the previous value is zero, or
arithmetic would yield a nonfinite result. Known values remain visible even when
the percentage is suppressed. Callers choose the relevant sample unit—sessions,
stretches or PRs—and use matching current/previous windows.

## Synthetic evidence

F11's named `favorite` snapshot contains actual canonical records in two ISO weeks:

| Week beginning | A output / turns | B output / turns | Favorite     |
| -------------- | ---------------- | ---------------- | ------------ |
| 2026-08-31     | 100 / 2          | 100 / 1          | A, by turns  |
| 2026-09-07     | 10 / 1           | 30 / 1           | B, by output |

A first-week variant removes only the second A prompt. Both A response blocks
remain measured, yielding 100 outputs and one turn each for A and B, resolved to
A lexically. Both retention modes and replay produce the same weekly favorites.
F11 remains a skeleton for its broader fixture obligations.

F3 assigns its one session to its first in-window date and includes six zero days;
F2's existing three lanes yield three sessions on the final date and a mean of
3/7. Tests additionally cover timezone offsets, sub-nanosecond ordering, leap-second
date membership, partial/DST dates, output uncertainty bounds, partial output,
unknown turn facts, schema rejection without repair, and a writer commit between
composed reads. No new schema, dependencies, cache or calendar framework is added.

```sh
cargo test -p xt-metrics favorite
cargo test -p xt-metrics sessions_per_day
cargo test -p xt-metrics delta
cargo test -p xt-metrics -p xt-fixtures
cargo clippy -p xt-metrics -p xt-fixtures --all-targets --locked -- -D warnings
cargo fmt --all --check
```
