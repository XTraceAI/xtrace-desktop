# One session's hands-off stretches acceptance

`MetricsDb::session_stretches(Window, session_id)` states the M-09 hands-off
stretches **one** session contributed inside one explicit window, for the
session detail timeline. It is the reviewed core contract the timeline reads;
[hands-off.md](hands-off.md) defines M-09 itself.

Run `cargo test -p xt-metrics --test session_stretches --locked --offline`,
`cargo test -p xt-metrics --lib session_stretches --locked --offline`,
`cargo test -p xtrace-desktop --test session_stretches --locked --offline` and
`cargo test -p xtrace-desktop --features fixtures --test fixture_mode --locked
--offline`.

## Nothing is redefined

The events, the raw-surface timestamp health and the stretch fold are M-09's
own private helpers, shared with `MetricsDb::hands_off`. A listed stretch is
therefore one of the segments the Dashboard's hands-off figure was built from,
with the same duration on the same stored millisecond projection. The existing
M-09 suite passes unchanged.

## Three answers, never an empty list for all of them

- `missing` — no indexed user session owns the identifier. The identity is
  compared exactly: a prefix, a whitespace variant or a case variant of another
  session's identifier is `missing`.
- `unmeasured { excluded_surface }` — the session is indexed and M-09 states no
  stretches for it. `excluded_surface` names the raw `(host, surface)` whose
  timestamp health excluded it, with its qualifying and degenerate session
  counts, when that is the reason; it is `null` when the session's own in-window
  records leave a boundary or a segment's tool presence unknown. An excluded
  surface is a statement about how that surface records time, not a claim that
  this session's timestamps are absent.
- `measured { stretches }` — every stretch the session has in the window, in
  chronological order. An indexed session with no qualifying segment in the
  window has an honest empty list.

## Narrower than the global report, on purpose

- **Health is the surface's, not the session's.** Timestamp health is judged
  over every session the raw surface has in the whole selected window, exactly
  as M-09 judges it, and only then narrowed to the requested session. One
  session can never make its own surface healthy, and an eventless session
  cannot avoid its surface's exclusion.
- **An unknown is the session's own.** The global report publishes no
  distribution while any in-window session is unmeasured. Here another
  session's unknown classification says nothing about this one.

## Each stretch

`start_uuid` is the person's message (M-02's human-message rule, as in
[hands-off.md](hands-off.md)) the stretch starts at; `end_uuid` the last
explicitly non-human record before the next person's message, including a
tool-result carrier. `start` and `end` are those records' exact native
timestamps as stored. `duration_ms` is M-09's own duration and is **not**
recomputed from the two spellings.

`first_tool` is `{record_uuid, block_index}`: where the stretch's first tool
call sits — the record that carried it and that record's content block index.
It is a **position, not an identity**. The native `tool_use` id is not persisted
anywhere, and `tool_uses.id` is a database surrogate that is never the
transcript's identifier, so resolving the position against verified content is
left to whatever reads content.

It is only ever the call the stretch actually started with. Walking the
stretch's records in order, a record stating zero calls is passed over; the
first record stating at least one call is the caller, and its lowest stored
block is named only when its stored blocks account for every call it states.
A record whose call count is unknown ends the walk. Anything less leaves
`first_tool` `null` — a later call is never reported in place of an earlier one
that cannot be accounted for. The stretch keeps its place and its duration
either way, because it qualifies on the records' own tool counts.

## Reads

Metadata, events and stored block positions are read inside one snapshot, so
the surface a session is on, the records health was judged from and the blocks
a stretch names all describe one committed state while the native writer keeps
appending. The locator lookup reads `uuid`, a count and a minimum
`block_index` from `tool_uses` and nothing else: no content, no tool input, no
tool name and no source file. No schema, view, dependency, ingestion or stored
projection changed.

## Through the app

`session_stretches(session_id, window_days)` resolves the window with the same
`selected_window` every windowed read uses, and converts the core answer into
`MetricSessionStretches` through the drift-refusing conversion the Dashboard
uses: the wire is the core's JSON with no field added, dropped or recomputed,
and a test asserts the two are equal. An unsupported window is refused. The F1
fixture export carries each listed session's stretches per window preset,
produced by the same command, and the fixture-mode test asserts those bytes
equal what a running app answers.
