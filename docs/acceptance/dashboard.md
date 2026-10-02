# Dashboard screen acceptance

`/dashboard` renders the generated `DashboardMetrics` report through `DataSource`. The UI formats values only; every metric, sample count, delta and coverage figure comes from the Rust report. No UI library, chart framework, dependency, storage or DTO was added.

## Layout

The primary composition, top to bottom:

- The TopBar shows the report period immediately before the 7d/14d/30d presets. It updates with the range, is hidden while a range loads and off the Dashboard, and gives the time zone in its tooltip and accessible text.
- Heading "What your agents did", then sessions, human messages, tool calls and favorite model (M-10; an unknown favorite shows `—` with its reason).
- **Agent / human hours**: a compact card with agent hours and estimated human-in-the-loop hours inline ("human est."), and the agent-to-human ratio on a small second line. Each value opens its definition (M-05, M-07 with the estimation method, M-08). Comparisons appear only when the report does not suppress them. Beside it, **Caught by your rules** (R-05) shows `—` and the report's unavailable reason.
- Four StatTiles with full labels: Agent h/day (the note states the day divisor), Concurrency (mean, with max), Merged PRs, and Hands-off median (with p90). Each definition quotes its rule and adds the report note, reason, sample counts and suppressed-delta explanation. The hands-off definition begins "How long your agents run before they need you." and lists the surfaces in `hands_off_excluded_surfaces` with qualifying and degenerate session counts.
- **Effort by type** (M-19) and **Environment** (M-17), at 1.6:1, keep their titles and reserved area and show the report's unavailable reason, never zero or sample data.
- **Sessions**: one row per returned active span on the report's fixed 48-hour axis, whatever range is selected. A session with several spans has several rows, keyed by host, session, start and end. Rows show the host glyph and real session ID; screen readers get the full ID and span times. Counts and truncation (`lanes_total`) are labelled as spans. "View sessions" opens the existing Sessions route.

Supplementary measurements follow in collapsed sections; each keeps its title, rule and summary visible, and its expanded state survives a range change:

- **Tokens per day** (M-04), captioned with the recorded total and `usage_coverage.total` measured/sessions: one row each for fresh input, output and cache read, each on its own scale labelled "measured peak" (a series with no numeric counter says "no recorded measurement"; a measured zero peak stays 0), with a table of every day's counters including cache writes and total. The row count matches the range (7, 14 or 30 days). A day without selected usage is "no recorded usage", a null counter is a dashed "unmeasured" bar, and a measured zero stays 0.
- **API-equivalent cost**: total, catalog version, `as_of`, basis, priced and unpriced response counts, and each unpriced model/tier with its reason. When `total_usd` is null, the total shows `—`; a priced subtotal is described as partial and never shown in the total slot.
- **Coverage**, summarized with the inventory state: token measurement (total and by surface, gaps named, fixed trailing-14-day gate); session capture with the inventory state (only `fresh_complete` uses the success tone; `unknown` is neutral; an empty list reads "No session capture coverage rows available.", which does not prove there are no receipts) and a note that historical receipts do not mean a surface is capturing now; hands-off timestamp-health exclusions.
- A one-line native index status (phase and freshness), stating that indexing does not certify capture coverage, with a link to the full diagnostics in Settings.

Percentages in the report are percentage points (50 means 50%). `present.ts` divides by 100 exactly once before the kit fraction formatters. Suppressed deltas are not shown.

When a tile's values (for example mean, change and max) do not fit beside its full label, the value row wraps below the label rather than truncating it. At a Dashboard width under 1040px (1120×720 windows), tiles become a 2×2 grid; paired cards stack only under 720px. At 1440×900 the primary composition through the Sessions lanes fits in the first view. The page scrolls vertically without horizontal overflow.

## Verification

| Check                 | Result                                                                          |
| --------------------- | ------------------------------------------------------------------------------- |
| `pnpm check`          | Types, ESLint, hex lint, Prettier, 7 Node and 113 Vitest cases (34 files) pass. |
| `pnpm build`          | Passes.                                                                         |
| `pnpm e2e`            | 64 cases pass in WebKit and Chromium, including `dashboard.spec.ts`.            |
| `pnpm e2e:production` | 4 cases pass.                                                                   |

`src/app/dashboard/DashboardPage.test.tsx` covers generated F1 plus synthetic reports: F2 concurrency 1.8/max 3 with overlapping lane geometry; F7 unmeasured tokens, cost, favorite model and 0% coverage; F11 deltas (50 → ▲50%, 100 → ▲100%, −25 → ▼25%, suppressed hidden); partial pricing; complete total; measured zeros versus unknown ratio; hands-off exclusions; truncated lanes; several spans of one session as separate rows; a caption using usage-coverage sessions (1 measured of 10); measured, measured-zero and unmeasured series peaks; an empty capture list with unknown inventory; section order (hero and rules, tiles, effort/environment, sessions, then collapsed tokens, cost and coverage with their content rendered); the TopBar period hidden while a range loads and off the Dashboard; the hands-off definition's leading sentence; a 30-day chart; shared range changes for Dashboard and sidebar; loading, safe error and retry; refresh after an index-status event that enriches tokens without adding a session, and after an import that adds a session; and no metric reads in a browser preview.

`e2e/dashboard.spec.ts` loads fonts and checks light and dark themes at 1440×900 and 1120×720. It asserts document width equals the viewport, no outlet overflow, a 228px sidebar, 44px TopBar, main width, h1 at x=245/y=57 with 29px height, no clipped text box in the page, every StatTile label at full width, four tiles in one row at 1440 and a 2×2 grid at 1120, and that the lanes are reachable by scrolling. It also checks the report period sits immediately left of the presets on the same row, hero and rules cards share a row and height, effort and environment follow the tiles at 1.6:1 with Sessions next and the collapsed sections after it, the primary composition through Sessions fits 1440×900, and every section expanded still has no clipping or horizontal overflow. It also exercises the 7/14/30 presets against the TopBar period, sidebar caption, host value and chart day count (with the tokens section kept open across range changes), the disabled custom range, and keyboard access to a tile definition and the Sessions link. `e2e/dashboard-dense.spec.ts` serves a synthetic dense report (Concurrency mean 1.8 ▼12.3% max 8, plus multi-digit values and changes on the other tiles) and checks, in both themes at 1440×900 and 1120×720, that every tile label, value and aside has scrollWidth ≤ clientWidth, stays inside its tile, and that the page has no horizontal overflow. Screenshots go to the Playwright output directory only and use the synthetic F1 export.

Not covered here: the native app launch, transparency, drag regions and traffic-light inset (native gate), and the native invalidation path, which Rust bridge tests cover.
