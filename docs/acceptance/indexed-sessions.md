# Indexed Sessions browser

The Sessions route reads the local index through a generated Rust DTO. It shows
host, repository/branch, a shortened session ID, observed model, first recorded
time, indexed record count and identity-conflict status. The list query does
not read source transcripts or select stored content. The identity column leads with
the repository's own name and, when it is known, the branch, because that is
what names the work; the shortened session ID and the observed model follow on
one meta line beneath them. No title is derived from transcript text to fill
that line; a host's own title for the rows in view is read separately and never
stored, as [session-titles.md](session-titles.md) describes. An unknown repository is named as such, and is named before a known
branch so a branch never reads as a repository; an unknown branch is left to the
cell's title rather than spending the row's width on saying so twice. The ID
stays visible on every row, so two sessions in one repository can still be told
apart, and the whole ID is on its title. `first recorded` is the earliest
visible work record, counting copies inherited from a forked session; it is not
the host's session start time, and no column claims to be.

Search matches literal substrings in IDs, repository paths, branches and titles
the index saved; a host title shown for display is not indexed and not searched. Host
filters and descending time/ID pagination run in SQLite, with 50 rows per page.
Unknown metadata remains unknown. Record counts are indexed user/assistant
records, including tool-result carriers; they are not human-message counts.
The current view does not include PR filtering or session-detail navigation.
A PR-filter URL explicitly explains that limitation.

## Filters in the address

The search text (`q`), host filter (`host`) and selected range (`range`) live in
the route's query string, so a link can open this list already searching and the
browser's own Back and Forward restore what was filtered. Both directions are
live: the list initializes from the address and follows it when it changes
underneath, and editing a filter by hand writes it back. Filter edits replace
the history entry rather than pushing one, so Back still means the previous
page and a keystroke never lands in the back stack. Typing is debounced 200ms
before it reaches the address, exactly as it reached the query before.

Every value is validated before use, and an unusable one is ignored rather than
sent to a query that would fail: a host outside the offered `claude`, `codex`
and `cursor` is no filter at all, an unsupported range falls back to the last
range chosen in this window, and a search longer than the 256 characters the
store accepts is bounded. An invalid address therefore degrades to a working
list whose controls still work by hand. Parameters this page does not own,
including `pr`, are preserved untouched.

The Shell derives the selected range from the address when it carries a
supported one, so the TopBar presets, the report period and the measured columns
always describe the same window. A range arrived at through a link also becomes
the window's chosen range, so moving on to a page whose address carries none
keeps it rather than reverting to the default. Choosing a range writes it back
only to an address that already carries one, which keeps a plain `/sessions`
link clean.

The Dashboard's Sessions rows and its "View sessions" link build these addresses
through one shared helper, with `URLSearchParams`, so a session ID carrying
query-string punctuation cannot split or truncate the query. Those links promise
a search, never a session detail: no row is selected for the reader, and a
search that matches nothing says "No sessions match these filters."

## Per-session measurements

Each row also shows what its session contributed inside one explicit event
window: human messages (M-02), the four-counter token total (M-04) and agent
minutes (M-05). The window is the Shell's selected range, so the TopBar presets
and report-period label appear on Sessions; the page reports that window with
its bounds, zone and clock, exactly as the Dashboard does.

The numbers come from the projections the Dashboard already reads. Global M-04
response selection and copied-record deduplication happen inside those shared
projections, before any session or window filter, so a row is a slice of the
Dashboard's totals rather than a second definition of them. The metadata page
and every row's measurements are read on the metrics connection inside one
SQLite read transaction: the native index writes on its own connection, so a
second read could otherwise pair a row with numbers taken after it changed.

