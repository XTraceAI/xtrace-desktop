# Appearance and overlay acceptance

Users can choose System, Dark or Light appearance from the app header. The choice
persists across launches. Shared pop-up menus and dialogs inherit the surrounding
colors and provide keyboard dismissal and focus handling for later screens.
The demonstration overlay page is used only by browser tests.

## Cases and expected results

| Case                       | Setup and action                                                                                             | Expected result                                                                                                                                                                             |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Appearance and persistence | Start in System mode; change the system color scheme, choose Dark/Light, then reload or restart.             | System follows the current scheme. An explicit choice stays selected across reload/restart and does not follow later system changes.                                                        |
| Unavailable storage        | Deny preference reads/writes and change appearance.                                                          | The app still renders and changes appearance for the current session.                                                                                                                       |
| Shared colors              | Compare computed dark/light values with `design/token-contract.json`; render forced light and dark sections. | All keys and values match. Components and sections keep their intended colors when the page theme changes.                                                                                  |
| Offline fonts              | Block external requests, load all eight font weights, then disable networking.                               | Every weight loads from local `/fonts/` assets and remains usable offline. All 16 WOFF2/TTF files reproduce from pinned source hashes.                                                      |
| Pop-up menus               | Open using mouse and keyboard; scroll; dismiss outside or with Escape; reopen repeatedly; remove the anchor. | The menu keeps its local colors and 14px test offset, remains within the viewport, closes correctly and returns lost keyboard focus to its trigger. An outside focused control keeps focus. |
| Dialogs                    | Open a dialog; use Tab/Shift+Tab; attempt to click a background action; close by Escape or its close button. | Focus stays inside while open, the background is inactive, local colors persist and focus returns to the invoker. Unmounting also restores focus.                                           |
| Cleanup                    | Mount/unmount under React StrictMode and let native subscriptions finish after unmount.                      | Event listeners and observers are cleaned up; stale callbacks do not update unmounted state.                                                                                                |
| Color consistency guard    | Add a raw hex color to a scratch component outside the approved color file, then run `pnpm lint`.            | Lint fails with a source location. The approved color file and test files are accepted.                                                                                                     |
| Native desktop             | Change appearance in the real app, restart, restore System, and run the local native validation command.     | The desktop renders both appearances, retains the saved choice and still loads native app information. Record the actual macOS version and any pending interaction below.                   |

## Recorded verification

- `pnpm check`: typechecking, lint, formatting, 20 UI tests and one color-lint
  regression passed.
- `pnpm build`: production build passed with the Safari 17 target and local-font
  policy.
- `pnpm e2e`: all 20 browser checks passed in Chromium and WebKit, including the
  existing app startup check and sidebar and expanded overlay cases in each engine.
- Font reproduction with FontTools 4.64.0, Brotli 1.2.0 and Zopfli 0.4.3 matched
  every pinned byte. The 16 files total approximately 1 MB.
- Native interaction on macOS 26.5.2 arm64: Dark and Light rendered correctly;
  Dark remained selected after quitting and reopening. Restoring System matched
  the Mac's current Light appearance, and native app information loaded.
- Live native System-mode check: changing macOS from Auto (currently Light) to
  Dark changed the running app to Dark. Restoring macOS Auto changed the app back
  to Light without restarting it. The app remained set to System throughout.

The live macOS check, browser system-change checks and native subscription
adapter tests pass.
The exact source/base and full local native command results are recorded in the PR.

## Screenshots and font requests

These captures were refreshed using Playwright 1.58.2 / WebKit 26.0. They contain
synthetic app/demo content and are browser screenshots.

- [Dark appearance](appearance/dark.png)
- [Light appearance](appearance/light.png)
- [Light pop-up menu on a dark page](appearance/popover-light-subtree.png)
- [Light dialog on a dark page](appearance/modal-light-subtree.png)
- [Local font requests](appearance/offline-font-network.json)

## Compatibility limits

Playwright is pinned to 1.58.2 because [version 1.59 removed macOS 14 WebKit
support](https://playwright.dev/docs/release-notes#version-159). A compatible test
runner does not certify the native operating-system floor.

The shared popover/dialog wrappers use Base UI 1.8.0 with fixed positioning
and scoped theme portals. Chromium/WebKit checks cover focus containment,
Escape/outside dismissal, replacement anchors, transformed/clipped parents and
nested dialogs. The build retains its Safari 17 target; this does not prove all
library behavior on the minimum OS.
Native release qualification on macOS 14 remains part of preparing a downloadable
release; the local desktop interaction above was performed on macOS 26.5.2.
