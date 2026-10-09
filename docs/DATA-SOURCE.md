# App data and routing

The shell reads app metadata, database counts and Dashboard metric reports through `DataSource`. The UI does not calculate metrics; the Rust application assembles them from `xt-metrics`. DTOs are generated from Rust into `apps/desktop/ui/src/data/generated/`; regenerate and verify them with `cargo xtask dto-export` and `bash scripts/ci/check-dto.sh`. Generated files retain the generator's formatting.

## Selecting a source

Native Tauri always selects `TauriDataSource`, including when `VITE_XTRACE_FIXTURE` is set. Its `appInfo()`, `dbCounts()` and `nativeIndexStatus()` methods invoke the `app_info`, `db_counts` and `native_index_status` commands; the last returns the generated `NativeIndexStatus` (phase, freshness, interpreter, bundled readers and each host's last scan), which the app also publishes on every change as the `native-index://status` event. Command and event names live in `src/data/ipc-names.ts`; components import the generated types and use query hooks rather than calling IPC.

For a browser fixture preview:

```sh
pnpm fixtures:export
VITE_XTRACE_FIXTURE=F1 pnpm dev
```

`FixtureDataSource` reads the generated `apps/desktop/ui/fixtures/F1.json` shell export. It returns copies of the same `AppInfo`, `DbCounts` and Dashboard report shapes used by native IPC. The export contains a portable `fixture://F1` label and canonical counts of 1 session, 25 records, and 15 usage rows. Its `emit(event)` helper delivers deterministic test events without timers or a simulated ingestion service. F2, F20, and unrecognized fixture names fail explicitly; they have no populated shell export.

The fixture adapter is dynamically imported only inside a Vite `DEV` condition. A production build ignores `VITE_XTRACE_FIXTURE`, even when it is set during compilation, and excludes the adapter chunk and fixture JSON. A browser without an enabled development fixture shows an explicit unavailable preview. It never falls back to sample data. Native fixture selection and temporary database ownership are enforced separately by the Rust application.

## Queries and events

`DataProvider` owns one active QueryClient for the window's selected source. Replacing the source remounts the query runtime, creating an isolated client and fresh query observers and warning state. Queries use a one-second stale time, no automatic retry, and no focus refetch. The shell's metadata consumers share `['app', 'info']`, `['database', 'counts']` and `['native', 'index']`, so reading the same data in the sidebar and Settings shares a cache entry. Refresh explicitly refetches the first two; the index status refreshes on its own event and, while it is transient (the initial scan or interpreter discovery), polls every second, since the one-time `ready` event can precede the listener's registration; when the status is first seen settled, or seen to settle after polling, the same query prefixes that event would have invalidated are invalidated.

The provider registers each event once per mounted lifecycle. A single 500ms window starts with the first event; matching query prefixes are collected in a set. Continuous imports therefore refresh at bounded intervals rather than postponing refresh indefinitely.

| Event                        | Query prefixes invalidated                           |
| ---------------------------- | ---------------------------------------------------- |
| `ingest://import-received`   | `database`, `metrics`, `sessions`, `hosts`           |
| `ingest://backfill-progress` | `database`, `metrics`, `sessions`, `hosts`           |
| `ingest://turn-completed`    | `database`, `metrics`, `sessions`, `hosts`           |
| `ingest://host-connected`    | `app`, `database`, `metrics`, `sessions`, `hosts`    |
| `rulebook://fire-received`   | `fires`, `rules`                                     |
| `prs://refreshed`            | `prs`, `gh`                                          |
| `native-index://status`      | `native`, `database`, `metrics`, `sessions`, `hosts` |

These are structural query-key prefixes, not wildcard strings. Future screen queries must include all input dimensions, including range, in their keys. Tests exercise a harness query with reversed request completion; the old range's result cannot replace the current range. The Dashboard keys are `['metrics', 'dashboard', days]` and `['metrics', 'tokens-by-host', days]`, so every ingest and index status event above refreshes them, including a reconcile that enriches existing records without adding any.

## Dashboard metrics

`dashboard(days)` and `tokensByHost(days)` invoke `metrics_dashboard` and `tokens_by_host` with `{ windowDays }`. Only 7, 14 and 30 days are accepted; other values fail before storage is opened. Both return generated DTOs (`DashboardMetrics`, `TokensByHost`).

- **Clock and zone.** The native app uses the system clock and time zone. Fixture mode uses the fixture manifest's pinned `now` and UTC, and reports `clock: "fixture"`. Fixture reports come from the same assembler as native reports, and `pnpm fixtures:export` writes all three ranges into `F1.json`. Reports never contain database paths.
- **One snapshot.** Current and previous windows (the previous window has the same length and ends where the current one starts), coverage and lanes are read in one SQLite read transaction.
- **Tiles.** Each tile has fixed fields: `value`, `unit`, `rule_id`, a `reason` when the value is unknown, an optional `note`, sample counts and a `delta`. `null` means unknown; a measured zero stays `0`. `delta.pct` is in percentage points (`50` means +50%). It is `null` and `suppressed` when either window has fewer than five samples or the previous value is zero. The previous value is kept either way. Samples are sessions for session-derived tiles (token and cost tiles count distinct sessions with selected usage, not responses) and stretches for hands-off. Surfaces excluded from hands-off for unhealthy timestamps are listed in `hands_off_excluded_surfaces` with their qualifying and degenerate session counts. Agent hours per day divides agent hours by the number of local calendar days in the range, including empty days, partial first and last days, and DST days. The tile's `note` states the divisor.
- **Lanes.** Activity lanes always cover the most recent 48 hours (`lane_start_ms`–`lane_end_ms`), whatever range is selected. They are sorted by end, then start (both descending), then session and host, and capped at 200 rows. `lanes_total` and `lanes_truncated` report the cap. The cap affects only which rows are displayed; concurrency and other aggregates use every span.
- **Coverage and cost.** Usage coverage and session capture coverage are reported separately (`pct` is 0–100). `capture_inventory` stays `unknown` because the app does not yet supply a runtime discovery inventory. Capture receipts and native index status do not change it. Cost is labelled API-equivalent and includes the price version, `as_of` date and basis. `total_usd` is `null` when any selected response is unpriced; `priced_subtotal_usd` and the named unpriced reasons (for example, a missing service tier) are reported separately.
- **Unavailable sections.** Merged PRs, rule fires, environment and work type are listed in `unavailable` with reasons; they never report zero. The Dashboard report's `environment` entry stays: Environment data is a separate command, below.

Integers above JavaScript's safe range and non-finite values fail the command rather than being rounded or turned into `null`.

Cleanup cancels the timer and query promises, releases installed listeners, and immediately releases listeners whose asynchronous registration finishes after cleanup. Late callbacks cannot invalidate a disposed client. A subscription failure produces a controlled live-update warning without backend error text. Query cancellation prevents late cache updates; it does not promise to abort an already-running native command.

## Environment

`environment(days)` invokes `metrics_environment` with `{ windowDays }` and returns the generated `EnvironmentMetrics`. Only 7, 14 and 30 days are accepted. Its query key is `['metrics', 'environment', days]`, so every event that refreshes the Dashboard refreshes it too, including enrichment that adds no record.

- **Two disclosed windows.** `window` is the selected range. `strip_window` is fixed: exactly 14 local calendar dates ending with the date that holds the last instant before `now` (the current, possibly partial, local day), starting at that first date's local midnight. Both use the same clock and zone (system, or the fixture's pinned `now` and UTC) and are read in one SQLite snapshot. `selected` and `strip` are the M-17 reports for those windows, preserving raw optional surfaces, unknown identity details and unresolved reasons, including `hook_attribution` for stop-hook summaries.
- **Unknown inventory.** `inventory` is always `unknown`, and each host's M-17 join is `unknown`. The configured components beside it cannot prove a complete callable inventory, so the report contains no not-installed, never-called or used/installed figure and a consumer must not derive one.
- **Rows Rust orders.** `identities` has one row per host and identity (summed over surfaces), with the selected-range `calls`, `strip_calls` and 14 `strip` buckets. `order` sorts by selected calls, then strip calls (both descending), then host and identity. A top slice and the full list are taken in that order; React does not recount.
- **Configured facts.** `configured`, `sources`, `cache` and `roots` are the probe reading described in [Environment probe](ENVIRONMENT.md). They carry structural names, counts and statuses. Discovered paths and command, argument, environment, credential and URL field values are omitted; component names remain visible, including path-like or URL-like text.

