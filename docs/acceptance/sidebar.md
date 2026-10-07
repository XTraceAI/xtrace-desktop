# Sidebar and Hub panel acceptance

The sidebar, Hub panel and brand mark render from supplied props without a router
or data service. The current app entry point remains the existing desktop shell;
the component fixture demonstrates the new controls independently.

## Verification

- Typechecking, ESLint, token-color lint and the production build pass.
- All 20 Vitest tests and the Node color-lint regression pass. Component tests
  cover controlled Sidebar/Hub/Brand content and Base UI portal/theme lifecycle.
- `pnpm e2e` covers the sidebar in Chromium and WebKit, plus appearance, fonts,
  popup dismissal, moving anchors, clipped ancestors and nested modal focus.
- The sidebar scenario reports no browser console errors or page exceptions.

Run `pnpm e2e --grep sidebar` to repeat the two focused browser tests. Browser
executables must first be installed with
`pnpm --dir apps/desktop/ui exec playwright install webkit chromium`.

## Cases and expected results

| Case                    | Setup and action                                                                                       | Expected and observed result                                                                                                                                                                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controlled navigation   | Supply an active item and action spies; click enabled items and rerender with another active key.      | Observe/Govern/Community contain five default items. Enabled clicks call the handler; selection changes only with the supplied key. The Rulebook badge appears for a positive count and disappears at zero.                                              |
| Disabled actions        | Leave Leaderboard, Hub connection and Settings unavailable; vary Team visibility and connection props. | Leaderboard shows “soon”. Unavailable actions are disabled. Team is hidden by default and becomes actionable only when shown and connected.                                                                                                              |
| Measurements            | Supply zero, null and positive token counts, then no measurements.                                     | Zero displays `0` with no bar; null displays `—` with no bar. Positive values use compact labels. Empty measurements have an explicit message.                                                                                                           |
| Capture coverage        | Supply capturing CLI, uncaptured Desktop, unfamiliar and missing identifiers; open plugin status.      | Each row retains its own status. Missing coverage stays unknown; listener status does not certify capture.                                                                                                                                               |
| Footer                  | Change listener, version, update, settings and appearance props.                                       | Labels follow supplied values; status text and the two controls share one footer row. Long version text truncates with its full value in a title. Only supplied actions run; no “up to date” status is inferred.                                         |
| Hub interaction         | Open with Enter, dismiss with Escape, reopen and use the close button, then reopen and click outside.  | The popover opens and dismisses correctly, returns keyboard focus to its invoker and can reopen repeatedly. The disconnected CTA is disabled without a handler; supplied connection state controls the title and CTA.                                    |
| Layout                  | Render a 228×900 sidebar, then reserve a 74px top inset. Open the Hub panel.                           | The sidebar is 228px wide with 16px default top padding and 32px navigation rows. The Hub is 264px wide, 14px beyond the footer content edge and 6px below the status row at its bottom edge. The configured inset reserves native window-control space. |
| Appearance and branding | Render dark and light themes with bundled fonts and 20/22/26/34px marks.                               | Shared theme colors apply to the sidebar and Hub. The approved XTrace image retains its geometry at each size.                                                                                                                                           |

## Local index status row

In the app the status row describes the **local index**, not the optional plugin
receiver. The Shell passes `localIndex`, worded by the pure presenter
`src/app/sidebar-index-status.ts` from the typed `native_index_status` the pages
already read, and keeps the receiver as its own line in the panel. The gallery
stories, the component fixture and the visual baselines pass no `localIndex`, so
their plugin row and “Capture by surface” panel are unchanged.

