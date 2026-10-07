# Tray popover acceptance

## Status: off, not ready

The menu-bar item is turned off (`tray::ENABLED` is `false`). On macOS 26 the
system gave neither this item nor a plain AppKit status item from this app a
place in the menu bar, and no product fix is known. The tray is deferred and
unresolved, and nothing on this page has passed native acceptance.

One switch turns off the item, the popover window and the main window's
hide-on-close together, so the app is never left running with no window and no
item to bring it back:

- No `tray` window and no menu-bar item are created. The three tray commands
  refuse every caller, because no window is labelled `tray`.
- The main window's close button closes it. It is the only window, so the app
  ends through the normal `RunEvent::Exit` shutdown, as it did before the tray.
- The Dock icon still restores a minimized main window. After a close it starts
  the app again.

The rest of this page describes the tray as built, for when it is turned on.

## As built

SPEC 1.7. One template icon in the macOS menu bar. A left click toggles a
360×540 popover anchored under the icon. The icon's menu has **Open XTrace
Desktop**, which shows, restores and focuses the existing main window, and
**Quit XTrace Desktop**, which calls `AppHandle::exit` so the normal
`RunEvent::Exit` shutdown still stops the index and closes the database.

The popover is its own webview window (`tray`, route `#/tray`) outside the
Shell. Its capability grants only event listen/unlisten. Its three commands
(`tray_visible`, `tray_hide`, `tray_open_main`) refuse any other window.
Escape or losing focus hides it. The main window's close button hides that
window instead of destroying it; the Dock icon or **Open** brings it back.
Single-instance still focuses the existing main window.

## What it shows

`today_summary` captures one clock and zone. It reads M-04 tokens and cost and
M-05 active spans in one `MetricsDb` snapshot over `[local midnight, now)`. It
defines no metric of its own.

| Figure                          | States                                                                                                                                                                 |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Output tokens                   | `recorded` (a measured sum, `0` included), `incomplete` (a selected response lacks its output counter: `—`), `none_recorded` (no response today: `—`)                  |
| API-equivalent cost             | `priced` (a total), `partial` (a subtotal labelled "not a total", with priced-of-selected coverage), `unpriced`, `none_recorded`                                       |
| Agent hours                     | Always measured; `0` when there was no activity. The same number as the Dashboard's bar for today: a span that began before midnight counts from midnight.             |
| Local date, zone, observed time | The report's own date and zone, and the captured instant.                                                                                                              |
| Active now, last rule fire      | Stated as unavailable. Indexed events cannot prove what is running, and no rule-fire data exists. Nothing is invented: no waiting state, fire command, share or ratio. |

At exactly local midnight the half-open day is empty. The report says
`empty: true` with none-recorded figures; it does not return an invalid-window
error. F1's clock is pinned there, so the fixture export carries that day.

## Refresh

The popover reads when it is shown and reads nothing while hidden. While shown,
committed-data events refresh it through the shared invalidation helper, which
ignores scan-progress status. One timer reads again at the report's next local
midnight. There is no polling.

## Verification

- `cargo test -p xtrace-desktop --features fixtures`: the `today` tests cover
  the window, a DST date, the exact-midnight empty day, global response dedupe,
  partial/unknown/none-recorded states, one snapshot, the native fixture and a
  native re-read. The `tray` unit tests cover anchoring at 1× and 2× scale,
  mixed-scale displays, edge clamping and the reopen guard. While the item is
  off, `tray_off_closing_the_main_window_ends_the_app` runs the real setup and
  window handler on Tauri's mock runtime: only `main` is created, its close is
  not turned into a hide, and the run loop exits. The mock cannot deliver a Dock
  click, so that path is a native check.
- `pnpm check`: `TrayPage.test.tsx` covers reads only while shown, reopen,
  mount-after-show, the figure states, refresh on commits (not on scan progress
  or while hidden), a commit during the first read, load error and retry, the
  single midnight timer, Escape, Open, and light and dark.
- `pnpm e2e --grep tray`: browser layout in both themes. This is not native
  evidence.

## Native checks (manual)

While the item is off, run these in a debug bundle on macOS:

1. The close button closes the main window and the app ends: no
   `xtrace-desktop` process is left, and no popover or menu-bar item was shown.
2. Clicking the Dock icon after that starts the app, and it opens normally.
3. Minimize the main window, then click the Dock icon: the window is restored
   and focused. Launching the app a second time does the same.
4. **Quit** in the app menu exits the app, and the next launch opens normally.

When the item is on, run these instead:

1. The icon is a template image. It follows a light and a dark menu bar.
2. A left click opens the popover under the icon. Check a 2× built-in display,
   a 1× external display, and an icon near the right edge of the screen.
3. Clicking the icon again closes the popover and does not reopen it. Clicking
   elsewhere, or pressing Escape, also hides it.
4. Minimize the main window, then choose **Open** in the menu and the button in
   the popover. Close the window and choose **Open** again.
5. **Quit** exits the app, and the next launch opens normally.
