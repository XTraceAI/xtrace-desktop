# FND-08 acceptance

Issue #20. This change supplies grid tables, controlled host filtering and small activity/state presentations. It is based on reviewed FND-05, FND-06b, FND-07a and FND-07b producer code. It closes no metric rule, full gallery milestone or native release gate.

| Case                  | Setup/action                                                                                      | Expected result and evidence                                                                                                                                     |
| --------------------- | ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Typed table           | Render columns, all five heights, numeric zero and absent usage                                   | Shared column template and alignment; correct heights; unknown remains an em dash with reason. Unit and browser checks pass.                                     |
| Actions and expansion | Click row text, press Enter/Space, click a nested button or checkbox label, toggle details        | Exact row callback; nested controls do not open rows; controlled expansion stays in the same row group. Real label-text click and unit callback checks pass.     |
| Sorting and states    | Toggle sort; rerender empty and busy data                                                         | Correct controlled direction and aria-sort; rows are not sorted internally; explicit empty/status states replace data. Unit checks pass.                         |
| Containment           | An eight-column wide table and a 600px flexible-title table at a 1120px viewport                  | Wide table scrolls internally with aligned cells/header; fitting flexible title ellipsizes; page has no horizontal overflow. Both browser themes pass.           |
| Host filter           | Tab to checkbox, press Space, Escape, reopen and click outside                                    | Controlled selection updates; unknown host/count remains unknown; native dismissal returns focus correctly. Unit and actual WebKit checks pass.                  |
| Microcharts           | Zero, missing, negative/nonfinite, maximum and invalid-scale values; 14 days with boundary values | Visible zero differs from unknown; no invalid CSS/division by zero; exact labelled cells and threshold fills. Unit checks and browser zero/unknown styling pass. |

Commands run:

```sh
pnpm check
pnpm build
pnpm --dir apps/desktop/ui exec vitest run src/kit/DataTable.test.tsx src/kit/FilterMenu.test.tsx src/kit/SparkBars.test.tsx src/kit/DayStrip.test.tsx
pnpm --dir apps/desktop/ui exec playwright test --config playwright.tables.config.ts
```

All 53 Vitest cases (9 focused here), one Node lint regression, type/lint/format checks and the production build pass. Two real WebKit scenarios run on Playwright 1.58.2, with 1120px width and emulated dark/light appearance. They use synthetic content and local fonts; no native app or machine appearance setting is changed. Screenshots include the full fixture below the 900px viewport.

- [Dark table/states](FND-08/table-dark.png)
- [Light table/states](FND-08/table-light.png)
- [Dark filter](FND-08/filter-dark.png)
- [Light filter](FND-08/filter-light.png)

Focused real-browser regressions cover nested label activation, flexible-column intrinsic sizing, checkbox Tab reachability and matching chart/legend fills. No original design export parity or full screen data integration is claimed. Native macOS 14/Safari 17 component qualification remains separate from these browser checks.