Each measured column states its own rule. The header label is a real button
wrapped in the shared `RulePopover`, so hovering or focusing `msgs`, `tokens` or
`agent min` quotes M-02, M-04 or M-05 verbatim from the reviewed rule contract,
and Escape dismisses it without moving focus. Those labels are abbreviated for
density while the spoken column name stays whole, so the button is still reached
as "Human messages, definition M-02" or "Agent minutes, definition M-05". Because the label
itself is the trigger, no glyph is added and a measured column stays as narrow
as an unmeasured one. Every cell beneath a header is tied to it by the grid's
`columnheader`/`cell` roles, so no number is presented without its definition,
and there is one tab stop per metric rather than one per cell. The unknown
reason on a cell explains why that value is missing; it is not the definition.

The `Records` column still counts records visible in that session, including
copies inherited from a forked session; copied work is measured once under the
session that owns it, so a copied session's measured columns are legitimately
lower. The column header states this.

Unknown, zero and missing stay distinct. A session with no events in the
selected window reports a measured `0` for events, human messages and agent
minutes, while its token counters stay unknown, because M-04 selected no
response to measure. A session whose span is too short for the shown scale is
distinct from both: two seconds of active span reads `<0.1`, not the idle
window's `0` and not an unknown. An unmeasured human classification leaves that row's
human-message count unknown without hiding its other numbers. An identifier
no indexed session owns reports `missing` and no numbers at all. No metric
definition, count or inferred surface is introduced here.

## Range summary above the list

Four tiles sit between the heading and the recent-activity section, under one compact row of the
page's limits, each a short phrase that stays in view with its explanation one
closed native disclosure away (Tab, then Enter or Space):

- "All indexed activity · selected range", whose disclosure — also the range
  summary's accessible description — is the full rule: "All indexed activity in
  this range. Search, host and pull-request filters apply to the table below."
- When the index reports it, one warning-toned state word — `Index unavailable`,
  `Indexing`, `Updates interrupted` or `History incomplete`, chosen by the same
  predicates as before — whose disclosure holds the full sentence and the
  "View indexing status" link to Settings. Nothing is shown when the index is
  healthy or its status has not been read.
- When the report counts untimed history, "Untimed history · N records ·
  outside dated measurements": a count of all indexed history, never of the
  filtered rows or the selected range, disclosed with the host/surface list.

An opened item takes a line of its own. The row sits outside the range summary
region. The tiles describe the whole selected range, not the page of rows the
list happens to have loaded, and the caption says so rather than leaving a
reader to compare the two.

They are read from the Dashboard's own `dashboard(days)` report, through the
same cached query the Dashboard uses, so no new query, endpoint or metric
definition is introduced and the two pages cannot disagree for one range. The
search and host filters reach the list's query alone: the report is never read
again for a filter change and never narrowed by one.

- **Human messages** is the report's M-02 tile, with its reason, sample counts
  and its change against the previous period, suppressed exactly where the
  report suppresses it.
- **Output tokens** is `tokens.counters.output_tokens`. M-04 measures output on
  its own, so it is shown even when the range total is unmeasured; the tile's
  definition says that and gives the range total when there is one. The report
  publishes no previous-period output, so no change is shown rather than one
  being derived here. A null output counter can mean the counters are absent
  and can mean a response mixed measured counters with missing ones, and the
  report does not distinguish them, so the unknown reads "Output counters are
  absent or incomplete" rather than claiming there was no output. The coverage
  count in the definition is `usage_coverage.total`, which counts sessions with
  no gap at all: it is named as sessions with all four counters and a known
  model, and stated not to be a count of sessions with measured output.
- **Agent minutes** is the M-05 hours tile stated in minutes. An unmeasured
  hour count stays unmeasured: only a number is converted, and the report's own
  reason is what the tile gives. A percentage change is the same in minutes as
  in hours, so the report's change carries over unrecalculated.
- **Sessions / day** is the M-16 mean, denominator untouched. Its aside is the
  busiest of the very day buckets that mean divides by, read from the report's
  `days` and never recounted, and it is omitted when the report carries no
  buckets at all.

