# Sidebar components

`Sidebar`, `HubPopover`, and `BrandMark` are independently renderable component
foundations. The production app still uses the existing scaffold; a later shell
change will supply real data and navigation. These components read props, render
them, and call action handlers. They contain no router, data hooks, native API
calls, connection workflow, or metric calculations.

Import components directly from `apps/desktop/ui/src/kit/`. Mount them beneath
the existing theme provider with the shared `index.css`. A sidebar fills its
parent's height, occupies 228px, and uses a transparent background. Its parent
supplies the canvas color. Long content scrolls vertically.

## Sidebar props

| Prop                 | Contract / default                                                                                                                                                                                      |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `activeKey`          | Required: `dashboard`, `sessions`, `prs`, `rulebook`, `leaderboard`, or `team`. Sets `aria-current="page"`; clicks do not change it internally.                                                         |
| `onNavigate(key)`    | Required action for enabled navigation. The caller owns routing and the next active key.                                                                                                                |
| `rulebookCount`      | Defaults to 0. Only finite positive values show a badge.                                                                                                                                                |
| `leaderboardEnabled` | Defaults to false; the row is disabled and shows “soon”.                                                                                                                                                |
| `showTeam`           | Defaults to false. When true, the Team navigation row appears.                                                                                                                                          |
| `hubConnected`       | Defaults to false. Controls the Hub panel and enables a visible Team row independently of Leaderboard.                                                                                                  |
| `teamLabel`          | Optional Team navigation/connection label. Defaults to “Team” in navigation and “XTrace Hub” in the connected panel.                                                                                    |
| `onConnectHub()`     | Optional callback passed to the Hub CTA. Without it, the disconnected CTA is disabled.                                                                                                                  |
| `hosts`              | Required array of `{ host, tokens, fillPercent, glyph? }`. One row per host; supported token hosts are `claude`, `codex`, and `cursor`. Order comes from the caller.                                    |
| `tokensCaption`      | Optional label beside “Usages”; no reporting period is inferred.                                                                                                                                        |
| `surfaces`           | Defaults to `[]`. Rows are `{ host: string, surface: string \| null, status, reason? }`, keyed by the unique host/surface pair. `status` is `capturing`, `not-capturing`, or `unknown`.                 |
| `listener`           | Required discriminated state: `{ status: 'listening', port: number }`, `{ status: 'off' }`, or `{ status: 'unknown' }`.                                                                                 |
| `version`            | Required version without the `v` prefix.                                                                                                                                                                |
| `updateLabel`        | Optional exact updater copy. Omission displays only the version.                                                                                                                                        |
| `theme`              | Required resolved `dark` or `light`; supplies the toggle's accessible next-action label.                                                                                                                |
| `onToggleTheme()`    | Required action; the caller changes appearance through the theme provider.                                                                                                                              |
| `onSettings()`       | Optional action. Settings stays disabled until supplied.                                                                                                                                                |
| `topInset`           | Total top padding in CSS pixels, default 16; finite values clamp to at least 16. The native shell can pass 74 to reserve its existing window controls.                                                  |
| `icons`              | Optional decorative React nodes keyed by navigation key, `hub`, `theme`, or `settings`; 14px navigation slots, a 15px cloud icon, and a 13px theme icon. Accessible names come from button text/labels. |

Host token values are display inputs, not computed totals. `null`, non-finite,
or negative values display “—” with an accessible “unmeasured” label and no bar
fill. Measured zero displays “0” and no fill. Values from 100,000 use millions with at most one decimal (400,000 → 0.4M); smaller positive values use compact English formatting; the accessible label retains the full number. A supplied
`fillPercent` clamps to 0–100 and defaults to zero when invalid. An empty host
array displays “No host measurements”.

Click the plugin status row to open “Capture by surface”. Surface identifiers remain raw discovered strings. For example, a capturing
`Codex / cli` row and a not-capturing `Codex / desktop` row coexist; neither
overwrites the other. An unfamiliar surface name is shown as supplied; `null`
displays “Unknown surface”. An empty array displays “Capture status unknown”.
Optional reasons remain visible under their own row. Listener status reports
the listener only and does not certify capture on any surface.

Token-host glyph slots are 16px and decorative; host names remain explicit.
The default navigation SVGs and bundled host logos match the design reference. Cursor’s dark logo has a fixed light backing in both themes to preserve its contrast. Callers can override individual slots without changing status or navigation behavior. Host logo licenses and pinned upstream sources ship in `public/hosts/LICENSES.txt`. The sidebar reserves native
chrome space; it draws no replacement traffic lights or drag regions.

## HubPopover props and states

| Prop                                      | Contract / default                                                                                                      |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `id`, `open`, `onOpenChange`, `anchorRef` | Required shared Popover contract. Use a trigger with the same `popoverTarget`, its ref, and controlled `aria-expanded`. |
| `positionRef`                             | Optional positioning-only target forwarded to Popover; `anchorRef` remains the native invoker and focus-return target.  |
| `connected`                               | Defaults to false. Shows the connection invitation and CTA. True shows the connected title and removes the CTA.         |
| `teamLabel`                               | Optional connected title label, falling back to “XTrace Hub”.                                                           |
| `onConnect()`                             | Optional callback; an absent callback disables the disconnected CTA.                                                    |

The Hub popover is 264px wide, uses canvas tokens and a left caret, and anchors
14px right of the footer content edge with its bottom 6px below the status row. The Sidebar supplies that row as its positioning target; the full-width “Cloud and Team” button remains the invoker and focus-return target. Standalone HubPopover consumers can provide their own `positionRef` or use the trigger for geometry. It composes the
existing native Popover; Escape, outside dismissal, and focus restoration stay
with that primitive. Its visible “esc” button also closes it. Content can scroll
in a constrained viewport. The shared popover supplies viewport clamping.

Sidebar opening is controlled so switching between Hub and capture details cannot race deferred native toggle events. Shared geometry updates are separate from native opening, so outside clicks that change navigation or appearance keep a dismissed panel closed. Native Escape, outside dismissal and focus return remain in the shared Popover.

The CTA only invokes the supplied callback; it does
not connect, navigate, or mark the desktop connected on its own. The CTA uses
`--btn-ink-text` over `--accent`: a light foreground in light mode and a dark
foreground on the lighter dark-mode accent, preserving stronger contrast than
forcing the legacy white foreground in both themes.

## BrandMark props

| Prop        | Contract / default                                                                                 |
| ----------- | -------------------------------------------------------------------------------------------------- |
| `size`      | 20, 22, 26, or 34px; default 26. The sidebar selects 22px.                                         |
| `showLabel` | Defaults to true, showing the XTrace wordmark. False gives the image the accessible name “XTrace”. |

The image is the approved `/sidebar-mark.png` asset with its geometry intact. No clipping,
recoloring, or replacement brand art is applied.

## Verification

Run the focused tests and synthetic Chromium/WebKit preview from the repository root:

```sh
pnpm --dir apps/desktop/ui exec vitest run src/kit/Sidebar.test.tsx src/kit/HubPopover.test.tsx src/kit/BrandMark.test.tsx
pnpm lint
pnpm e2e --grep sidebar
```

The browser fixture uses the shared runner and emulated appearance. It exercises the
real native popover API; unit tests isolate rendering and callbacks without
claiming browser behavior. The fixture is outside the production entry point.
See [Sidebar acceptance](acceptance/sidebar.md) for results, screenshots, and limits.
