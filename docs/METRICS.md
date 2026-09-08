# Metric presentation components

Plan slot: FND-07a. These components render caller-provided values and current technical definitions. They do not compute metrics, decide capture verification, select windows, convert durations, or decide whether a delta has enough samples to display.

| Component     | Inputs and expected states                                                                                                                                                                                                                                                                                        |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MetricCell`  | `value` is a number, string, null, or undefined. Finite numbers use optional `format` (default `String`); nonempty strings remain verbatim. Missing, blank, NaN, and infinite values render `Unmeasured`. Sizes: 10.5, 11 (default), 18, 24, 30 px. Alignment: left (default) or right within the allocated cell. |
| `Unmeasured`  | Visible em dash plus screen-reader text `Unmeasured: <reason>` and a native title. Default reason: `This value was not measured`. Callers should provide the source-specific reason when known. A measured zero always remains zero.                                                                              |
| `StatTile`    | Required `label`, `ruleId`, `icon`; optional `value`, `format`, `reason`, `unit`, `delta`, `deltaTone`, `aside`, `tip`, `iconTone`. A 52 px surface card fills its container. The whole tile is a focusable definition button. Label and aside truncate independently; the aside retains its native title.        |
| `SectionCard` | Required `title` and `children`; optional `ruleId`, `meta`, `right`, `footer`. Header height 40 px by default or 36 px. Inner padding defaults to `12px 14px 10px` and accepts a CSS padding value. The section has a unique accessible heading association. The optional right slot remains caller-controlled.   |
| `RuleChip`    | Required typed `ruleId`; `size` is `normal` (default) or `sm`. A native button named `Definition <id>` exposes the definition through `aria-describedby`. Do not nest it inside another button. StatTile uses a decorative badge inside its own trigger.                                                          |
| `RulePopover` | Required `ruleId` and render-function `children`; optional `context` and wrapper `className`. The render function receives `aria-describedby` and `onClick`; apply both to one focusable native trigger. Context is an additional caller-supplied explanation, separate from the registry definition.             |

Example of an unknown value with a precise explanation:

```tsx
<MetricCell
  value={null}
  reason="Cursor Agent CLI usage is absent; token totals were not measured."
/>
```

`StatTile` shows a finite supplied delta only when its main value is measured. Positive changes use ▲, negative changes use ▼, and zero uses `0%`. `deltaTone="good"` uses success color; `bad` uses warning color. Direction does not choose the tone. The caller owns sample-count eligibility and metric-specific interpretation.

The seven original inline SVG symbols are `lanes` (info), `merge` (accent), `clock` (success), `bolt` (danger), `msg` (warning), `token` (info), and `shield` (success). `iconTone` can override the default with info, accent, success, danger, warning, or meta. They are decorative; the adjacent label supplies the meaning. This small metric icon module has no dependency on a sibling component kit.

## Formatting contract

Formatters use the explicit `en-US` locale. They accept numeric inputs only; null, undefined, NaN, and infinities return an em dash. Negative zero is normalized. Finite negative inputs retain their sign. Validating a metric's domain belongs to its producer.

| Helper             | Input unit                         | Presentation                                                                                                                                                                |
| ------------------ | ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tokens`           | Tokens                             | Below 1,000: plain; from 1,000: K; from 100,000: M; from 1,000,000,000: B. At most two decimals. Examples: 15,100,000 → `15.1M`, 3,720,000,000 → `3.72B`, 400,000 → `0.4M`. |
| `count`            | Count                              | Grouped integer; 4,639 → `4,639`.                                                                                                                                           |
| `hours`, `minutes` | Already-converted hours or minutes | At most one decimal; 62.3 → `62.3`. No duration conversion or unit suffix.                                                                                                  |
| `percent`          | Fraction                           | At most one decimal; 0.18 → `18%`.                                                                                                                                          |
| `delta`            | Signed fractional change           | 0.18 → `▲18%`, -0.09 → `▼9%`, 0 → `0%`.                                                                                                                                     |

The examples are formatter inputs, not product metric results.

## Definition source and tooltip behavior

[rule-contract.json](../design/rule-contract.json) contains 30 selected normative definitions from the current XTrace Desktop SPEC. The typed runtime registry imports that file directly. It includes M-01–M-19, amendments M-11a/M-12a, and the consumed receipt, privacy, coverage, and provenance definitions C-08, O-11/O-12, P-01/P-02, R-05/R-08, U-03/U-08.

Markdown emphasis and code markers are removed. Supporting observations, historical samples, and anecdotes are omitted from M-01/M-02/M-03 and O-11/O-12; their selected normative wording is preserved. This is an excerpt contract, not a copy of the whole SPEC. To verify an approved source after editing a definition:

```sh
pnpm --dir apps/desktop/ui exec node scripts/check-rules.mjs --spec APPROVED_SPEC.md
```

The comparison checks every selected string, fails on changed or missing definitions and changed selection boundaries, and writes only rule IDs and SHA-256 hashes to [the coverage report](acceptance/FND-07a-rule-coverage.json). Failure leaves the previous report intact and reports IDs or a controlled generic error. The command needs the approved source; ordinary repository tests validate contract/report parity without that external document. A matching hash report is evidence of that comparison, not independent approval of the definition.

Hover or keyboard focus opens a native Popover with `role="tooltip"`. It stays open while its trigger remains hovered or focused, closes when both leave, and supports native Escape dismissal. The tooltip has no interactive descendants and `pointer-events: none`, so it never traps keyboard focus or intercepts clicks. Full definitions remain available through `aria-describedby` and the trigger's native title. At the supported 1120×720 minimum, the tested longest usage and coverage definitions fit without scrolling.

The shared Popover's new `restoreFocus` prop defaults to `true` for existing menus. RulePopover passes `false`: closing a hover explanation must not move focus to its trigger. Native menus and modal behavior retain their existing tests.