Both one-decimal tiles are formatted by `metric-format.ts`'s `continuous`
rather than by the shared scale alone, and the list's own `agent min` column
keeps that same scale, so the page reads one way throughout. That scale rounds anything under
half a tenth to `0`, which would read as "nothing happened" for a session or a
range that did a little: one session across 30 day buckets is 0.03/day, and a
two-second span is 0.03 minutes. Such a value renders as `<0.1`, so the only
`0` on this page is a measured zero, and an unmeasured value still renders as
`—` with its reason because a formatter is never reached for one. This is not a
new precision policy: the scale, its one decimal and every value reaching it
are unchanged, and `usd` already reads a small cost as `<$0.01`. The helper is
app-level and shared with the Dashboard's continuous values; no kit formatter
and no report value changes.

The `agent min` column states the row's M-05 `agent_ms` as hours and the
minutes that remain, as the design does: 192 minutes reads `3h12m`, 124
minutes `2h04m`, 47 minutes `0h47m` and 192.8 minutes `3h12.8m`. The local
helper `agent-duration.ts` rounds once, with the scale above, to whole tenths
of a minute and only then splits, so a value the scale shows as 60 minutes
reads `1h00m`, never `0h60m`, and no tenth the column showed before is lost. A
positive span under the scale reads `<0.1m`, a measured zero `0h00m`, and an
unindexed session keeps its reason. The compact text is drawn for the eye; a
screen reader hears the value in words with the exact millisecond measurement
(`3 hours 12.8 minutes, exactly 11,568,000 ms`), which is also the cell's
tooltip and an "Agent time" line in the row's keyboard-reachable details. A
single session's spans never overlap, so a row reads at most just under
`720h` in a 30-day range; `719h59.9m` fits the unchanged 64px column. The
summary tiles, hands-off and the session detail's timeline durations keep
their own formats.

Every tile is a `StatTile` wrapped in the shared `RulePopover`, so its rule is
quoted verbatim from the reviewed contract, as on the Dashboard.

A summary that is still loading, or that failed, cannot hide the list or invent
a zero: each tile reports itself unmeasured with the reason, the list keeps
loading and paging normally, and a failure adds one line saying the list is
unaffected with a Retry beside it. No backend error text is shown.

A failed refresh of an already-loaded report is the same failure. The query
cache still holds the last successful report, but the page drops it with the
error rather than annotating it: a retained value, change or busiest day would
be presented as current when nothing confirms it still is. Every tile goes
unmeasured, the list is untouched, and Retry restores all of them together.

At 1440 and 1120 native widths, in light and dark, the four tiles keep their
full labels: a value wraps under its label rather than the label being clipped,
and the row halves below two tiles' worth of page width.

## Recent indexed activity above All sessions

The Sessions page shows "Recent indexed activity · last 48 hours" above the
existing All sessions table. It uses the same cached Dashboard report as the
range summary. The fixed 48-hour activity window does not change with the
selected range. `groupSessionLanes(report.lanes)` makes one row per session,
ordered by the end of its newest returned span. Sub-sessions are then grouped
and hidden exactly as the Dashboard's lanes are (`listedLanes` and `laneRows`
over the same report's `lane_sessions` context, by exact host and session ID):
a verified sub-session is collapsed under the session that created it; one
whose parent returned no span sits under a row that only names the parent and
says "Main session: no activity returned here", with no time, live state or
compactions; a known sub-session whose creator is not verified is not listed,
nor anything returned under it. The first eight main sessions and groups are
then shown; if there were more, the section says "8 most recent main sessions
and groups with activity in the last 48 hours; opening a group also lists its
sub-sessions." Opening a group lists its sub-sessions under it, beyond the
eight. A session that started days ago can appear near the top when it has
recent indexed activity.

