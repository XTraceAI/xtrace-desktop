# Usage measurement and session capture coverage

`MetricsDb::coverage(display_window, now_ms, discovery_health)` returns separate
usage measurement, a fixed trailing-14-day token gate, and per-surface session
capture health. All component queries run in one read transaction, reusing an
existing caller transaction when present. The read-only open path prepares the
actual queries; it does not repair or migrate missing schema.

## Usage measurement

The denominator is distinct canonical sessions with work events whose precise
native timestamps fall in the display window. Missing event timestamps cannot be
assigned to that window. Shared exclusions and global UUID ownership remain in
the existing work-event view.

A session is measured only when it has at least one selected in-window usage
observation and every selected observation has all four counters plus a known,
nonblank model. Measured zeros count. Content-only blocks do not create phantom
usage observations. Claude response selection occurs before window filtering in
the existing response view. The report gives total, host and raw-surface counts,
percentages on a 0–100 scale, and non-additive gap categories: no selected usage,
incomplete counters, and unknown model. Missing raw surfaces remain a null group.

The token gate always uses `[now_ms − 14 days, now_ms)`, independent of display
range. Only the explicit `(cursor, cursor-cli)` structural pair is removed from
its eligible denominator and named in `excluded_surfaces`. Unknown surfaces,
other Cursor surfaces and other hosts remain eligible. The gate passes at 90%
using integer comparison. An empty eligible set has null percentage and pass
result, never an invented 100%.

## Capture health

The denominator is distinct discovered native sessions whose stored native
`started_at_ms` is in the display window, including discovery-only sessions not
yet imported. Event timestamps and receipt dates never substitute for native
start. Unknown-start discoveries are counted separately and suppress the
percentage for their surface.

A captured session requires a sealed receipt for its known canonical conversation,
matching the discovery host and the same known raw surface on both canonical
session and receipt. Unknown surfaces do not match each other. A receipt on CLI
cannot establish Desktop capture; a source label or installation state cannot
substitute for a receipt. A valid partial receipt can establish session health.
This API does **not** establish complete measurement verification or export a
`verified_capture` claim. Receipt masks and revisions remain immutable.

The caller supplies an explicit `DiscoveryHealth` for each host/raw surface:
`fresh_complete`, `unknown`, `incomplete`, `stale`, `missing_python`, or
`unavailable`. Missing context defaults to unknown. Observed denominator and
receipt counts remain available in every state, but an authoritative percentage
requires fresh/complete context and no unresolved row-level gaps. Stored
`discovery_complete` flags alone are not proof of a fresh inventory. Duplicate
context entries are rejected rather than silently choosing one.

A discovered unknown surface with an in-window or unknown native start might
belong to any surface of that host. Those sibling percentages are suppressed with
`unresolved_surface_denominator`; their observed counts, receipt facts and supplied
inventory states remain unchanged. Another host is unaffected. An unknown-surface
row with a known start outside the window does not cause that uncertainty.

This consumer performs no native scans, refresh timers or discovery lifecycle
work. Runtime focus/manual/periodic refresh integration remains separate; callers
must not supply `fresh_complete` without that evidence.

## Synthetic evidence

| Case                                                       | Observed result                                                                                  |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| F6: zero, mixed partial, blank/missing model, content-only | 5 sessions, 1 measured, 20%; gap categories remain explicit                                      |
| F7 Cursor CLI without usage                                | Unmeasured and specifically excluded from the fixed gate                                         |
| F10 abort without counters                                 | Unmeasured and still eligible                                                                    |
| Fixed gate: 9 measured of 10                               | Exactly 90%, passes; display range changes do not change it                                      |
| Empty eligible denominator                                 | Percentage and pass are null                                                                     |
| F20 CLI with sealed receipt                                | 1 discovered / 1 captured when caller inventory is fresh and complete                            |
| F20 Desktop without receipt                                | 1 discovered / 0 captured; CLI receipt has no effect                                             |
| F20 new raw surface, no canonical import                   | Discovery denominator retained; 0 captured                                                       |
| F20 missing start, identity or incomplete discovery        | Observed counts plus explicit reasons; percentage suppressed                                     |
| Unknown-surface row inside / untimed / outside window      | Same-host suppression / suppression / no effect; other host isolated                             |
| F18 canonical arrival orders and partial receipt           | Usage can become measured without capture; adding a partial receipt changes only session capture |

F6/F7/F10/F20 add named canonical snapshots through the existing fixture loader;
F18 reuses its named canonical enrichment inputs. These checks use the production
store APIs and do not claim full fixture, reader, plugin transport, or UI acceptance.
Tests also cover unsealed/mismatched receipts, exact window selection, measured
zeros, absent runtime health, read-side schema rejection, and a writer commit
between component reads proving snapshot consistency.

Run `cargo test -p xt-metrics coverage` and
`cargo test -p xt-metrics -p xt-fixtures --locked`, followed by Clippy and formatting.
