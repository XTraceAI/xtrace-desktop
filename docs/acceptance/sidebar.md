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
