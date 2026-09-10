# TopBar, HostGlyph, and shared icons

These reusable components follow the reviewed desktop design. The app shell
will supply navigation context, range state and actions when it adopts the kit.

## TopBar

| Prop                | Contract                                                                                                                                                                                   |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crumb`, `subcrumb` | Primary and optional secondary breadcrumb. The final crumb has `aria-current="page"`. Long text truncates visually while retaining its full title and accessible text.                     |
| `showRange`         | Defaults to true. False removes presets and the calendar button, and requires neither `range` nor `onRange`.                                                                               |
| `range`             | Required when visible: `7d`, `14d`, `30d` or `custom`. Selection comes only from this prop.                                                                                                |
| `onRange(range)`    | Required when visible. Requests a preset change; receives only `7d`, `14d` or `30d`. The caller supplies the next selected value.                                                          |
| `onCustomRange()`   | Opens the caller's date picker. Disabled until supplied. Opening does not change selection; after confirmation the caller supplies `range="custom"`. Clicking again can reopen the picker. |
| `actionLabel`       | Optional action text. Missing, empty or whitespace-only text omits the button. Long text truncates with its full title and accessible name preserved.                                      |
| `actionIcon`        | `share` (default), `scan` or `copy`; decorative beside the action label.                                                                                                                   |
| `onAction()`        | Callback; a visible action stays disabled until supplied.                                                                                                                                  |

TopBar fills its parent, stays 44px tall, and uses 20px horizontal padding.
Breadcrumb spacing is 8px. The preset segments are 24px high; the calendar is
26px wide and the primary action is 28px high. Breadcrumbs shrink to preserve
controls at the 1120px minimum app width (892px after the 228px sidebar).

Presets use the existing [Base UI Radio Group](https://base-ui.com/react/components/radio).
One Tab stop enters the selected preset; arrow keys select and wrap between
presets. Tab then reaches the calendar and action, skipping disabled controls.
The calendar is a separate button because it opens a picker and must remain
usable when a custom range is already selected. No date arithmetic, metric
recomputation, picker implementation or global keyboard handlers live here.

Import `TopBar`, `TimeRange` (presets) and `SelectedRange` (including custom) from
`apps/desktop/ui/src/kit/TopBar.tsx`.

## HostGlyph

| Prop      | Contract                                                                                 |
| --------- | ---------------------------------------------------------------------------------------- |
| `host`    | Exact `claude`, `codex` or `cursor`; other strings, null and omission render as unknown. |
| `size`    | 16 (default), 18, 20 or 30px. Radius is 4px, or 8px at 30px.                             |
| `stacked` | Adds a 1.5px surface border and overlaps adjacent stacked glyphs by 4px.                 |

Known hosts reuse the bundled logos from `public/hosts`, preserving aspect ratio
and a 1px inset. Cursor has a fixed white backing in both themes. The sidebar
now consumes this same component, retaining its layout and glyph override.
Upstream sources and licenses remain in `public/hosts/LICENSES.txt`.

Each glyph has a host title and accessible image name. Missing hosts display
`?` with “Unknown host”; unfamiliar strings use “Unknown host: supplied value”.
Glyphs are not focusable. A caller with an existing accessible label can place
one in an `aria-hidden` wrapper, as the sidebar does.

## Shared icons and tokens

`Icon` accepts `name`, pixel `size` (default 14), and optional `className`.
Names: `dashboard`, `sessions`, `prs`, `rulebook`, `leaderboard`, `cloud`, `moon`,
`gear`, `share`, `scan`, `copy`, `calendar`. Navigation artwork reuses the existing
sidebar SVGs; action and calendar paths come from the reviewed TopBar design.
SVGs are decorative and unfocusable; their containing controls supply names.

Colors use existing theme tokens. `--segment-shadow` adds the designed compact
selected-range shadow to both the CSS tokens and normative token contract.
No dependency or new logo asset is added.

## Verification

```sh
pnpm check
pnpm e2e
pnpm build
```

`e2e/topbar.html` provides synthetic design-width, minimum-width and host-size
fixtures, outside the production entry point. See [acceptance](acceptance/topbar.md)
for expected behavior, screenshots and native qualification limits.