Fixture exports include `environments` for all three ranges, produced by the same assembler over the F16 synthetic probe matrix. Integers above JavaScript's safe range fail the command.

## Routes and shell controls

Native windows use `HashRouter` so navigation survives reloads under the native application protocol. Browser previews use `BrowserRouter` with Vite's SPA fallback. The layout shares the existing Sidebar, TopBar, theme provider, fonts, logo, and native drag-region contract.

| Route                          | Placeholder / breadcrumb                            |
| ------------------------------ | --------------------------------------------------- |
| `/first-launch`                | Welcome to XTrace / `first-launch`                  |
| `/dashboard`                   | What your agents did / `dashboard`; metric report   |
| `/sessions?pr=<canonical URL>` | Sessions / `sessions`; retains PR query context     |
| `/prs`                         | Pull requests / `pull-requests`                     |
| `/rulebook`                    | Rulebook / `rulebook`                               |
| `/rulebook/:ruleId`            | Rule detail / `rulebook` / rule ID                  |
| `/rulebook/fires`              | Rule fires / `rulebook` / `fires`                   |
| `/settings`                    | Settings / `settings`; metadata and database counts |
| `/leaderboard`                 | Leaderboard / `leaderboard`                         |

The root route redirects to Dashboard. Unrecognized routes show a page-not-found placeholder. Sidebar selection and breadcrumb derive from the route; Settings and first launch do not highlight an unrelated navigation item. Query parameters are displayed as text only at this stage; there is no session filtering implementation yet.

