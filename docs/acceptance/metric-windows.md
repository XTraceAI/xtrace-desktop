# Metric windows and canonical projections

Metrics read the existing app-owned SQLite database. The writer applies the two
named metric views after its migration chain, atomically replacing them only
when their embedded definitions change. No separate version table, migration
runner or data copy is introduced. Reopening with unchanged definitions does not
rewrite the database. The metric reader cannot create, migrate or write it.

`Window` is a validated half-open interval `[start_ms, end_ms)`. Its previous
window has the same elapsed duration and ends at the current start. Local-day
buckets use the caller's timezone, clip the first and last days, and handle
23/25-hour days and skipped civil dates through Jiff (already pinned in the
workspace). The caller supplies the end/now; the query never reads the clock.

`v_records` joins canonical records, their owning sessions and optional usage
without multiplying records by source receipts or native copied-context rows.
Metadata, judge sessions and the synthetic model are excluded; unknown fields
remain null and sidechains remain included. `v_session_events` additionally
requires a timestamp. `v_records` and `v_human_inputs` both read the internal
`v_record_metadata`, which joins each record with its session and its stored
input facts once and decides Human eligibility in one place. Window predicates use the existing indexed `ts_ms` column,
not session start, file modification time or a derived timestamp.

## Boundary evidence

F3 declares `input/boundary-events.jsonl`, proving the harness has no alternate
hard-coded input path. Its session starts in August; the selected interval is
September 1 through September 8, 2026, UTC, excluding the end.

| Event timestamp           | Current window | Previous window |
| ------------------------- | -------------- | --------------- |
| August 31, 23:59:59.999   | no             | yes             |
| September 1, 00:00:00     | yes            | no              |
| September 7, 23:59:59.999 | yes            | no              |
| September 8, 00:00:00     | no             | no              |
| missing                   | no             | no              |

The current window contains two events with raw fixture input/output counters
50/5; the previous window contains one event. These raw counters verify the
window boundary only. Response-level token deduplication, complete-counter
coverage, cost and Dashboard DTOs are subsequent work, not implemented metrics
in this foundation.

## Checks

- `cargo test -p xt-metrics window`: interval validation, adjacent previous
  windows, clipped days, DST and skipped dates.
- `cargo test -p xt-metrics schema_contract`: F1/F3 golden comparisons, committed
  updates visible to an already-open reader, replay/copy deduplication, exclusions,
  unknown metadata, view replacement and a reader that rejects writes.
- `cargo test -p xt-fixtures`: shared manifest loader and independent reference
  arithmetic, including unchanged fixture export and no-clobber guarantees.

No visible Dashboard change is expected from this PR.
