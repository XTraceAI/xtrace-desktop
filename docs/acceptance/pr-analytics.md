# Merged-PR report (PRs page)

The `/prs` page opens on **Merged effort**: overlapping linked-session effort
for each pull request whose cached state is merged at an instant inside the
selected window (SPEC 1.4, U-04), with an exact linked-session drilldown
(U-07). It binds the accepted Stage B1 transport (`prs_analytics`,
`prs_sessions`) and shows the Rust core report as read. It is the selected
window only: weekly groups and previous-window deltas (the rest of M-11) are
not part of it, and nothing on it is a cost or an authorship claim.

## What is read, and when

- `pullRequestAnalytics(days, confirmedOnly)` under
  `['metrics', 'pr-analytics', days, confirmedOnly]`, for the Shell's range
  and the page's Confirmed only switch. `/prs` is a windowed route: the TopBar
  range control and period show here (not on the inventory view).
- `pullRequestSessions(request, after)` under
  `['sessions', 'pr-linked', repository, number, days, windowEndMs, confirmedOnly]`
  when a drilldown is open. The request is derived on every render from the
  report the page displays (`window.days`, `window.end_ms`, `confirmed_only`)
  and the chosen canonical identity; it is never stored. A newly read report
  (a new anchor after any committed change, another range or mode) therefore
  replaces the drilldown whole under a new key from its first page, and a late
  answer for another pull request or anchor is never painted.
- Mount, range, mode, filter, retry and opening a drilldown are local reads.
  Nothing refreshes a pull request, reads a transcript or source, or contacts
  GitHub; no link leaves the app.

## What is shown

- **Tiles** (tokens, human messages, agent time, hands-off per PR): the
  report's `median` when it is published, with its n, the PRs that gave it a
  value (`n = N PRs`, or `n = m of N PRs` when some had no sample); otherwise its
  `measured_median`, labelled `measured PRs · m of N`; otherwise a dash whose
  reason states the report's counts. The definition popover carries the
  eligible/measured/unknown/no-sample counts, the selected window, "not
  weekly" and the overlap wording. Token medians withheld by the fixed
  14-day gate say so with the gate's own coverage and span; row totals and
  the other medians are not gated. With no merged PR the tiles say that
  instead.
- **Rows** (newest merge first, the report's order reversed): title or
  `Title not cached` over the canonical `owner/repo#number` (text, not a
  link); work type or `unresolved` (never `other`); merge day and time in the
  report's zone, `stale` after a failed refresh that kept earlier facts;
  linked sessions with the active-in-range count; human messages; the
  four-counter token total (every counter and the measured/no-usage/
  incomplete session counts in its title); agent time; pooled hands-off with
  its stretch n and `excl` when a member's surface was excluded; the strongest
  evidence with `mixed` or `all N` and the full per-session mix spoken and in
  the title. Unknown values are dashes with their reason; measured zeros are
  `0`.
- **By type**: each type's per-PR medians of messages, agent time and
  hands-off, each with its own sample: `*` marks a measured-PRs-only value,
  `m/n` shows when not every PR of the type gave a value, and the metric's
  measured/eligible/unknown/no-sample counts are its title and spoken text.
- **Hands-off without a median** is a known absence only when no member was
  excluded; with an excluded member it is unknown and names the surfaces,
  also beside an unknown classification.
- **An open drilldown follows the same report.** While the report is being
  read or its read has failed, the drawer keeps its pull request selected but
  shows no report totals and no member rows, says the report could not be
  read, and offers Retry; its query is disabled, so nothing is read for it
  meanwhile. The recovered report's own pinned key brings the members back
  from their first page. Focus returns to the control that opened the
  drawer, or to the one the recovered table drew in its place.
- **Report states**: while the report is read, tiles and the type panel say
  `Reading the report…`; after a failed read they say `Not available: the
report could not be read` beside the table's Retry, with no eligibility
  line and nothing previously read; a successful empty read with unknown facts
  says `No known merged PR to group; …`.
- **Eligibility line**: merged in range, outside, unknown cached facts,
  unresolved type and excluded hands-off surfaces, with a button to the
  Cached inventory. When no merged PR is known but facts are unknown the table
  says exactly that, never "nothing merged".
- **Drilldown**: opened from the sessions count or the token total (also when
  the total is unknown). Close is its first control; Escape or Close returns
  focus to the opener and leaves filter, mode and range as they were.
  It states the U-07 banner, the report window and mode, and the report row's
  own values beside 50-row pages of members in the Sessions list order (each
  once, idle members included, the selected pull request's own link evidence
  per member); `Show 50 more` follows the returned cursor over the same
  window. Rows are never added up. A member is named by the title the index
  saved, else its identity: the drawer reads no host title and shows no
  sub-session parent marker, both of which stay on Sessions, and a committed
  sub-session relation does not re-read it.

The filter narrows rows only; the meta line says `n of N PRs shown · medians
cover all`.

## Test data

`apps/desktop/ui/src/app/prs-analytics.synthetic.json` is written by
`apps/desktop/src-tauri/tests/pr_analytics.rs::ui_export` through the
commands' own assemblers over a seeded synthetic database at F1's pinned
instant, and byte-checked there (regenerate with
`XTRACE_WRITE_PR_SYNTHETIC=1`). It holds a measured scenario (overlap, idle
inferred member, judge link, measured zeros, unknown classification,
unresolved type, excluded surface, no stretch, stale facts, a 53-member PR
over two pages, 14/30-day-only row, open/never-refreshed/failed facts) and a
sparse one (no known merge at 7/14 days with unknown facts; a failing token
gate). F1's own report stays empty because its merge is at the window's
exclusive end.

## Tests

- `src/app/PrAnalytics.test.tsx`: window, overlap wording and range; measured
  versus complete medians and differing samples; every row state; row-only
  filter; Confirmed only and range reads; no known merge with unknown facts
  and the inventory; gate withholding; failed read and Retry; exact pinned
  drilldown request, members and evidence, keyboard close with focus return;
  unknown-token entry and idle member; paging with the returned cursor;
  out-of-order answers across pull requests; a refreshed report's new anchor
  restarting the drilldown at page one; an open drilldown suppressing report
  totals and members while the report is unavailable, and recovering on Retry
  under the recovered anchor with focus returned.
- `src/app/PrAnalytics.adapters.test.tsx`: fixture and native adapters draw
  the same tiles, rows, type panel and drilldown, with only the report's IPC
  commands; F1's empty report worded as unknown facts.
- `src/app/pr-analytics.test.ts`, `src/app/session-cells.test.tsx`: wording
  helpers and the cells shared with the Sessions list.
- `e2e/prs-analytics.spec.ts` (WebKit and Chromium, light and dark, 1440×900
  and 1120×720): no outlet or table sideways scroll, fixed 40 px rows, no
  clipped value, sample or header, no untitled clipped text, no console error
  or external request; keyboard drilldown bounded in the window with a
  focusable rows region; unknown-token entry; the sparse case and its route to
  the inventory and back.

Not claimed: comparison with the current live design, native Tauri layout, or
read latency on a real index; weekly M-11 grouping and deltas.
