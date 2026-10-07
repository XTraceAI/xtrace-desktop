# Cached pull-request inventory

The `/prs` page's **Cached inventory** view (`/prs?view=inventory`) lists every
stored pull request that an indexed session still links, with the facts the
last manual refresh left in storage. It is the second view of the PRs page,
beside the merged-PR report ([pr-analytics.md](pr-analytics.md)), and is
independent of it: lifetime link counts of every confidence, never the
selected range or Confirmed only, never joined to the report's rows, and no
report value is derived from it. The one line that stays in view says so; a
native `<details>` beneath it explains what is listed and counted, and links
to the Dashboard, where the manual refresh lives. The Shell shows no range
control on this view.

## What is read, and when

- One read: `DataSource.pullRequests()` (`prs_list`) under the existing
  `queryKeys.pullRequests` key that the Dashboard's refresh dialog also uses.
  The rows are shown in the order storage returns them (repository, then
  number); nothing is sorted, joined or calculated in the browser.
- The list is read when the view mounts, again when the runtime's existing
  event map re-reads it after a committed `prs://refreshed`, again after a
  committed import, turn, backfill or host connection, or a settled index
  status (committed indexing can add links; a `scanning` status reads
  nothing), all coalesced in the same 500 ms window as every other event, and
  again by the runtime's reconnect catch-up. A batch that stored nothing emits
  nothing and reads nothing. A read still running when a change arrives is
  followed by exactly one read that began after it (`refreshQueries`,
  unchanged). The page names these reads.
- Retry after a failed read is a local read of storage again. Mounting,
  focusing, filtering, retrying and reconnecting never invoke `prs_refresh`,
  `prs_refresh_cancel`, any session, transcript, Dashboard, Environment, index
  or database command. Unit tests trap every one of those seams and the native
  test asserts the exact IPC command set (`app_info`, `tokens_by_host`,
  `prs_list`). Nothing here polls, refreshes on focus or on a timer, or
  contacts GitHub; the page holds no external link.
- The Shell shows no range control on this route (already the case) and the
  page asserts it: no `Date range` radiogroup and no report period, while the
  sidebar's own token caption keeps the chosen range.

## What each cell says

Every value is a `PrRow` field as read. Null is never drawn as zero, open or
unmerged:

| Field                   | Cached                                                                                                                 | Not cached                                                    |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| title                   | the stored title                                                                                                       | `Title not cached`, muted, never styled like a title          |
| state                   | the stored word with its tone; `merged` adds the stored merge time, or `merge time not cached`                         | `—` with the reason "not known to be open, closed or merged"  |
| merged_at without state | never shown as merged: state decides                                                                                   |                                                               |
| additions/deletions     | `+n` over `−m`, exact and grouped, never compacted; a cached `0` is `+0` / `−0`; the whole value is the cell's tooltip | `—` when both are null; one null side is `−—` with its reason |
| head_ref_name           | `⑂ branch` under the identity, whole in the tooltip                                                                    | omitted                                                       |
| linked_sessions         | the count (retained links, all confidences, all time); `0` is shown as `0`                                             |                                                               |

The identity `owner/repo#number` is text, never a link; the canonical URL is
the tooltip. The cache status column uses the app's one check-status wording
(`refreshStateLabel` / `refreshStatusWords`, the same words as the Merged PRs
tile, the refresh dialog, the effort chart and the PRs report) and the shared
`refreshErrorText` codes: `not checked yet`; `checked` with its time;
`could not be checked` with the code and the attempt time; `stale after a
failed check` with the code and the time of the facts that are
still shown, beside them, exactly as stored. `checked` is toned `info`, never
`success`: storage applies no age policy and the page claims no freshness.
Times are this Mac's local zone, written by the app's one clock format (this
Mac's locale and 12- or 24-hour setting) with the day and the year, because cached facts have no age limit; there is no report
window on this page and none is fabricated to borrow a zone from. A stored
instant a `Date` cannot hold is stated as unreadable, not thrown on.

## Filter and scrolling

The filter box narrows the rows already read by a case-insensitive substring
of the title, repository or number; it never asks storage for anything, and
the card's meta line reads `n of N shown` while it is in effect. ⌘K reaches it
through the Shell's existing shortcut. The rows take the page's remaining
height and scroll in their own focusable region under a sticky header, as the
Sessions list does; rows hold nothing that takes a tab stop, so the page has
three: the filter, the explanation's summary and the rows' region.

## Tests

- `src/app/PrsPage.test.tsx`: a separate lifetime view with no analytics or range;
  no re-read on focus or over ten idle minutes; fixture and native adapters
  agree cell for cell in the initial, partially refreshed, failed and complete
  cache states with the export unmutated; zero versus unknown; all four
  statuses with earlier facts beside a failure; unreadable stored values;
  empty list; failed read, no backend detail, Retry as one local read; browser
  preview; local filter over 600 rows without another read; scroll region and
  header; the explanation and its one in-app link.
- `src/app/PrsPage.events.test.tsx` (fake timers, held reads): one re-read per
  committed refresh and none for an unchanged batch or a 20-event burst; one
  coalesced re-read for committed ingest and a settled index status, none for
  a `scanning` status; a first read that began before the commit followed by one
  that began after it; a failed re-read shown as a failure and retried
  locally; a change missed while disconnected reconciled on Reconnect; and a
  read that predates the reconnect followed by exactly one that began after
  it, with nothing shown as loaded in between.
- `e2e/prs.spec.ts` (Chromium and WebKit, light and dark, 1440×900 and
  1120×720): all six cache states at 40 px rows with no clipped fact, every
  size line (five- and seven-digit counts) whole, stacked and inside its
  column with the whole value on the cell, no horizontal page or table scroll, tones taken from the theme's tokens, the
  explanation opened from the keyboard inside the page; a 600-row list
  reached in three tab stops and scrolled by keyboard with the header pinned
  and the outlet still; the browser fixture's own refresh made on the
  Dashboard reflected here without any refresh started from this page.

Not claimed: comparison with the current live design, and native Tauri
layout and read responsiveness on a real index.