For those eight sessions, the app may read the host's own title from its local
source, using the existing bounded title reader. A row falls back to its saved
title or ID. Each row shows its full ID without clipping, host, and last
recorded time. Its link opens the exact `/sessions/<ID>` route and carries the
list's address so Back restores the filters and range. The section states that
recorded activity does not mean the session is Running, and that search and
filters affect All sessions only. If the report capped its spans, it warns
that earlier activity can be missing. A failed report shows no recent rows,
including when a prior result remains in the query cache. The All sessions
table keeps its existing query, order, pagination, and filters.

## Sub-sessions in All sessions

All sessions groups the rows it has loaded with the same Dashboard helpers.
Each 50-row page, its query, filters, cursor and every row's measurements are
unchanged; nothing extra is fetched. A verified sub-session is collapsed under
its parent's own row when that row is loaded, and the group sits where its
newest loaded member does in the list's order. A parent on a later page or
outside the filters is only named, on a row that says "Main session not loaded
here: on a later page or outside these filters"; it has no measurement, start,
PR, live state, compactions or details, and its link opens the parent's page
with the list's address. When the parent's own row arrives on a later page it
takes that row's place, and the group stays open or closed as it was.

A known sub-session whose creator is not verified is not listed, even when
searched for, nor any loaded session under it, nor a branch whose parent is not
loaded and whose page context (`referenced_parents`) says that parent is such a
session. Only the direct parent's own context is read; no further ancestor.
Parents are matched by exact host and identity. An ordinary session with no
title or parent is listed as before.

The caption keeps the raw loaded count and adds how many loaded sub-sessions are
grouped under main sessions and how many are hidden because their main session
is unknown. Group disclosures are separate from each row's details. Only drawn
rows are read for host titles, live state and compactions; a collapsed or hidden
row, or a row only naming a parent, is not. Refresh and next-page scroll
anchoring follow the drawn row by its group, so a parent that replaces the row
naming it keeps its place. Groups are not kept open across routes; Back returns
to the same list with its groups collapsed.

## Verification and expected results

- `cargo test -p xt-store --test session_list --locked`: page through more than
  two pages with tied/unknown dates without duplicates or omissions; verify host
  and literal-substring filters, unknown metadata and rejected invalid filters.
- `cargo test -p xt-metrics --test session --locked`: F1's single session
  matches its goldens (5 human messages, 1,100 tokens, 1,380,000 active ms);
  F2's three overlapping lanes partition the global totals; F3 proves the window
  boundary, an existing empty window and a missing identifier stay distinct; an
  unclassified record keeps only the human-message count unknown; and the
  metadata page and its measurements share one read snapshot.
- `cargo test -p xtrace-desktop --test fixture_mode --all-features --locked`:
  the native fixture export and committed browser fixture agree on session rows
  and their measurements, for all three window presets; the listed row's numbers
  equal the Dashboard tiles for the same window, and an invalid range fails.
- `pnpm check`: typed DataSource contract, metadata rendering, host/search resets,
  load-more, import-event refresh and query-error recovery pass. Six focused
  tests cover the range summary: the four tiles against F1's report and the
  caption, with filter changes leaving the report unread and unnarrowed;
  measured output beside an unmeasured range total, with a measured zero kept
  apart from an unknown and an unmeasured hour count converted to nothing; a
  dropped busiest-day aside when the report carries no buckets; a range change
  re-reading the report; a failed report leaving the list working, showing no
  backend detail and recovering on Retry; and an unfinished read standing no
  zero in for any tile. Five more cover the reviewed defects: a mean of 0.03,
  a measured `0` and a showable `0.5` each rendering as themselves while agent
  minutes of 0.03 reads `<0.1`; a successful load whose refresh then fails
  hiding every value, change and busiest day while the list stays readable and
  Retry restores them; and an absent output counter reading as absent or
  incomplete with the definition naming its coverage count for what it is. One
  more covers the list's own rows on that same scale: a two-second span reads
  `<0.1`, an idle window reads `0` and an unindexed session reads neither. Three focused
  tests open the M-02, M-04 and M-05 definitions from their headers by keyboard
  focus, assert the quoted rule text and `aria-describedby`, and check Escape
  closes the definition while focus stays on the header. One test covers the
  identity column against known, partly known and wholly unknown
  repository/branch metadata. `src/app/session-search.test.ts` covers the link
  contract: the whole canonical ID, the reported host, the selected range, an
  encoded ID, a dropped unsupported host and every validated host, range and
  search bound. `src/app/SessionNavigation.test.tsx` follows a lane link from
  the Dashboard into a search over the carried range, restores that search with
  the router's own Back and Forward, replaces it by typing, reports no match
  without selecting a row, and ignores an unusable host and range.
