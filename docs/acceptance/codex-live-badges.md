# Codex live badges — UI handoff

The UI shows the exact desktop runtime state for a Codex chat: Running,
Waiting for approval, Waiting for input, Idle, or Unknown. The badge carries
the full textual label and a short tooltip naming the Codex desktop runtime.
Running uses the existing green theme colors; both waiting states use the
existing warning colors; Idle and Unknown use neutral colors.

## UI-owned files

- `apps/desktop/ui/src/app/live-session-status.ts`
- `apps/desktop/ui/src/app/live-session-status.test.tsx`
- `apps/desktop/ui/src/app/LiveSessionBadge.tsx`
- `apps/desktop/ui/src/styles/live-session-status.css`
- `apps/desktop/ui/src/styles/sessions.css`
- `apps/desktop/ui/src/app/SessionsPage.tsx`
- `apps/desktop/ui/src/app/SessionsPage.test.tsx`
- `apps/desktop/ui/src/app/dashboard/ActivityLanes.tsx`
- `apps/desktop/ui/src/app/dashboard/ActivityLanes.live.test.tsx`
- `docs/acceptance/codex-live-badges.md`

## Displayed chats and read limits

- Sessions has one status hook. The eight displayed recent sessions take
  priority, considering Codex chats only. Remaining slots go to Codex rows
  actually inside the All sessions table's scroll area, reported by the
  existing `useVisibleIds` helper on their name cells.
- Dashboard has one status hook. It reuses the existing visible set and asks
  only about returned Codex lane rows. A synthetic row naming a parent with no
  returned lane has no badge and is never requested. Expanding a group lets
  its visible children request their own state.
- Each hook removes duplicate exact IDs and caps its selected set at 16.
  Priority is applied before sorting the selected IDs into a stable request
  key. Reordering the same selected set does not reopen the subscriptions.
- An unobserved or over-cap Codex row shows Unknown when the capability is
  present. A missing snapshot row is Unknown, never inferred Idle. There is
  no status enumeration over history and no state inferred from recency.
- Fixture and preview sources without `liveSessions` show no live badges.
  Other hosts have no badge.

The recent section still describes indexed activity in the last 48 hours,
shows at most eight sessions, reports the existing count and truncated-span
limit, and keeps full IDs and title tooltips. Search, filters, paging, sorting,
metrics, and the existing shared title queue are unchanged. Its new sentence
distinguishes recorded history from desktop runtime state.

## Polling and cleanup

The optional DataSource method is `liveSessions.read(ids, viewId: string | null)`.
Every snapshot returns `{ view_id, states }`. On entering a nonempty selection,
the UI first calls `read([], null)` to register an empty lease. Native issues
the token. Only a still-current registration may then call
`read(selectedIds, issuedToken)`. Registration states are never displayed.
The UI generates no lease IDs and keeps no tombstones.

Later polls call `read(selectedIds, issuedToken)` every two seconds using the
same token. A hook allows at most one read chain in flight, including a pending
registration or status read from a replaced scope. Changing the scope or
selected set clears old states and registers a fresh empty lease after any
old chain finishes. Reordering the same selected set retains its lease.

Read errors clear all previous states, including Running, release the captured
old token, and discard it. The next timer poll explicitly registers a fresh
empty lease and then reads the selected IDs. Native rejects expired or released
tokens; the UI never tries to recreate them. Missing or invalid states become
Unknown. A selected-read reply with a different token is not adopted.

Navigation, filter/range changes, selected-row changes, and unmount release
only their captured token. A registration that arrives after navigation or
replacement releases its issued token immediately without any selected-ID
read. A late old status read is discarded and releases its own old token
again. Neither cleanup path releases a successor. Repeated releases are
harmless; failed releases are caught. The native worker owns its 15-second
lease fallback. Source-factory capability gating and its fixture checks belong
to the native owner; UI tests inject fake controls explicitly.

## Verification

Focused UI tests cover:

- Running → both waiting states → Idle; Running → read error → Unknown.
- Duplicate IDs, priority/cap 16, missing and unexpected states, unsolicited
  reply IDs, one pending read, stable leases across reordered sets, replaced
  answers, empty selections, and StrictMode cleanup.
- Empty native registration before a selected-ID read, no state displayed from
  registration, late registration released without subscription, serial
  replacement chains, and native expiry errors followed by fresh registration
  on the next poll.
- An explicitly named old-start Codex recent chat shows Running under its
  exact ID even when it is absent from the first All sessions page.
- Recent8 consumes the first slots; visible table rows consume the remaining
  slots; unobserved and over-cap rows are Unknown; scroll/filter changes
  release old views; navigation discards both late registration and late
  selected-read answers.
- Returned Dashboard parent/child rows keep their own states; absent parents
  have no badge or status request; collapsing children and changing the range
  release the old view.
