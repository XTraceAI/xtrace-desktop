# Desktop shell acceptance

The desktop scaffold provides a reproducible shell and workspace. Capture, metrics, storage and Rulebook behavior are not implemented. Required commands are in [DEVELOPING.md](../../DEVELOPING.md).

| Setup and action                                                                                                                | Expected result                                                                                                                                                                                                                        | Required evidence                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Fresh macOS checkout: frozen pnpm install, UI checks, workspace fmt/Clippy/tests and debug app build                            | All checks succeed without changing lockfiles; a conflicting `MACOSX_DEPLOYMENT_TARGET=15.0` still builds for macOS 14.0. All eight subsystem crates, xtask and the Tauri application compile.                                         | Command results tied to the PR head; OS/toolchain versions.                                                                   |
| Open the native bundle; refresh application information                                                                         | XTrace brand and real app name/version appear through the Rust command; browser preview labels its own limitation.                                                                                                                     | Native UI observation; Vitest success/error/retry coverage.                                                                   |
| Resize between default and minimum size; drag sidebar/header; use Refresh, minimize/restore, fullscreen/return and close/reopen | Native buttons retain roughly 16-point top/left margins, including after maximize → fullscreen → exit without a manual resize. No overlap or separate title strip; dragging preserves button clicks.                                   | Native observations on the actual bundle.                                                                                     |
| Launch a second copy while the first is running                                                                                 | The existing process/window is focused; no second application owner remains.                                                                                                                                                           | Process/window observation.                                                                                                   |
| Inspect source layout, toolchain pins and legal files                                                                           | Subsystem boundaries is documented. Apache LICENSE, NOTICE, DCO, trademarks and contribution/conduct/security guidance exist. Every PR commit is signed off.                                                                           | Reviewed file list, provenance and git log.                                                                                   |
| Regenerate icons; build and open the native bundle; inspect the browser preview                                                 | The published XTrace mark appears without clipping in the sidebar and welcome screen. The favicon and native icon center it on dark charcoal with 15% padding per edge; the bundle contains the generated ICNS selected by Info.plist. | Source checksum/provenance, background/border measurement, generated-asset parity, browser screenshot and native observation. |

Rust placeholder crates are validated by compilation. The evidence below applies to this scaffold's source and native bundle. CI has a separate [acceptance contract](ci.md).

Packaged-font acceptance: after `pnpm build`, inspect every font URL in `apps/desktop/ui/dist/assets/*.css`. Each must point to an existing bundled asset, with no `data:font` URLs. Open the rebuilt native bundle and check Web Inspector for font-loading/CSP errors. Expected: Manrope and Geist Mono load from the app origin under `font-src 'self'`; no font requests are blocked. Record build inspection and native console results separately from browser-preview checks.

## Recorded shell evidence

Source commit `70f233da74d6f1fe5244096ef56c9efaf8ec1396` passed a fresh-clone
frozen install, UI checks and four unit tests, workspace fmt/Clippy/tests and a
debug macOS build without lockfile changes. The executable declares macOS 14.0
as its minimum version. Placeholder Rust crates have no behavior tests.

Native observations on macOS 26.5.2 confirmed application metadata, refresh,
default/minimum window layouts, sidebar/header dragging, fullscreen/return,
minimize/restore and close/reopen. A second launch exited while one application
process remained. Manual dragging was also confirmed on `faeaf21`.

The current window configuration `{ x: 16, y: 26 }` measured 15 points from the
left and 16 points from the top to the close-button frame, with 16×16 native
buttons and system spacing at startup, fullscreen return and minimize/restore.
AppKit retains its standard active/inactive appearance. Measure insets without
screen sharing, which can insert a system indicator and alter control placement.

The published mark's provenance is in [TRADEMARKS.md](../../TRADEMARKS.md).
Icon generation was byte-identical across two runs. The generated favicon and
ICNS matched the built outputs; `CFBundleIconFile` selected that ICNS. Icons use
opaque `#17181b` with 15% padding per edge. The ICNS contains eight PNG-backed
representations to avoid a decoder artifact in legacy RGB records.

The [browser preview screenshot](desktop-shell-browser.jpg) records the renderer
layout at 1440×900. It showed the published mark, no browser errors and only
same-origin requests, including bundled fonts. It does not verify native controls.

With `MACOSX_DEPLOYMENT_TARGET=15.0` inherited, the workspace still built with
Mach-O `minos 14.0`. An isolated build-script probe confirmed the repository's
forced target reached the compiler. Disabling Vite asset inlining resolved native
font CSP errors: all 60 CSS font references resolved to bundled files, with no
inline font URLs, and the rebuilt app showed no font/CSP errors in Web Inspector.

These are recorded observations of the shell changes, not a fresh native test of
every later PR. Changes affecting these behaviors must repeat the relevant cases
and record current source, platform and results in their PR verification.
