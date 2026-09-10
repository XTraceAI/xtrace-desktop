# Component gallery acceptance

The opt-in development gallery uses labelled, authored sample inputs. Selecting
a story displays the actual shared components in independent dark and light
frames. Product screens continue to read their existing DataSource.

## Test plan and expected results

| Command                                                  | Expected result                                                                                                                                                                                                                                                                               |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm check`                                             | Typecheck, lint, formatting and unit tests pass. Four gallery cases verify the kit export inventory, unique story IDs, synchronized sizes and the actual import restriction.                                                                                                                  |
| `pnpm test:ci`                                           | Existing CI helper tests pass.                                                                                                                                                                                                                                                                |
| `pnpm e2e`                                               | Existing shell and component interactions pass in Chromium and WebKit. `/gallery` shows the ordinary not-found page when the gallery flag is absent.                                                                                                                                          |
| `pnpm e2e:gallery:smoke`                                 | Hosted subset: Sidebar with open Hub, TopBar, Button variants and expanded DataTable render in both themes and browsers (16 frames), with frame/popup bounds, browser-error and external-request checks. Independent toggles, modal dismissal and navigation reset also pass in both engines. |
| `pnpm e2e:gallery`                                       | Local full sweep: all 92 stories render in both themes in both browsers (368 frames), with the same layout and interaction checks as smoke plus paired review captures. Required before merging gallery, shared-component, theme, font or brand-asset changes. Record source SHA and results. |
| `pnpm e2e:production`                                    | One build with both `VITE_GALLERY=1` and `VITE_XTRACE_FIXTURE=F1` contains no gallery route, frame/story assets or sample/fixture payload. Both browsers show the not-found page at `/gallery` and preserve the production shell's unavailable-data behavior.                                 |
| `pnpm check:native --base <reviewed-base-sha> --release` | Full local Rust/DTO/dependency checks and debug/release launch checks pass on the final source commit.                                                                                                                                                                                        |

The gallery suite owns port 5183 and uses a 2880×1120 viewport. Production
checks use port 5194 and 1120×720. Run the browser suites sequentially.
Playwright 1.58.2 supplies Chromium and WebKit; all fonts and brand assets are
local. The existing Ubuntu UI job runs only the gallery smoke subset and shares
the shell's production build. The full sweep stays local; hosted success does
not certify all story layouts. No new hosted job or macOS runner is added.

Local browser validation passes all 48 ordinary cases, all four full-gallery
cases, all four smoke cases and all four shared production cases. `pnpm check` passes 78 Vitest cases and
three script cases; `pnpm test:ci` passes 23 cases. Exact source/base and local
native validation results are recorded in the PR description.

## Review evidence

The 92-story inventory covers all 33 primary kit component exports. Export
coverage checks declaration and inventory consistency; reviewers still assess
whether the chosen states are meaningful. The gallery guide describes each
[story family](../GALLERY.md#frames-and-state-inventory).

| Story                                              | Paired dark/light capture                                               |
| -------------------------------------------------- | ----------------------------------------------------------------------- |
| Sidebar with open Hub                              | [Capture](component-gallery/sidebar-hub-popover-open.png)               |
| TopBar                                             | [Capture](component-gallery/topbar-default.png)                         |
| Button variants, heights, disabled state and icons | [Capture](component-gallery/button-variants-heights-disabled-icons.png) |
| Expanded table                                     | [Capture](component-gallery/datatable-rulebook-44-expanded.png)         |
| Modal                                              | [Capture](component-gallery/modal-open.png)                             |

Captures include the sample label and complete component frames. Browser traces
and unrelated desktop captures are not committed. The gallery reserves sidebar
space for native controls without drawing imitation buttons. Its browser frames
cannot verify native dragging, macOS sidebar material, or the macOS 14 support
floor. These images are review evidence; separately approved visual regression
baselines and independent design-export comparisons remain future work.
