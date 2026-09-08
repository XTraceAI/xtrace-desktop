# TopBar, HostGlyph, and shared icons

Plan slot: FND-06b

These presentational primitives are an independent sibling to FND-06a. They
import the FND-05 theme foundation directly and do not require Sidebar. The
production application still renders its existing scaffold; consumers will
supply navigation context, range state, and actions in later shell work.

## TopBar

| Prop             | Contract / expected state                                                                                                                                                 |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crumb`          | Required primary breadcrumb text. It truncates visually and retains its complete title and accessible text.                                                               |
| `subcrumb`       | Optional secondary breadcrumb. The final visible crumb has `aria-current="page"`. Breadcrumbs are text, without router links.                                             |
| `showRange`      | Defaults to true. False removes the entire range group and requires neither `range` nor `onRange`.                                                                        |
| `range`          | Required when the range is visible: `7d`, `14d`, or `30d`. Selection comes only from this prop.                                                                           |
| `onRange(range)` | Required when the range is visible. Each click invokes the callback once; the caller supplies the next selected value.                                                    |
| `actionLabel`    | Optional visible action text. Missing, empty, or whitespace-only text produces no action button. Long text truncates while preserving its full accessible name and title. |
| `actionIcon`     | `share` (default), `scan`, or `copy`. The icon is decorative; the label names the action.                                                                                 |
| `onAction()`     | Optional callback. A visible action stays disabled until supplied.                                                                                                        |

TopBar fills its parent, remains 44px tall, and uses 20px horizontal padding.
The breadcrumb can shrink; the range and action controls retain their geometry.
Range controls are ordinary pressed-state buttons in a labelled group, with
24px segments. The 28px action has a bounded width. Buttons use explicit zero
tab indices so ordinary Tab reaches them under WebKit's macOS keyboard
preference; disabled buttons remain unfocusable. No keyboard listeners or
global event handlers are added.

Import `TopBar` and `TimeRange` from `apps/desktop/ui/src/kit/TopBar.tsx`. A caller
can connect `onRange` to local state or its own data flow. This PR covers only
the range dispatch part of U-02. Metric recomputation, rule popovers, deltas, and
the sample-count suppression rule remain consumer responsibilities.

## HostGlyph

| Prop      | Contract / expected state                                                                                                                                  |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `host`    | Canonical `claude`, `codex`, or `cursor`; any other string, `null`, or omission is unknown. Known keys are matched exactly, without guessing from aliases. |
| `size`    | 16 (default), 18, 20, or 30px. Smaller variants use a 4px radius; 30px uses an 8px radius.                                                                 |
| `stacked` | Defaults to false. True adds a 1.5px surface-colored border; adjacent stacked glyphs overlap by 4px.                                                       |

Known hosts use their `--host-*` color and explicit Claude/Codex/Cursor title and
accessible image label. Missing hosts display `?` and “Unknown host”. Other
strings retain the honest label “Unknown host: supplied value”, use neutral
track/meta colors, and never take a known host's identity. Glyphs introduce no
focusable element. When placing one inside an already labelled button, the
caller may mark its decorative wrapper `aria-hidden`.

The corrected font contract uses the actually bundled Geist Mono **600**,
superseding the legacy 700 request. The A, triangle, and question mark exist in
that font's glyph map. Its concentric-circle character is absent, so Codex uses
two inline SVG circles at the requested glyph size instead of relying on an
unbundled fallback font. These are compact host identifiers, not replacement
XTrace brand artwork.

## Icons and tokens

`Icon` from `src/kit/icons.tsx` accepts `name`, optional pixel `size` (default
14), and optional `className`. Names are `dashboard`, `sessions`, `prs`,
`rulebook`, `leaderboard`, `cloud`, `moon`, `gear`, `share`, `scan`, and `copy`.
Each icon has a 24-unit viewBox, a 2-unit current-color stroke, and hidden,
unfocusable SVG semantics. The containing control supplies its accessible name.
Sidebar consumers can map cloud/moon/gear to its hub/theme/settings slots
without requiring a Sidebar change in this PR.

The drawings are new simple line art in source. No unavailable design export,
external icon package, or pixel-for-pixel export provenance is claimed. All
component colors come from tokens. Two shared technical tokens are added to
both `design/token-contract.json` and `styles/tokens.css`: `--host-glyph-ink`
for the specified fixed white host foreground, and `--segment-shadow` for the
compact selected-range shadow. Existing token values and bundled assets remain
unchanged; the inherited contract test checks the new entries too.

## Verification

```sh
pnpm --dir apps/desktop/ui exec vitest run src/kit/TopBar.test.tsx src/kit/HostGlyph.test.tsx
pnpm lint
pnpm --dir apps/desktop/ui exec playwright test --config playwright.topbar.config.ts
```

The synthetic fixture uses port 5177. It renders design-width and minimum-app
content-width TopBars, known/unknown glyphs at every size, a stacked group, and
the icon set. It is outside the production entry point. Results and browser
floor limitations are recorded in [FND-06b acceptance](acceptance/FND-06b.md).
