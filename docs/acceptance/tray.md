# Tray popover acceptance

## Status: enabled

The menu-bar item is enabled (`tray::ENABLED` is `true`). This document
records its implementation and automated checks. Display placement, focus,
and menu-bar interaction still require the manual native checks below.

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

## Click and dismissal behavior

The left press records whether the popover should close. Release uses that
same decision even when focus loss already hid it, or a press lasts longer
than the existing 400 ms blur guard. A completed explicit close clears that
guard so the next click can open the popover again.

On macOS a shown popover remembers only the process ID of the external app
that was frontmost before it showed and requested focus. An icon close or
Escape consumes that saved ID once and requests a cooperative return to that
app, only if XTrace is still frontmost at the native main-thread callback.
Outside blur hides without returning focus. A new show, outside blur or an
explicit Open/Dock/second-launch request invalidates older queued dismissals.
Open/Dock/second launch discard the saved ID before showing the main window.
No all-windows activation flag or Reopen suppression is used.

These decisions have automated checks. Real menu-bar click/focus behavior
has not been verified for this change.

## What it shows

`today_summary` captures one clock and zone. It reads M-04 tokens and cost and
M-05 active spans over `[local midnight, now)`, and the existing human-hours
metric for that whole local day, in one `MetricsDb` snapshot. Human time uses
the saved break length read under the app store lock, as on the Dashboard.
It defines no metric of its own.

| Figure                          | States                                                                                                                                                                                                                             |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| API-equivalent cost             | `priced` (a total, including `$0.00`); `partial`, `unpriced`, `none_recorded` and a missing total show `—`. The estimate basis, pricing coverage and partial subtotal explanation are available on hover and to assistive readers. |
| Agent hours                     | Always measured; `0` when there was no activity. The same number as the Dashboard's bar for today: a span that began before midnight counts from midnight.                                                                         |
| Local date, zone, observed time | The report's own date and zone, and the captured instant.                                                                                                                                                                          |
| Your hours                      | The shared whole-local-day human time, shown as an estimate. The saved break length and explanation are available on hover and to assistive readers. Unknown classifications show `—`; zero stays zero.                            |
| Account usage                   | The same widget and passive query as the sidebar. Manual Refresh is disabled in the tray; open the main window to refresh Claude usage.                                                                                            |
| Recent sessions                 | Up to three shown rows from the first indexed page sorted by recently active. Shared session display checks, transient/saved titles, and bounded live status reads are reused. The list is not every running session.              |

Agent hours, human hours and cost appear together in three cards at the top.
The cost card shows only today’s price; output tokens, response counts and a
separate cost details line are not shown. The shared money formatter keeps
cents below $100, whole dollars from $100, and `<$0.01` for tiny positive costs.

At exactly local midnight the half-open day is empty. The report says
`empty: true` with none-recorded figures and zero human time; it does not return an invalid-window
error. F1's clock is pinned there, so the fixture export carries that day.

## Refresh

The popover reads when it is shown and reads nothing while hidden. While shown,
committed-data events refresh it through the shared invalidation helper, which
ignores scan-progress status. One timer reads again at the report's next local
midnight. The usage widget uses its existing passive refresh interval and reset
timer. Live states use the existing two-second bounded session lease. Today,
usage, recent-session queries, transient title reads, timers and live leases
are mounted only while the popover is shown. The footer stays fixed while its
content scrolls, including expanded usage details.

## Verification

- `cargo test -p xtrace-desktop --features fixtures --lib tray::tests`: the
  existing anchor and close-window checks, plus slow and quick clicks,
  blur-before-release during a long press, blur-before-press, guarded release,
  Escape, outside blur, queued-dismissal cancellation, consumed return IDs,
  and the shared Open/Dock entry point on Tauri's mock runtime. Mock window
  operations do not verify native focus or menu-bar clicks.
- `cargo test -p xtrace-desktop --features fixtures --test today --test human_break`:
  the Today checks cover local day bounds, DST, exact midnight, response
  states and dedupe, the single snapshot, and the shared human-hours calculation
  under three break lengths and unknown sender classification outside today. The saved-setting test checks the tray reads
  the committed break length.
- `TrayPage.test.tsx`: visibility races, reopen, committed-data refresh,
  midnight invalidation, error/retry, usage and human states, up to three recent
  sessions, shared title/live status, hidden lease/timer cleanup, and Open/Escape.
- The tray browser layout tests cover the fixed 360×540 panel, themes, human
  hours, usage, recent-session placement, scrolling and the fixed Open button.
  They also check the three cards share one row with priced, partial, unknown,
  zero, tiny and large costs and long hour values in both themes.
  Browser checks are not native menu-bar evidence.

## Native checks (manual)

1. The icon is a template image. It follows a light and a dark menu bar.
2. A left click opens the popover under the icon. Check a 2× built-in display,
   a 1× external display, and an icon near the right edge of the screen.
3. Clicking the icon again closes the popover and does not reopen it. Clicking
   elsewhere, or pressing Escape, also hides it.
4. Minimize the main window, then choose **Open** in the menu and the button in
   the popover. Close the window and choose **Open** again.
5. **Quit** exits the app, and the next launch opens normally.