| Case                         | Synthetic setup                                                                                                                                             | Expected and observed result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Ready, watched, receiver off | Phase `ready`, freshness `live`, every host scan complete; `listening: false`.                                                                              | `index · updating` with the green dot. The panel says “Plugin receiver · Off” and “Plugin delivery · Unknown”. Nothing says “not capturing” or “not installed”, and indexing is not described as depending on the plugin.                                                                                                                                                                                                                                                                                                                                    |
| Checking / scanning          | No status yet; then phase `scanning` with pending hosts.                                                                                                    | `index · checking`, then `index · scanning`, both with the plain dot. Hosts the initial scan has not reached read “Waiting to be read” and are not listed as gaps.                                                                                                                                                                                                                                                                                                                                                                                           |
| Watching not established     | Phase `ready`, freshness `unknown`.                                                                                                                         | `index · ready`, plain dot; the summary says new activity may not appear yet.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Degraded watcher             | Phase `ready`, freshness `degraded` with a reason.                                                                                                          | `index · degraded`, warning dot, title “Updates interrupted”, and the reported reason.                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Partial or failed host scan  | Ready and live with a host the app marks `needs_attention`: `incomplete`, `reader_failed`, `missing_runtime`, `cancelled`, a host left `pending` and so on. | `index · partial`, warning dot, title “Updating · partial”. The sidebar reads the app's `needs_attention` and the shared host labels (“Read with gaps”, “Reader failed”, “Needs Python 3”, …), never its own rule. A note names each host and state and says the index may be incomplete or out of date for them, never that stored history is missing: a scan that did not complete says nothing about what earlier scans stored. Those rows use the warning colour and show the reported reason. A live watcher never keeps the green dot over a host gap. |
| Absent source                | A host's last scan is `missing_source`.                                                                                                                     | Its row reads “No local history found” as a plain value. It is not a reader failure and does not qualify the index; it is never called complete.                                                                                                                                                                                                                                                                                                                                                                                                             |
| Stopped / disabled           | Phase `stopped`; phase `disabled` with a reason (the F1 fixture's own state).                                                                               | `index · stopped` / `index · disabled`, warning dot, with the reported reason for a disabled index.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Initial read fails           | The first status read rejects.                                                                                                                              | `index · unknown`, title “Status unavailable”, no host rows, and no backend error text.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Stale cached status          | Listener registration fails, then Reconnect fails again, then succeeds.                                                                                     | While events are not heard the row reads `index · updating?`, named “Last known: Updating”, with the warning dot and a note that the status may be out of date; the existing notice and Reconnect are unchanged. Once events are heard it stays last known until a status read that began since has succeeded, then returns to `index · updating`.                                                                                                                                                                                                           |
| Receiver states              | `listening` true, false, and unread app metadata.                                                                                                           | “Listening”, “Off”, “Unknown”. No port is shown: the app reports none. A listening receiver is described as not proving plugin delivery.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Coverage                     | The Shell reads no capture coverage.                                                                                                                        | “Plugin delivery · Unknown”. Host scans are never shown as capture surfaces, no surface is listed, and an empty list is not zero coverage.                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Browser preview              | A source of kind `preview`.                                                                                                                                 | Reads no index, so it keeps the plain `plugin · unknown` row.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

Requests. The Shell observes the pages' cached status query passively
(`useNativeIndexObservation`): same key, command and event invalidation, with no
timer, listener or reconciliation of its own, and it selects only the facts the
row shows. `src/app/ShellIndexStatus.test.tsx` counts every read: one status
read on a route with no status reader and none on remounts between such routes;
no Dashboard, Environment or counts request made for the sidebar; the runtime's
listeners only; no polling from the sidebar during ten seconds of a transient
status; on Settings, one shared first read and then exactly the page's own
one-second polling (three polls in three seconds); one read per status event;
and no Shell render for scan progress the row does not show. Recorded tokens,
their range caption and the share bars are asserted unchanged.

Keyboard and geometry. `e2e/sidebar-index-status.spec.ts` runs the real Shell
over test-only synthetic statuses in Chromium and WebKit, light and dark, at
1440×900 and 1120×720. The row stays one line inside the footer, left of the two
controls, with its words drawn whole, including a 17-character probe for the
widest state the presenter can word (a last-known state); the sidebar stays
228px with no horizontal overflow. The row takes a visible focus ring, Enter
opens the 264px panel beside the sidebar and wholly inside the window with the
page width unchanged, a reason with a 120-character unbroken run wraps inside
it, Escape closes it and returns focus to the row. “Index details in Settings”
is focusable with a visible ring and Enter opens Settings, where the full
diagnostics remain.
The index panel and the Hub panel never stay open together. Dot colours follow
the theme's success, warning and meta tokens.

Reconnect recovery. Registered listeners mean events are heard from then on;
they do not mean the status shown was read since. After a successful Reconnect
the row stays last known, with “Live updates are back and the status is being
read again”, until a status read that began after the listeners registered has
been accepted. A read that began before them can answer after them, so its
answer is shown but does not count; a catch-up read that fails stays last known.

The runtime raises a per-client count (`heardEpoch`) when its listeners
register, synchronously, before its catch-up reads begin and before `connected`
is announced. `connected` still means only that every event is heard, and
nothing is read, timed or polled for this. Each status read notes the count it
finds when it begins and marks its own copy of the answer with it. The mark
moves onto the cached object where the cache accepts a result, so a failed or
cancelled read and a manual cache write never carry one, and an identical
answer from a later count becomes a new object, so it still makes the status
current. Because the mark is on the cached result, nothing has to be listening
when a read succeeds: a sidebar that mounts later reads it from the cache.

`src/app/ShellIndexStatus.test.tsx` holds the status read after the listeners
register and checks what every Shell render showed, with every read counted:
deferred then changed; rejected, then recovered by the next event's read; a
first read still out at the reconnect, whose identical follow-up answer makes
the status current; a cached status with an older read still out; an identical
catch-up answer; a manual write (identical, then changed) and a cancelled read
that answers late, neither of which makes it current; the Shell remounting on a
cached result with nothing read, including a catch-up accepted while no Shell
was mounted and one still out at the remount; a reconnect with no status reader
mounted, where the remounting Shell's own read is the one that counts; and a
replaced source, whose runtime has its own count and never sees the old one's
late answer. An ordinary status event's read, held the same way, never draws
“?” and renders nothing. `src/data/heard-epoch.test.ts` checks the count itself:
raised once per complete registration, before the catch-up read begins and
before `connected` is announced, never by a failed attempt, and per client.

Limits. The words and tones are this slice's display choices over existing
facts, reviewed against SPEC O-11/O-12 and not against a live design export.
`degraded` is used on the row because “interrupted” does not fit beside the
controls; the panel title says “Updates interrupted”. Per-surface native health
and receipt-based plugin coverage are not read here. No native window, live
history or installed app was used: every status above is synthetic.

## Visual reference

The clean design-only images below were rendered from the sidebar design source
captured on 2026-09-09, with its own layout/styles and the same bundled fonts,
WebKit, 228×900 viewport and 2× pixel density used for the implementation.
They are independent reference renders, not copies of the implementation.
Compare matching themes at the same scale:

| Theme | Design reference                      | Implementation                           |
| ----- | ------------------------------------- | ---------------------------------------- |
| Light | [Reference](sidebar/design-light.png) | [Implemented](sidebar/sidebar-light.png) |
| Dark  | [Reference](sidebar/design-dark.png)  | [Implemented](sidebar/sidebar-dark.png)  |

Check the 22px mark, SVG paths, host logos, navigation spacing, 1.45 line height,
usage-card layout, full-width 204×30 cloud row and the status/control alignment.
Browser assertions check key dimensions, loaded local assets and footer bounds;
reference comparison remains a visual review, not a claim of pixel equality.

Cursor’s dark logo uses a fixed light backing in both themes; browser checks verify it remains light after theme changes.

Intentional functional differences: the plugin row opens per-surface capture
details instead of displaying them permanently; long version text truncates
instead of pushing controls outside the footer; absent actions remain disabled.
The sample reset caption and update status are fixture props, not product defaults.
Hub and capture panels use Base UI keyboard dismissal and focus return; switching
between them is covered in both browser engines. The Hub CTA uses the accessible
shared foreground token in dark mode. Native shells still reserve their window
controls through the existing inset prop.

## Evidence

- [Dark sidebar, 228×900](sidebar/sidebar-dark.png)
- [Light sidebar, 228×900](sidebar/sidebar-light.png)
- [Light Hub panel](sidebar/sidebar-hub.png)
- [Brand sizes and reserved native inset](sidebar/brand-sizes-native-inset.png)
- [Component props and behavior](../SIDEBAR.md)

The screenshots use synthetic fixture data and emulated appearance in WebKit;
they contain no personal desktop content or real measurements.

## Integration and platform limits

The shared Popover delegates positioning, dismissal and focus restoration to
Base UI. Its optional element-valued positioning anchor changes geometry while
the registered trigger remains the focus-return target. Browser tests replace
both trigger and separate anchor nodes while open and verify position and focus
return. State-changing outside clicks stay dismissed and can later reopen.
The sidebar adds no document-level Escape, click or positioning listeners.

The caller supplies navigation, real measurements, shared icons and connection
actions. These components neither connect to Hub nor change routes by themselves.
Native controls and drag regions belong to the desktop shell.

Exact-source macOS build and launch evidence is recorded in the PR. Browser
tests and a launch on newer macOS do not establish the macOS 14 support floor;
that qualification remains part of downloadable-release validation.
