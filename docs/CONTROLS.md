# Shared controls

Plan slot: FND-07b

These controlled primitives build on the theme foundation. Import each component
from its named `apps/desktop/ui/src/kit` module; no router, data hooks or metric
engine is required. The application entry point does not yet render these
controls. Consumer wiring belongs to later shell and screen work.

| Component   | Props and behavior                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| KindBadge   | `kind` names skill, agent, mcp, hook, plugin, cmd, feature, bug or chore. Optional semantic `tone` overrides its default. The visible text retains meaning without color.                                                                                                                                                                                                                                |
| EvidenceDot | `evidence` is exact, sha or inferred; an accessible image label/title distinguishes all three.                                                                                                                                                                                                                                                                                                           |
| StatePill   | Content and semantic tone; height 20/24/26, optional outline. Only explicit `live` adds a pulse ring; reduced-motion preference removes animation. It does not invent a live announcement or connection state.                                                                                                                                                                                           |
| Button      | Native button props/ref, icon slot, primary/accent/outline/ghost and height 28/30/34/38. Defaults to type=button and an explicit zero tab stop. Disabled is native and inert.                                                                                                                                                                                                                            |
| Toggle      | Named native switch button, controlled checked/onChange, optional disabled and trailing display content. The trailing slot must be noninteractive; never nest a rule-popover button inside this button.                                                                                                                                                                                                  |
| Segmented   | Named radiogroup, string options/value/onChange and disabled states. Arrow/Home/End moves focus and requests a value; selection changes only from props. Roving tabindex skips disabled options, wraps and falls back to the first enabled option if the selected value disappears. Leaving the group resets its tab stop to the controlled selected option.                                             |
| ProgressBar | Required accessible label; either a nullable measured percentage and tone, or labelled semantic segments. Height 3/4/6. Negative values clamp to zero; oversized totals normalize proportionally. Nonfinite/null/empty segment input is unmeasured, with no numeric aria value. Segment proportions are announced in aria-valuetext, so different compositions with equal totals remain distinguishable. |
| Search      | Label, controlled string value/onValueChange and native input props/ref. A supplied shortcut is display-only: the consumer owns any global shortcut and should show the hint only when wired. The component itself registers no shortcut listener.                                                                                                                                                       |

All geometry and state colors use the token contract. `--toggle-knob` adds a
fixed white knob and `--segment-shadow` supplies the compact selected-control
shadow (the same value used by TopBar). The accent button uses the theme's
contrast-aware button foreground; dark-theme white text would have lower
contrast against its lighter accent. This is an explicit correction to the
legacy white-foreground request. Production controls use no new font assets.

Ordinary Tab reaches Button and Toggle in the pinned macOS WebKit browser.
Native Space/Enter behavior is preserved rather than recreated by key handlers.
Segmented owns only its radiogroup arrows and does not install global listeners.

## Verification

```sh
pnpm check
pnpm build
pnpm --dir apps/desktop/ui exec playwright test --config playwright.controls.config.ts
```

The browser fixture runs on port 5178 and contains synthetic component states.
`UPDATE_EVIDENCE=1` writes the two reviewed captures in
`docs/acceptance/FND-07b/`; it does not update runtime code or real user data.
See [acceptance](acceptance/FND-07b.md) for results and native floor limits.
