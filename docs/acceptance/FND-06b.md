# FND-06b acceptance

Plan slot: FND-06b

TopBar, HostGlyph, and shared icons now provide controlled component primitives
on the FND-05 design foundation. The production scaffold is unchanged. This
slice is independent of FND-06a and includes no Sidebar implementation.

## Verification

- `pnpm check`: typechecking, ESLint, the hex guard, formatting, 23 Vitest tests
  (9 new cases), and the inherited Node lint test pass.
- `pnpm --dir apps/desktop/ui exec vitest run src/kit/TopBar.test.tsx
src/kit/HostGlyph.test.tsx`: all 9 focused cases pass.
- `pnpm build`: the existing Safari 17-targeted production build passes.
- `pnpm --dir apps/desktop/ui exec playwright test --config
playwright.topbar.config.ts`: 1 scenario passes on Playwright 1.63.0 / WebKit
  26.6, with no browser console errors or page exceptions.

| Case              | Expected result and evidence                                                                                                                                                                                                  |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controlled range  | Clicking 30d calls `onRange('30d')` once. Selection remains on 7d until a rerender supplies 30d. WebKit also verifies Tab and Enter activation of 14d.                                                                        |
| Actions           | Scan, copy, and share invoke the supplied action once without invoking the range callback. The browser fixture demonstrates each dispatch.                                                                                    |
| Optional controls | `showRange=false` removes range controls. Missing or blank action text leaves no focusable ghost button. A visible unwired action is disabled.                                                                                |
| Host identity     | Claude, Codex, and Cursor have their own labels and token colors. Missing and unfamiliar hosts, including object-property names, stay visibly unknown.                                                                        |
| Glyph geometry    | All 16/18/20/30px variants retain their size and real 600 weight. Stacked 20px glyphs overlap by 4px. Known hosts retain fixed white foreground; unknown hosts use neutral theme colors.                                      |
| Minimum width     | TopBar measures 1200×44 at design width. At an actual 1120px browser viewport, reserving 228px leaves 892×44 for content. Long breadcrumbs truncate, controls remain inside their bounds, and page scroll width stays 1120px. |

## Evidence

- [Dark TopBar, 1200×44](FND-06b/topbar-dark.png)
- [Light TopBar, 1200×44](FND-06b/topbar-light.png)
- [Dark minimum-width frame](FND-06b/minimum-dark.png)
- [Light minimum-width frame](FND-06b/minimum-light.png)
- [Dark host sizes and shared icons](FND-06b/glyphs-dark.png)
- [Light host sizes and shared icons](FND-06b/glyphs-light.png)
- [Props, expected states, and provenance](../TOPBAR.md)

All captures are synthetic component fixtures. The blank 228px strip in the
minimum-width frame reserves the sidebar's width; it does not implement a
Sidebar or draw native controls. Appearance uses emulated media, without
changing the user's system setting or capturing their desktop.

## Limits

U-02 range dispatch is covered; metric recomputation and sample-count-based
delta suppression are not performed by this kit. No router, native commands,
data fetching, or action workflow is added. The approved logo and native window
controls are unchanged.

The parent contract's unbundled Geist Mono 700 is reconciled to real 600. The
Codex circle glyph uses inline vectors because that character is not in the
bundled font. The shared icons are new line drawings; screenshots establish
technical geometry and token behavior, not parity with an unavailable export.

The run uses current WebKit 26.6 rather than an installed Safari 17.0 build.
Actual macOS 14 / Safari 17 qualification remains pending with FND-05.