Cmd+, opens Settings. Cmd+K focuses an enabled page Search if one is present; placeholder screens do not add an unwired search field. Appearance follows the existing system/light/dark preference and persistence behavior. A fixture badge uses `app_info.fixture`, so it also identifies native fixture runs. Missing host and capture data remain explicitly unknown rather than becoming fabricated zeros.

The Shell owns one selected range (7d default, 14d, 30d). The TopBar presets are shown on Dashboard only; the custom-range control stays disabled. The same range keys the Dashboard report and the sidebar `tokensByHost` query, and it is kept when navigating to other routes. The sidebar lists each known host's recorded `total_tokens` for that range, captioned "Recorded tokens · <range>"; an incomplete total shows `—`. Its bar is the host's share of the recorded totals shown, not a quota, limit or reset. The plugin listener stays `unknown`/`off` and capture status stays unknown: historical capture receipts do not mean a surface is capturing now. See [Dashboard acceptance](acceptance/dashboard.md).

On macOS, the platform-specific Tauri configuration uses its built-in `sidebar` window material with a transparent webview. Only the sidebar background is translucent; the content pane, text and controls remain opaque. The native material follows window focus, the selected appearance is also applied to the native window, and Reduce Transparency uses the system opaque material plus a CSS fallback. Other platforms and browser previews retain an opaque sidebar.

On native macOS only, the sidebar reserves 74px above its brand for the existing system window controls. The empty chrome region, brand, and TopBar provide native drag regions; interactive controls exclude dragging. No HTML traffic lights are drawn.

## Verification

Run the browser suites sequentially; they start and stop their own servers:

```sh
pnpm check
pnpm e2e
pnpm e2e:production
```

The default suite includes the shell and all component tests on port 5174 (`E2E_PORT` overrides it). The production suite uses port 5194 (`E2E_PRODUCTION_PORT`). Both share a test-results directory, so run them sequentially. The shell cases run Chromium and WebKit with a 1120×720 viewport and F1 selected. The production suite builds with the fixture flag set, inspects emitted assets, then opens that actual production build in both browsers and checks the unavailable preview. Neither suite emulates Tauri IPC. Browser evidence does not establish native window behavior or an actual Safari 17 platform-floor run.

Rust-dependent DTO generation, fixture parity, license notices and native builds run through the local macOS checkpoint described in [CI](CI.md). The default `pnpm test` and hosted UI job do not require a native toolchain.

The implementation follows the pinned library APIs for [React Router declarative routing](https://reactrouter.com/start/declarative/routing), [TanStack Query invalidation](https://tanstack.com/query/v5/docs/framework/react/guides/query-invalidation), and [query cancellation](https://tanstack.com/query/v5/docs/framework/react/guides/query-cancellation).
