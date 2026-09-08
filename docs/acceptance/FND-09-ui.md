# FND-09 frontend acceptance

Plan slot: FND-09. This evidence covers the typed frontend seam and routed shell. Rust DTO generation, database isolation, and release fixture guards have their own native checks.

## Verified behavior

- The generated F1 export and native IPC adapters expose the same typed metadata/count shapes. Native presence takes precedence over a Vite fixture flag; unsupported browser fixtures fail explicitly.
- Query keys isolate reversed range completion, and source replacement isolates caches. One 500ms event coordinator coalesces repeated events across explicit hierarchical prefixes. Strict Mode and delayed registration cleanup balance listeners; synchronous and asynchronous registration failures use a controlled warning.
- Route navigation updates Sidebar selection and TopBar breadcrumbs. All nine placeholders and the not-found route open directly. Sessions retain the PR query string across reloads, and Cmd+, opens Settings. A focused component test verifies Cmd+K with the real Search component and leaves focus unchanged when no Search exists.
- Native macOS chrome reservation and interactive drag exclusions are asserted in the component tree. Browser captures contain no fake window controls.
- Both browsers render the canonical F1 counts and visible badge, support theme changes, fit 1120×720 without horizontal overflow, load bundled fonts with external network denied, and navigate after going offline.
- An actual production build made with `VITE_XTRACE_FIXTURE=F1` contains no fixture export or adapter chunk. Both browsers show the explicit unavailable preview from that build, with no fixture badge or database counts.

## Commands and results

| Check                                                                  | Result                                                            |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `pnpm typecheck`                                                       | Pass                                                              |
| `pnpm lint`                                                            | Pass, including semantic-token hex lint                           |
| `pnpm test`                                                            | 73 Vitest cases and 3 Node cases pass; 16 focused FND-09 cases    |
| `pnpm --dir apps/desktop/ui test:e2e:shell`                            | 6 pass: 3 scenarios × WebKit and Chromium                         |
| `pnpm --dir apps/desktop/ui exec playwright test e2e/overlays.spec.ts` | 5 inherited WebKit theme/overlay cases pass                       |
| `pnpm --dir apps/desktop/ui test:e2e:production`                       | 2 pass: production exclusion and browser behavior in both engines |

The browser commands each start and stop port 5181 and must run sequentially. The production command includes the UI production build. Both engines use Playwright 1.58.2 at 1120×720; system appearance is emulated per browser context. Screenshots below are synthetic test output using only the portable F1 export.

| Engine   | Dark Dashboard                            | Light Settings                             |
| -------- | ----------------------------------------- | ------------------------------------------ |
| WebKit   | [Capture](FND-09/shell-webkit-dark.png)   | [Capture](FND-09/shell-webkit-light.png)   |
| Chromium | [Capture](FND-09/shell-chromium-dark.png) | [Capture](FND-09/shell-chromium-light.png) |

The empty Dashboard is intentional: metrics and product screen behavior belong to later work. Settings displays metadata and counts from the source seam. Browser results do not claim native IPC end-to-end execution, actual macOS window-control acceptance, or the Safari 17 platform floor. See [the data and routing contract](../DATA-SOURCE.md) for the interfaces and limits.
