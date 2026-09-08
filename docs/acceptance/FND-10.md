# FND-10 gallery acceptance

Plan slot: FND-10. The gallery is an opt-in development review surface with newly
authored illustrative inputs. It does not supply product data or implement a
metric screen.

## Verified behavior

- The explicit inventory contains 91 unique stories and covers all 33 exported
  kit components. Its generated ID/export/size records match the story registry.
  Reviewers still own semantic state completeness; export reflection alone does
  not prove every possible prop combination.
- Every story renders in isolated dark and light documents in Chromium and
  WebKit: 182 pairs, or 364 individual frame renders. Frames fit their declared
  dimensions with no horizontal or vertical overflow, console errors or external
  HTTP requests. Sidebar itself remains 228×900, including the open Hub example.
- Navigation updates the selected story and replaces both frame documents.
  Controlled state changes in one theme leave the other unchanged. Both modal
  dialogs can remain open; Escape closes only the focused frame's dialog and the
  parent index remains usable. Returning to a story resets its local state.
- The gallery label is explicit and its mark and fonts come from existing local
  assets. Appearance is emulated inside browser contexts; no system settings are
  changed and no imitation native window controls are drawn.
- Without the development flag, `/gallery` shows the normal not-found page.
  An actual production build with `VITE_GALLERY=1` and `VITE_XTRACE_FIXTURE=F1`
  contains no gallery route, story/frame chunk, illustrative sample payload or
  fixture export. Both browsers confirm the not-found route without frames.
- The actual repository ESLint configuration rejects a temporary product import
  of `gallery/sample`, as well as re-exports, unguarded entry imports, dynamic
  imports, require, path templates and globs that can reach gallery code. The
  sole permitted import is the exact development-gated lazy route.

## Commands and results

| Check                                                    | Result                                                                                       |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `pnpm check`                                             | Typecheck, ESLint, token hex lint and formatting pass; 77 Vitest cases and 3 Node cases pass |
| `pnpm test:ci`                                           | 17 Node cases pass                                                                           |
| `pnpm --dir apps/desktop/ui exec vitest run src/gallery` | 4 focused inventory and import-boundary cases pass                                           |
| `pnpm e2e`                                               | 28 combined boot/component cases pass in WebKit and Chromium, including gallery disabled     |
| `pnpm --dir apps/desktop/ui test:e2e:gallery`            | 4 cases pass: complete traversal and frame interaction in both engines                       |
| `pnpm --dir apps/desktop/ui test:e2e:gallery-production` | Production build/output inspection and browser exclusion pass in both engines                |

Dedicated gallery commands own port 5183 and run sequentially. Playwright 1.58.2
uses both WebKit and Chromium; the traversal viewport is 2880×1120 so even the
two 1200px TopBars fit side by side. The production test uses 1120×720. The five
paired WebKit captures below show only the synthetic browser gallery; capture
regions retain the sample label and both full-size component frames.

| Story                                        | Paired dark/light capture                                    |
| -------------------------------------------- | ------------------------------------------------------------ |
| Sidebar with open Hub                        | [Capture](FND-10/sidebar-hub-popover-open.png)               |
| TopBar                                       | [Capture](FND-10/topbar-default.png)                         |
| Button variants, heights, disabled and icons | [Capture](FND-10/button-variants-heights-disabled-icons.png) |
| Expanded table                               | [Capture](FND-10/datatable-rulebook-44-expanded.png)         |
| Native modal                                 | [Capture](FND-10/modal-open.png)                             |

See [the gallery guide](../GALLERY.md) for the complete state-family summary,
machine-readable inventory, adding stories and updating evidence. Browser
traces and unrelated desktop captures are not committed.

These local checks do not claim an actual macOS 14/Safari 17 floor run for the
new gallery. CI includes both gallery commands on its macOS 14 runner. No native
app launch is required by this change. The current components and gallery serve as the working design reference.
These captures remain review evidence; FND-11 must establish explicitly reviewed,
versioned regression baselines before claiming automated visual coverage. This
does not approve every current pixel or claim parity with independent exports;
the optional source-design iframe/diff view remains absent.
