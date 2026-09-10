# Metric cards acceptance

The metric kit displays supplied values, distinguishes missing measurements from zero, and offers accessible rule explanations. The preview uses synthetic data. It does not change the installed application's screens yet.

## Review it locally

Run `pnpm dev` and open `/e2e/metrics.html` on the printed address. Switch themes, hover a card, and use Tab and Escape on the definition buttons. The preview uses the actual components, and its layout adapts to a narrow browser panel. Example controls exercise focus and click-through behavior; they do not perform product actions.

## Expected behavior

| Area                 | Test and expected result                                                                                                                                                                                                                          |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Missing measurements | Unit and browser cases distinguish null, undefined, blank, NaN, infinity, and absent usage from measured zero. Unknown values do not call formatters or show supplied deltas. The reason is accessible.                                           |
| Formatting           | Exact token, count, duration, fractional percent, and signed delta examples; deterministic locale and negative-zero handling. No metric calculation or sample eligibility is inferred.                                                            |
| Definitions          | All 30 selected entries match the reviewed source. Catalog/report tests validate IDs, hashes, and amended clauses. A private comparison with a deliberately changed entry rejects it without changing the report.                                 |
| Keyboard and hover   | Chromium and WebKit: hover preserves input/body focus; Tab opens the associated definition; Escape dismisses without moving focus; Tab leaves normally. No interactive descendants or focus trap.                                                 |
| Placement            | Pointer clicks through the popup reach an underlying control. Full usage and coverage definitions fit at 1120×720. A scoped theme propagates into the body portal, including after a theme change.                                                |
| Geometry             | Nine 300×52 states cover seven icons, tone override, both delta tones, long aside, missing value, and zero. Each label and number share a horizontal row. Section headers are 36/40 px; long text truncates without horizontal viewport overflow. |
| Regression           | The complete UI, browser, build, and local native suites cover the inherited sidebar, top bar, overlays, desktop launch, and data foundation.                                                                                                     |

Reproduce:

```sh
pnpm install --frozen-lockfile
pnpm --dir apps/desktop/ui exec playwright install chromium webkit
pnpm check
pnpm e2e
pnpm build
pnpm check:native --base <reviewed-base-commit>
```

Browser installation is a one-time prerequisite. A missing executable fails before a scenario can run. Native validation runs on a clean committed checkout on macOS; see [CI documentation](../CI.md). Hosted macOS validation remains reserved for release checkpoints.

## Visual evidence

- [Implemented cards, dark](metrics/metrics-dark.png) and [light](metrics/metrics-light.png).
- [Independent design reference, dark](metrics/design-dark.png) and [light](metrics/design-light.png).
- [Full usage definition, dark](metrics/definition-dark.png) and [light](metrics/definition-light.png).
- [Definition coverage](metrics/rule-coverage.json).

Reference images render the retrieved StatTile template with synthetic inputs and its original theme CSS, independently of the candidate components. Implemented cards use the reconciled contrast-aware tokens. The card uses the declared 52 px preview height (the raw template content sizes to 50 px). Horizontal layout and icon artwork follow the reference; text clipping, semantic buttons, and accessible explanations are deliberate additions. These images do not claim pixel-identical colors or native window validation.

## Validation result

The UI validation passes 40 Vitest cases, one lint-policy test, 28 Chromium/WebKit scenarios, TypeScript, ESLint, formatting, and the production build. The PR records the committed candidate and its local native result. Browser captures use bundled fonts and Playwright WebKit at 1120×720; reference captures use a 988×244 grid. The tests do not prove metric computation, real ingestion, receipt verification, or the macOS 14 release floor.
