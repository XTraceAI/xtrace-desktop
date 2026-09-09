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

| Prop                 | Contract / default                                                                                                                                                                      |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `activeKey`          | Required: `dashboard`, `sessions`, `prs`, `rulebook`, `leaderboard`, or `team`. Sets `aria-current="page"`; clicks do not change it internally.                                         |
| `onNavigate(key)`    | Required action for enabled navigation. The caller owns routing and the next active key.                                                                                                |
| `rulebookCount`      | Defaults to 0. Only finite positive values show a badge.                                                                                                                                |
| `leaderboardEnabled` | Defaults to false; the row is disabled and shows “soon”.                                                                                                                                |
| `showTeam`           | Defaults to false. When true, the Team navigation row appears.                                                                                                                          |
| `hubConnected`       | Defaults to false. Controls the Hub summary and enables a visible Team row independently of Leaderboard.                                                                                |
| `teamLabel`          | Optional Team navigation/connection label. Defaults to “Team” in navigation and “Hub connected” in the connected footer.                                                                |
| `onConnectHub()`     | Optional callback passed to the Hub CTA. Without it, the disconnected CTA is disabled.                                                                                                  |
| `hosts`              | Required array of `{ host, tokens, fillPercent, glyph? }`. One row per host; supported token hosts are `claude`, `codex`, and `cursor`. Order comes from the caller.                    |
| `tokensCaption`      | Optional label beside “Tokens by host”; no reporting period is inferred.                                                                                                                |
| `surfaces`           | Defaults to `[]`. Rows are `{ host: string, surface: string \| null, status, reason? }`, keyed by the unique host/surface pair. `status` is `capturing`, `not-capturing`, or `unknown`. |
| `listener`           | Required discriminated state: `{ status: 'listening', port: number }`, `{ status: 'off' }`, or `{ status: 'unknown' }`.                                                                 |
| `version`            | Required version without the `v` prefix.                                                                                                                                                |
| `updateLabel`        | Optional exact updater copy. Omission displays only the version.                                                                                                                        |
| `theme`              | Required resolved `dark` or `light`; supplies the toggle's accessible next-action label.                                                                                                |
| `onToggleTheme()`    | Required action; the caller changes appearance through the theme provider.                                                                                                              |
| `onSettings()`       | Optional action. Settings stays disabled until supplied.                                                                                                                                |
| `topInset`           | Total top padding in CSS pixels, default 16; finite values clamp to at least 16. The native shell can pass 74 to reserve its existing window controls.                                  |
| `icons`              | Optional decorative React nodes keyed by navigation key, `hub`, `theme`, or `settings`; each slot is 14px. Accessible names come from button text/labels.                               |

Host token values are display inputs, not computed totals. `null`, non-finite,
or negative values display “—” with an accessible “unmeasured” label and no bar
fill. Measured zero displays “0” and no fill. Positive values use compact English
formatting; the accessible label retains the full number. A supplied
`fillPercent` clamps to 0–100 and defaults to zero when invalid. An empty host
array displays “No host measurements”.

Surface identifiers remain raw discovered strings. For example, a capturing
`Codex / cli` row and a not-capturing `Codex / desktop` row coexist; neither
overwrites the other. An unfamiliar surface name is shown as supplied; `null`
displays “Unknown surface”. An empty array displays “Capture status unknown”.
Optional reasons remain visible under their own row. Listener status reports
the listener only and does not certify capture on any surface.

Token-host glyph slots are 16px and decorative; host names remain explicit.
Shared icons and host glyphs are supplied by callers; these components use small
text symbols and colored dots as defaults. Callers can supply those later primitives
without changing status or navigation behavior. The sidebar reserves native
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
14px right of the sidebar with its bottom 6px below the trigger. The Sidebar supplies an invisible status-row positioning target that reaches its right edge; the button remains the native invoker and focus-return target. Standalone HubPopover consumers can provide their own `positionRef` or use the trigger for geometry. It composes the
existing native Popover; Escape, outside dismissal, and focus restoration stay
with that primitive. Its visible “esc” button also closes it. Content can scroll
in a constrained viewport. The shared popover supplies viewport clamping.

The CTA only invokes the supplied callback; it does
not connect, navigate, or mark the desktop connected on its own. The CTA uses
`--btn-ink-text` over `--accent`: a light foreground in light mode and a dark
foreground on the lighter dark-mode accent, preserving stronger contrast than
forcing the legacy white foreground in both themes.

## BrandMark props

| Prop        | Contract / default                                                                                 |
| ----------- | -------------------------------------------------------------------------------------------------- |
| `size`      | 20, 26, or 34px; default 26.                                                                       |
| `showLabel` | Defaults to true, showing the XTrace wordmark. False gives the image the accessible name “XTrace”. |

The image is the approved `/mark.png` asset with its geometry intact. No clipping,
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