- Missing capability creates no badge on either page.
- Existing title and sub-session tests are included as regression checks.

Commands from the shared checkout:

```sh
pnpm --dir apps/desktop/ui test src/app/live-session-status.test.tsx src/app/SessionsPage.test.tsx src/app/dashboard/ActivityLanes.live.test.tsx src/app/session-titles.test.tsx src/app/sub-sessions.test.tsx
pnpm --dir apps/desktop/ui typecheck
```

All 87 tests in those five files passed. UI typecheck, Prettier, `pnpm lint`,
ESLint on the changed UI files, and `git diff --check` passed in the initial
handoff. After the layout fix below, the same 87 focused tests, UI typecheck,
and formatting checks passed again. No app build, installation, native live
socket call, or native runtime test is performed by this UI owner. This is
implementation evidence for independent review, not readiness approval.

## Review fix: recent panel at 1120 × 720

Main's private browser probe injects synthetic states into FixtureDataSource
and replaces F1 reports with nine returned Codex chats. Sessions renders eight
recent chats, including both waiting labels, full IDs, a long title, and the
eight-session count caption. These browser results do not verify real native
runtime states.

Before this fix, the updated nine-chat probe measured 33px Sessions outlet
overflow in Chromium and 29px in WebKit, in both themes. Dashboard measured
0px. The fix puts the count caption beside the existing filter note, with
wrapping when needed. Recent-panel gap changes from 5px to 3px, vertical
padding from 9px to 6px, and list vertical margins from 2px to 1px. Badge text,
titles, IDs, row padding, the 152px recent list, and the 260px All sessions
minimum height are unchanged. No status or title lifecycle code changes.

All eight cases passed after the fix:

| Browser  | Theme | Sessions outlet overflow | Dashboard outlet overflow | Badge text overflow |
| -------- | ----- | ------------------------ | ------------------------- | ------------------- |
| Chromium | Dark  | 0px                      | 0px                       | 0px                 |
| Chromium | Light | 0px                      | 0px                       | 0px                 |
| WebKit   | Dark  | 0px                      | 0px                       | 0px                 |
| WebKit   | Light | 0px                      | 0px                       | 0px                 |

Document dimensions were 1120 × 720 in all eight cases. A separate measurement
of the four Sessions cases confirmed exactly three whole recent rows inside
the unchanged 152px list, eight rendered recent rows in total, full IDs
fitting their cells, and both computed and actual All sessions card height of
260px. The count caption and filter note remain visible. I opened two updated
screenshots: Chromium dark and WebKit light.

Reproduce against main's existing dev server at port 5177:

```sh
node /private/tmp/xtrace-live-badge-browser.mjs
node /private/tmp/xtrace-live-badge-recent-layout.mjs
```

The original probe is unchanged. The measurement helper reuses its synthetic
setup, adds three-row/ID/minimum-height checks, and saves separate screenshots.
Main owns the dev server; this UI owner did not stop or restart it.

Updated original-probe snapshots:

- `/private/tmp/xtrace-live-chromium-dark-sessions.png`
- `/private/tmp/xtrace-live-chromium-light-sessions.png`
- `/private/tmp/xtrace-live-webkit-dark-sessions.png`
- `/private/tmp/xtrace-live-webkit-light-sessions.png`
- `/private/tmp/xtrace-live-chromium-dark-dashboard.png`
- `/private/tmp/xtrace-live-chromium-light-dashboard.png`
- `/private/tmp/xtrace-live-webkit-dark-dashboard.png`
- `/private/tmp/xtrace-live-webkit-light-dashboard.png`

Other viewport sizes and real native runtime behavior remain unchecked by this
UI owner. Main's full UI gate and independent combined review remain required.

## Review fix: native-issued lease lifecycle

This follow-up changes only:

- `apps/desktop/ui/src/app/live-session-status.ts`
- `apps/desktop/ui/src/app/live-session-status.test.tsx`
- `apps/desktop/ui/src/app/SessionsPage.test.tsx`
- `apps/desktop/ui/src/app/dashboard/ActivityLanes.live.test.tsx`
- `docs/acceptance/codex-live-badges.md`

The hook and all UI fake controls now use the native-issued two-phase API
described above. All 93 focused tests in the five test files passed, and UI
typecheck passed against the updated generated contract. Formatting and
`git diff --check` passed for this handoff.

Page markup, CSS, title lifecycle, source factory, DataSource, native code,
and generated files were not edited in this follow-up. The previously checked
152px recent list, three whole recent rows, and 260px All sessions minimum are
preserved. The browser evidence above predates this lifecycle change; this UI
owner did not rerun the browser probe. Main has updated its fake source for
the new API and owns the next eight-case browser run and independent review.
