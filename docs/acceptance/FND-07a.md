# FND-07a acceptance evidence

Plan slot: FND-07a. Issue: #17. Scope: metric value presentation, section/stat layouts, current rule excerpts, and noninteractive definition tooltips. All illustrated values and controls are synthetic acceptance fixtures.

## Expected behavior and checks

| Requirement                        | Check and expected result                                                                                                                                                                                                                                                                                           |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Unknown is distinct from zero      | MetricCell/StatTile tests cover missing, blank, NaN, infinity, the absent Cursor Agent CLI usage reason, and measured zero. Unknown values do not call formatters or display supplied deltas.                                                                                                                       |
| Deterministic formatting           | Exact token, count, duration, fractional percent, and signed delta examples; zero normalization; invalid inputs remain unmeasured.                                                                                                                                                                                  |
| Current technical definitions      | 30 consumed IDs include M-18/M-19 and M-11a/M-12a. Real approved-source comparison passes. The report contains only IDs and hashes. A deliberately changed M-06 is rejected with only its ID, and the existing report remains unchanged.                                                                            |
| Passive definition tooltip         | In real WebKit, hover from a focused input or body never steals focus on leave. Tab focuses a chip and shows its exact associated definition. Escape dismisses without moving trigger focus; next Tab leaves normally. Blur dismisses.                                                                              |
| Click-through and readable content | Tooltip has no interactive descendants and `pointer-events: none`. A normal pointer click through its visible top-layer bounds reaches an underlying button. Full M-04 and C-08 definitions fit inside 1120×720 without scrolling. Native integer dimension rounding permits less than one pixel of edge tolerance. |
| Component geometry                 | Nine 300×52 stat states cover seven icons, override, both delta tones, unknown and zero; header heights are 36/40; long aside truncates; viewport has no horizontal overflow.                                                                                                                                       |
| Existing overlays remain correct   | All five inherited theme/font/popover/modal WebKit scenarios pass after the optional focus-policy extension.                                                                                                                                                                                                        |
| Static and unit checks             | `pnpm check`: TypeScript, ESLint, hex-token guard, Prettier, 25 Vitest cases and one Node test pass. `pnpm build` passes.                                                                                                                                                                                           |

Reproduce the focused checks:

```sh
pnpm --dir apps/desktop/ui exec vitest run src/kit/StatTile.test.tsx src/kit/MetricCell.test.tsx src/kit/RuleChip.test.tsx src/kit/rules.test.ts src/kit/format.test.ts src/kit/SectionCard.test.tsx
pnpm --dir apps/desktop/ui exec playwright test --config playwright.metrics.config.ts
pnpm --dir apps/desktop/ui exec playwright test e2e/overlays.spec.ts
pnpm --dir apps/desktop/ui exec node scripts/check-rules.mjs --spec APPROVED_SPEC.md
pnpm check
pnpm build
```

Two focused WebKit scenarios pass on pinned Playwright 1.58.2, one per emulated theme. The four screenshots were regenerated on that version. Their fixture uses port 5179, 1120×720, local bundled fonts, and native Popover behavior. The inherited overlay suite uses port 5175. No machine appearance settings are changed.

## Visual evidence

- [Dark metric states](FND-07a-metrics-dark.png)
- [Light metric states](FND-07a-metrics-light.png)
- [Dark full usage definition](FND-07a-definition-dark.png)
- [Light full usage definition](FND-07a-definition-light.png)
- [Rule excerpt coverage](FND-07a-rule-coverage.json)

These are browser-rendered synthetic fixtures. They demonstrate layout, fonts, native browser top-layer behavior, and emulated themes. They do not prove native desktop chrome behavior or a packaged run on the macOS 14/Safari 17 floor. Metric computation, real ingestion, coverage verification, and sample-count gating are outside this component slice. No production application, native controls, approved logo, or app icon is changed.

## Reviewability

The implementation has three related responsibilities: honest value formatting, rule definitions and their passive tooltip, and metric/section presentation. Small component modules keep their props and behavior independently reviewable. File count includes six focused unit files, an existing Popover regression, a separate browser fixture/config/spec, four synthetic screenshots, the definition contract/report, and documentation. There is no unrelated refactor or sibling control implementation. Self-review and independent source, test, and screenshot review: PASS.
