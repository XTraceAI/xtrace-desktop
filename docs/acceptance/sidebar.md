# Sidebar and Hub panel acceptance

The sidebar, Hub panel and brand mark render from supplied props without a router
or data service. The current app entry point remains the existing desktop shell;
the component fixture demonstrates the new controls independently.

## Verification

- Typechecking, ESLint, token-color lint and the production build pass.
- All 22 Vitest tests and the Node color-lint regression pass. Seven component
  tests cover Sidebar, HubPopover and BrandMark; the shared Popover tests also
  cover detachment of its optional positioning target.
- `pnpm e2e` passes all 14 tests using pinned Playwright 1.58.2 with Chromium and
  WebKit. This includes the sidebar scenario in both engines and the inherited
  appearance, font-loading, popover, modal and shell tests.
- The sidebar scenario reports no browser console errors or page exceptions.

Run `pnpm e2e --grep sidebar` to repeat the two focused browser tests. Browser
executables must first be installed with
`pnpm --dir apps/desktop/ui exec playwright install webkit chromium`.

## Cases and expected results

| Case                    | Setup and action                                                                                       | Expected and observed result                                                                                                                                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controlled navigation   | Supply an active item and action spies; click enabled items and rerender with another active key.      | Observe/Govern/Community contain five default items. Enabled clicks call the handler; selection changes only with the supplied key. The Rulebook badge appears for a positive count and disappears at zero.                                       |
| Disabled actions        | Leave Leaderboard, Hub connection and Settings unavailable; vary Team visibility and connection props. | Leaderboard shows “soon”. Unavailable actions are disabled. Team is hidden by default and becomes actionable only when shown and connected.                                                                                                       |
| Measurements            | Supply zero, null and positive token counts, then no measurements.                                     | Zero displays `0` with no bar; null displays `—` with no bar. Positive values use compact labels. Empty measurements have an explicit message.                                                                                                    |
| Capture coverage        | Supply capturing CLI, uncaptured Desktop, unfamiliar and missing surface identifiers.                  | Each row retains its own status. Missing coverage stays unknown; listener status does not certify capture.                                                                                                                                        |
| Footer                  | Change listener, version, update, team, settings and appearance props.                                 | Labels follow the supplied values. Only supplied actions run; no “up to date” status is inferred.                                                                                                                                                 |
| Hub interaction         | Open with Enter, dismiss with Escape, reopen and use the close button, then reopen and click outside.  | The native popover opens and dismisses correctly, returns keyboard focus to its invoker and can reopen repeatedly. The disconnected CTA is disabled without a handler; supplied connection state controls the title and CTA.                      |
| Layout                  | Render a 228×900 sidebar, then reserve a 74px top inset. Open the Hub panel.                           | The sidebar is 228px wide with 16px default top padding and 32px navigation rows. The Hub is 264px wide, 14px beyond the sidebar and 6px below the invoking button at its bottom edge. The configured inset reserves native window-control space. |
| Appearance and branding | Render dark and light themes with bundled fonts and 20/26/34px marks.                                  | Shared theme colors apply to the sidebar and Hub. The approved XTrace image retains its geometry at each size.                                                                                                                                    |

## Evidence

- [Dark sidebar, 228×900](sidebar/sidebar-dark.png)
- [Light sidebar, 228×900](sidebar/sidebar-light.png)
- [Light Hub panel](sidebar/sidebar-hub.png)
- [Brand sizes and reserved native inset](sidebar/brand-sizes-native-inset.png)
- [Component props and behavior](../SIDEBAR.md)

The screenshots use synthetic fixture data and emulated appearance in WebKit;
they contain no personal desktop content or real measurements.

## Integration and platform limits

The shared Popover owns native dismissal and focus restoration. Its optional
positioning reference affects geometry while the original invoking button remains
the focus-return target. Detaching either target dismisses the panel. The sidebar
adds no global Escape or outside-click listeners.

The caller supplies navigation, real measurements, shared icons and connection
actions. These components neither connect to Hub nor change routes by themselves.
Native controls and drag regions belong to the desktop shell.

Exact-source macOS build and launch evidence is recorded in the PR. Browser
tests and a launch on newer macOS do not establish the macOS 14 support floor;
that qualification remains part of downloadable-release validation.
