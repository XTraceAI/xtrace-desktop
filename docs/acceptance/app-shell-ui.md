# App shell frontend acceptance

The app now composes the shared Sidebar and TopBar around routed placeholders. Settings reads app metadata and database counts through the same data interface used by native IPC. Dashboard and the other product pages remain placeholders; this is not acceptance of finished screen content.

## Verified behavior

- Native presence takes precedence over a browser fixture flag. Unsupported development fixtures fail explicitly; a production browser shows an honest unavailable preview.
- Source replacement gives queries a fresh cache and observers. Query keys keep reversed request completion from overwriting another range. One 500ms event coordinator coalesces updates without starving continuous imports; Strict Mode, late registration, disposal and registration failure have regression coverage.
- Navigation updates the selected Sidebar item and TopBar breadcrumb. Nine routes and the not-found page support direct load and reload. Sessions preserve PR query context as text. Cmd+, opens Settings; Cmd+K focuses a real page Search when present. Leaderboard stays the kit's disabled “soon” row in the app: a pointer press or the keyboard never leaves the current page, while `/leaderboard` still opens its named placeholder directly.
- Settings supports system/light/dark appearance and a database refresh. Empty host/capture information stays unknown instead of displaying sample measurements. Synthetic F1 counts are 1 session, 25 records and 15 usage rows.
- Both browsers fit the shell at 1120×720, load bundled fonts without external requests, and continue navigation while offline. Native window controls are not drawn in HTML.
- A real production build made with the fixture environment flag excludes fixture data and the adapter chunk. Both browsers show that build without a fixture badge or database counts.

## Commands and results

| Check                                    | Result                                                                                              |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `pnpm check`                             | Types, lint, formatting, 74 Vitest cases and the Node lint regression pass.                         |
| `pnpm e2e`                               | 46 cases pass in WebKit and Chromium, including 6 shell cases and the shared component regressions. |
| `pnpm e2e:production`                    | 2 cases pass against the actual production build.                                                   |
| `pnpm e2e --grep 'navigates real shell'` | Both engines pass after generating the final paired Dashboard captures.                             |

Run the browser commands sequentially because they share the output directory. The default server uses port 5174 (`E2E_PORT`); production uses 5194 (`E2E_PRODUCTION_PORT`). The production suite performs the build itself. These are installed Playwright engines, not an actual Safari 17 platform-floor run.

All screenshots are synthetic browser output using the portable fixture export. The native sidebar material is verified separately in [native acceptance](app-shell-native.md).

| Engine   | Dark Dashboard                               | Light Dashboard                               |
| -------- | -------------------------------------------- | --------------------------------------------- |
| WebKit   | [Capture](app-shell/shell-webkit-dark.png)   | [Capture](app-shell/shell-webkit-light.png)   |
| Chromium | [Capture](app-shell/shell-chromium-dark.png) | [Capture](app-shell/shell-chromium-light.png) |

See [the data and routing contract](../DATA-SOURCE.md) for interfaces, event mappings and launch commands.
