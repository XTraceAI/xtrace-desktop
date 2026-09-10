# Shared controls

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
| Toggle      | Base UI Switch rendered as a named native button, controlled checked/onChange, optional disabled and trailing display content. The trailing slot must be noninteractive; never nest a rule-popover button inside this button.                                                                                                                                                                            |
| Segmented   | Base UI RadioGroup/Radio with a label, string options/value/onChange and disabled states. Arrow keys move focus and request a value; selection changes only from props. Tab enters once and skips disabled choices. A missing or disabled selected value displays no selection; the first enabled choice remains reachable.                                                                              |
| ProgressBar | Required accessible label; either a nullable measured percentage and tone, or labelled semantic segments. Height 3/4/6. Negative values clamp to zero; oversized totals normalize proportionally. Nonfinite/null/empty segment input is unmeasured, with no numeric aria value. Segment proportions are announced in aria-valuetext, so different compositions with equal totals remain distinguishable. |
| Search      | Label, controlled string value/onValueChange and native input props/ref. A supplied shortcut is display-only: the consumer owns any global shortcut and should show the hint only when wired. The component itself registers no shortcut listener.                                                                                                                                                       |

Colors use the token contract. The only added token is `--toggle-knob`, a fixed
white knob. The selected segment reuses `--segment-shadow` from the top bar. The accent button uses the theme's
contrast-aware button foreground; dark-theme white text would have lower
contrast against its lighter accent. Production controls use no new font assets.

Ordinary Tab reaches Button and Toggle in the pinned macOS WebKit browser.
Native Space/Enter behavior is preserved rather than recreated by key handlers.
[Base UI Switch](https://base-ui.com/react/components/switch) and
[Radio](https://base-ui.com/react/components/radio) own interaction behavior.
There are no application key handlers or global listeners. Native Button/Search
and static badge/progress markup do not need another state-management layer.

Changing the Segmented option values or disabled flags rebuilds its Base UI item
registry. This avoids an unreachable Tab stop when a selected item becomes
unavailable. A focused item may lose focus when that list changes; callers that
change choices while the user is interacting should manage the surrounding
workflow's focus. Merely changing labels, tones, or the selected value does not
remount the group. Options must have unique values. Home/End is not a radio
navigation shortcut; use arrows and Space.

The search field and switch match the source geometry: 30px height, 8px radius,
10px horizontal padding; the switch has a 26×14px track and 10px white thumb.
The segmented group uses a 2px inset with 24px choices and 6px corner radii.
The reusable button variants share an 8px radius; individual screen designs can
have different button dimensions and are checked when those screens are wired.

## Preview and verification

```sh
pnpm dev --host 127.0.0.1 --port 5180 --strictPort
# Open http://127.0.0.1:5180/e2e/controls.html

pnpm check
pnpm e2e
pnpm build
pnpm check:native --base <reviewed-main-commit>
```

The development-only preview renders the actual components with synthetic data.
Try typing, toggling Capture, selecting a rule mode, and changing themes. The
collapsed keyboard examples exercise a held selection and choices becoming
unavailable. These controls are not yet wired into the installed app.

`pnpm e2e` runs the common Chromium and WebKit configuration. Screenshots go to
ignored test output; review them before copying selected synthetic images into
`docs/acceptance/controls/`. See [acceptance](acceptance/controls.md) for expected
results, the source comparison, and native validation limits.
