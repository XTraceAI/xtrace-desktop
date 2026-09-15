# App data and routing

The shell reads app metadata and database counts through `DataSource`. It does not calculate metrics or populate future screens. DTOs are generated from Rust into `apps/desktop/ui/src/data/generated/`; regenerate and verify them with `cargo xtask dto-export` and `bash scripts/ci/check-dto.sh`. Generated files retain the generator's formatting.

## Selecting a source

Native Tauri always selects `TauriDataSource`, including when `VITE_XTRACE_FIXTURE` is set. Its `appInfo()`, `dbCounts()` and `nativeIndexStatus()` methods invoke the `app_info`, `db_counts` and `native_index_status` commands; the last returns the generated `NativeIndexStatus` (phase, freshness, interpreter, bundled readers and each host's last scan), which the app also publishes on every change as the `native-index://status` event. Command and event names live in `src/data/ipc-names.ts`; components import the generated types and use query hooks rather than calling IPC.

For a browser fixture preview:

```sh
pnpm fixtures:export
VITE_XTRACE_FIXTURE=F1 pnpm dev
```

`FixtureDataSource` reads the generated `apps/desktop/ui/fixtures/F1.json` shell export. It returns copies of the same `AppInfo` and `DbCounts` shapes used by native IPC. The export contains a portable `fixture://F1` label and canonical counts of 1 session, 25 records, and 15 usage rows. Its `emit(event)` helper delivers deterministic test events without timers or a simulated ingestion service. F2, F20, and unrecognized fixture names fail explicitly; they have no populated shell export.

The fixture adapter is dynamically imported only inside a Vite `DEV` condition. A production build ignores `VITE_XTRACE_FIXTURE`, even when it is set during compilation, and excludes the adapter chunk and fixture JSON. A browser without an enabled development fixture shows an explicit unavailable preview. It never falls back to sample data. Native fixture selection and temporary database ownership are enforced separately by the Rust application.

## Queries and events

`DataProvider` owns one active QueryClient for the window's selected source. Replacing the source remounts the query runtime, creating an isolated client and fresh query observers and warning state. Queries use a one-second stale time, no automatic retry, and no focus refetch. The shell's metadata consumers share `['app', 'info']`, `['database', 'counts']` and `['native', 'index']`, so reading the same data in the sidebar and Settings shares a cache entry. Refresh explicitly refetches the first two; the index status refreshes on its own event.

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

These are structural query-key prefixes, not wildcard strings. Future screen queries must include all input dimensions, including range, in their keys. Tests exercise a harness query with reversed request completion; the old range's result cannot replace the current range. No metric query method or output is introduced by this shell.

Cleanup cancels the timer and query promises, releases installed listeners, and immediately releases listeners whose asynchronous registration finishes after cleanup. Late callbacks cannot invalidate a disposed client. A subscription failure produces a controlled live-update warning without backend error text. Query cancellation prevents late cache updates; it does not promise to abort an already-running native command.

## Routes and shell controls

Native windows use `HashRouter` so navigation survives reloads under the native application protocol. Browser previews use `BrowserRouter` with Vite's SPA fallback. The layout shares the existing Sidebar, TopBar, theme provider, fonts, logo, and native drag-region contract.

| Route                          | Placeholder / breadcrumb                            |
| ------------------------------ | --------------------------------------------------- |
| `/first-launch`                | Welcome to XTrace / `first-launch`                  |
| `/dashboard`                   | Dashboard / `dashboard`                             |
| `/sessions?pr=<canonical URL>` | Sessions / `sessions`; retains PR query context     |
| `/prs`                         | Pull requests / `pull-requests`                     |
| `/rulebook`                    | Rulebook / `rulebook`                               |
| `/rulebook/:ruleId`            | Rule detail / `rulebook` / rule ID                  |
| `/rulebook/fires`              | Rule fires / `rulebook` / `fires`                   |
| `/settings`                    | Settings / `settings`; metadata and database counts |
| `/leaderboard`                 | Leaderboard / `leaderboard`                         |

The root route redirects to Dashboard. Unrecognized routes show a page-not-found placeholder. Sidebar selection and breadcrumb derive from the route; Settings and first launch do not highlight an unrelated navigation item. Query parameters are displayed as text only at this stage; there is no session filtering implementation yet.

Cmd+, opens Settings. Cmd+K focuses an enabled page Search if one is present; placeholder screens do not add an unwired search field. Appearance follows the existing system/light/dark preference and persistence behavior. A fixture badge uses `app_info.fixture`, so it also identifies native fixture runs. Missing host and capture data remain explicitly unknown rather than becoming fabricated zeros.

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