- `pnpm e2e --grep 'indexed Sessions'`: fixture metadata, measured columns and
  filters render in Chromium and WebKit, in light and dark themes; each metric
  definition opens from keyboard focus on its header and closes on Escape.
- `pnpm e2e --grep 'Sessions summary tiles'` (`e2e/sessions-summary.spec.ts`):
  at 1440x900 and 1120x720, in light and dark, the four tiles report F1's range,
  no label, value or aside is clipped or leaves its tile, the page gains no
  horizontal scroll, and the session list is still shown beneath them.
- `pnpm e2e --grep 'Sessions rows scroll inside the list'`
  (`e2e/design-alignment.spec.ts`): as in the reference, the list takes the
  page's remaining height rather than a viewport cap, so at 1440x900 and
  1120x720, in light and dark, with 60 synthetic rows the page itself does not
  scroll, the rows scroll inside their own focusable region, End reaches the
  last row with the sticky header unmoved, and Load more stays under the rows
  outside their scroll. A window too short to leave the list a usable height
  keeps a minimum list height and scrolls the page instead of losing rows.
- `pnpm e2e --grep 'below the shown scale'` (same file): a synthetic export
  served in place of F1 renders agent minutes and sessions per day as `<0.1` at
  1120x720, shows the absent-counters reason for output, and renders the row's
  own two-second span as `<0.1m`; every one of those longer values still
  fits, in its tile and in the narrow `agent min` column.
- `pnpm e2e --grep 'agent time fits'` (`e2e/sessions.spec.ts`): at 1120x720,
  in light and dark, in Chromium and WebKit, rows served at `719h59.9m`,
  `3h12.8m`, `<0.1m` and `0h00m` fit the 64px column under its header without
  page scroll, the cell's tooltip is the exact millisecond value, and the row's
  details opened with Enter state it.
- `pnpm e2e --grep 'Sessions, by keyboard'` and `--grep 'carries the selected
range'` (`e2e/session-navigation.spec.ts`): in Chromium and WebKit, a lane
  link is focused and followed with Enter with its focus ring measured inside
  the clipped lane name, the list shows the search it ran, real browser Back and
  Forward restore it, a typed search updates the address, and a 30d selection
  travels into Sessions and measures that window.
- `pnpm e2e e2e/compact-diagnostics.spec.ts`: over test-only synthetic data
  with an incomplete index, untimed history, partial output, an unresolved
  session and unresolved environment observations at once, in Chromium and
  WebKit, light and dark, at 1440×900 and 1120×720: the Sessions limits share
  one line when closed, each disclosure (and the Settings link) is reached by
  keyboard with a visible focus ring, opened explanations stay inside their row
  and the viewport, the rows keep their own scroll, and the Dashboard's effort
  method and strip legend stay one line each and open by keyboard inside their
  cards. A healthy F1 page shows only the compact scope.
- Local native validation: `pnpm check:native --base <reviewed-base-sha>`.
  In a normal native launch, Sessions must display the existing local index,
  show incomplete coverage when reported by the indexer, and load subsequent
  pages. Keep real-history evidence local; use synthetic fixtures for PR images.

No new hosted CI job or browser installation step is added by this change.
