# FND-01 acceptance

The first foundation PR creates a reproducible shell and workspace. It closes no capture, metric, storage or Rulebook behavior. Required commands are in [DEVELOPING.md](../../DEVELOPING.md).

| Setup and action                                                                                                                | Expected result                                                                                                                                                     | Required evidence                                           |
| ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| Fresh macOS checkout: frozen pnpm install, UI checks, workspace fmt/Clippy/tests and debug app build                            | All checks succeed without changing lockfiles. All eight subsystem crates, xtask and the Tauri application compile.                                                 | Command results tied to the PR head; OS/toolchain versions. |
| Open the native bundle; refresh application information                                                                         | XTrace brand and real app name/version appear through the Rust command; browser preview labels its own limitation.                                                  | Native UI observation; Vitest success/error/retry coverage. |
| Resize between default and minimum size; drag sidebar/header; use Refresh, minimize/restore, fullscreen/return and close/reopen | Native buttons remain above the brand without overlap; no separate title strip. Dragging works without swallowing button clicks.                                    | Native observations on the actual bundle.                   |
| Launch a second copy while the first is running                                                                                 | The existing process/window is focused; no second application owner remains.                                                                                        | Process/window observation.                                 |
| Inspect source layout, toolchain pins and legal files                                                                           | Binding subsystem ownership is documented. Apache LICENSE, NOTICE, DCO, trademarks and contribution/conduct/security guidance exist. Every PR commit is signed off. | Reviewed file list, provenance and git log.                 |

Rust placeholder crates are validated by compilation. The evidence below applies to this scaffold's source and native bundle. FND-02 supplies repository CI; its acceptance remains separate.

## Recorded evidence

Source commit `70f233da74d6f1fe5244096ef56c9efaf8ec1396` passed a fresh-clone frozen install, `pnpm check` (four UI tests), `pnpm test -- --run`, workspace fmt/Clippy/tests and a debug macOS app build, with unchanged lockfiles. Rust placeholder crates have no behavior tests; compilation validates their initial boundaries. The built executable declares macOS 14.0 as its minimum version.

Native observations on macOS 26.5.2 confirmed real application metadata, refresh, default/minimum window layouts, sidebar/header dragging, fullscreen/return, minimize/restore and close/reopen. A second launch exited successfully while one application process remained. These observations apply to the packaged native app.

The [browser preview screenshot](FND-01-browser.png) records the renderer layout only. Its smoke check reported no browser errors and only same-origin requests, including locally bundled fonts. It is not evidence for native window controls. Later evidence-only commits do not change the tested source.
