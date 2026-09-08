# Development component gallery

Plan slot: FND-10.

Start `VITE_GALLERY=1 pnpm dev` and open `/gallery`. The left index selects one
declared story, shown in dark and light frames at its declared CSS size. At a
narrow viewport, scroll the review area to inspect each full-size frame. The
gallery has 91 stories covering all 33 component exports in
[`src/kit/index.ts`](../apps/desktop/ui/src/kit/index.ts).

The header labels every example `[SAMPLE] Illustrative component data`. Values,
names and times are newly authored examples, kept apart from DataSource and the
fixture database. Callbacks update local demonstration state. The gallery does
not compute metrics, connect a host or perform a product action.

## Frames and state inventory

Each frame is a same-origin document with its own React root and `ThemeScope`.
This gives each theme an independent native popover top layer and modal focus
scope: opening one theme's popover cannot dismiss the other, and one modal does
not make the surrounding story index inert. Frames load only local source,
fonts and the approved mark. Changing the story replaces both documents and
resets their local state. The gallery does not change system appearance or the
stored theme preference.

[`inventory.json`](../apps/desktop/ui/src/gallery/inventory.json) records every
story ID, covered export and frame size. It is generated from the hand-maintained
story registry, so it proves inventory synchronization rather than independent
design approval. Export coverage and unique IDs are checked automatically;
authors and reviewers remain responsible for meaningful state selection.

| Story family                     | Declared review states                                                                                                                                                                                                      |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Brand, icons, HostGlyph          | Approved mark sizes and label modes; all shared icons; known/unknown hosts at every size, including stacked glyphs                                                                                                          |
| Sidebar                          | Each primary navigation selection, disconnected plugin, unknown/zero token measurements, unmeasured Cursor, disabled navigation, Team, native inset reservation and open Hub                                                |
| TopBar                           | Default, hidden range, scan/copy actions, subcrumb and no action; range and action callbacks are interactive                                                                                                                |
| StatTile, MetricCell, MetricIcon | All seven icons, positive/negative delta, unknown and zero, long aside, numeric/string/unknown formats, all metric sizes, alignment and semantic icon tones                                                                 |
| SectionCard, definitions         | Default/compact headers, footer and slots; RuleChip sizes; open contextual definition                                                                                                                                       |
| Badges                           | All kinds and evidence levels; StatePill tones, heights, outline and explicit live appearance                                                                                                                               |
| Controls                         | Toggle on/off and disabled; range/mode/disabled Segmented; Button variants and heights, icons and disabled state; ProgressBar heights/composition/zero/unknown/clamping; empty/filled/disabled Search and a scoped shortcut |
| Tables and cells                 | All five row heights, expanded details, sorting and row actions, empty/loading/muted/sticky states, independent title ellipsis, known/zero/unknown numbers and muted mono text                                              |
| Filters and microcharts          | Filter closed/open/all/empty states with known/unknown hosts and zero/unknown counts; SparkBars hours/fires/share plus zero/unknown/clamping; 14-day threshold strip; dot/swatch legends                                    |
| Native overlays                  | Popover and Modal open/closed; Hub connected/disconnected. Both themes can keep their native overlays open simultaneously                                                                                                   |

Sidebar is 228×900, TopBar 1200×44, StatTile 300×52 and tables 1000px wide.
Open-overlay examples reserve additional space around the component so the
native top layer can be inspected. Reserved native chrome contains no imitation
window buttons. Theme-paired screenshots are in
[FND-10 acceptance](acceptance/FND-10.md).

## Add or update a story

Add a render function through `story(id, components, [width, height], Render)`
in the appropriate `src/gallery/stories/*.stories.tsx` file. A component export
must appear in the barrel and at least one story. Keep story inputs synthetic,
labels explicit, state controlled locally and frames large enough to expose
the intended content. Do not hide overflow to make a failing check pass.

Refresh the review inventory after changing an ID, size or covered export:

```sh
UPDATE_GALLERY_INVENTORY=1 pnpm --dir apps/desktop/ui exec vitest run src/gallery
pnpm exec prettier --write apps/desktop/ui/src/gallery/inventory.json
```

Run checks from the repository root:

```sh
pnpm check
pnpm e2e
pnpm --dir apps/desktop/ui test:e2e:gallery
pnpm --dir apps/desktop/ui test:e2e:gallery-production
```

Both dedicated commands use port 5183, start and stop their own server, and must
run sequentially. The positive suite traverses every declared story in Chromium
and WebKit and writes paired review captures into ignored `test-results`. It
also exercises index navigation, independent controls, modal dismissal and
state reset. The production command builds with both development flags set,
inspects the actual output, then opens `/gallery` in both browsers and expects
the ordinary not-found page. The default browser suite verifies that the route
is unavailable when the development gallery flag is absent.

## Production boundary and design limits

The sole product import is a lazy route guarded by
`import.meta.env.DEV && import.meta.env.VITE_GALLERY === '1'`. The iframe module
is not a build entry. Vite removes the guarded route and its source graph during
production compilation, even when `VITE_GALLERY=1` is supplied.

The repository ESLint rule rejects product imports/re-exports of gallery code,
including static dynamic imports, require calls, path templates and globs that
can reach the gallery. Only the exact guarded route import is allowed. Its
negative test writes a real temporary product source file and runs the actual
lint configuration. This code-review and build boundary is not a sandbox for
hostile source changes.

The current components and this gallery are the working design reference.
Review state coverage, layout and interactions here in both themes. FND-11 will
capture versioned visual regression baselines from explicitly reviewed stories;
record the source SHA, story inventory, theme, viewport, DPR, local fonts and
pinned browser/runner with each baseline set. Review before/after/diff images
when changing baselines; a failing comparison must not automatically rewrite
its expected image. Baseline creation and visual regression testing remain
FND-11 work, not completed gallery coverage.

This reference choice does not approve every current pixel or a checkpoint.
These captures do not prove parity with an independent design export. Any later
external design source needs its own publication review before inclusion.
