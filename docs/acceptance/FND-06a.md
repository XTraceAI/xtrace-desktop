# FND-06a acceptance

The component kit now supplies a presentational Sidebar, HubPopover, and
BrandMark. The existing production scaffold is unchanged. Later shell work can
pass real navigation, measurements, and status into these components without
adding data fetching to the kit.

## Verification

- `pnpm check`: typechecking, ESLint, the hex guard, formatting, 22 Vitest tests
  (7 new component cases and one added Popover lifecycle case), and the inherited Node lint test pass.
- `pnpm build`: the existing Safari 17-targeted production build passes.
- `pnpm --dir apps/desktop/ui exec vitest run src/kit/Sidebar.test.tsx
src/kit/HubPopover.test.tsx src/kit/BrandMark.test.tsx`: all 7 cases pass.
- `pnpm --dir apps/desktop/ui exec playwright test --config
playwright.sidebar.config.ts`: 1 scenario passes on Playwright 1.63.0 / WebKit
  26.6, with no browser console errors or page exceptions.

The five inherited WebKit theme/overlay scenarios also pass after the optional
positioning ref was added to Popover.

| Case                    | Expected result and evidence                                                                                                                                                                                                                                                                   |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controlled navigation   | Observe/Govern/Community render with five default items. Clicking an enabled item calls the handler; only a new `activeKey` changes selection. A positive Rulebook badge appears and disappears at zero.                                                                                       |
| Disabled actions        | Leaderboard shows “soon” and cannot navigate until enabled. Team remains hidden by default and requires both visibility and connection props to be actionable. Unwired Settings and Hub connection actions are disabled.                                                                       |
| Measurements            | Measured zero displays `0` and no bar, while `null` displays `—` and no bar even if given a positive fill. Empty host measurements are explicit.                                                                                                                                               |
| Capture coverage        | Capturing CLI, not-capturing Desktop, an unfamiliar raw surface, and a missing surface identity retain independent visible states. Missing coverage remains unknown.                                                                                                                           |
| Footer props            | Listener off/unknown/port, version, optional update label, team label, settings action, and appearance action come from props. There is no inferred “up to date” or host-wide capture success.                                                                                                 |
| Hub behavior            | Native keyboard opening, Escape and explicit close, trigger focus return, repeated reopening, and outside dismissal pass in WebKit. The connected title and unwired/wired CTA states pass isolated component tests.                                                                            |
| Geometry and appearance | WebKit checks 228×900 sidebar dimensions, 16px default top padding, 32px navigation rows, active color, light Hub canvas, 264px Hub width, a 14px gap outside the sidebar, a 6px bottom offset from the invoker, and a 74px native inset reservation. The approved mark renders at 20/26/34px. |

## Evidence

- [Dark sidebar, 228×900](FND-06a/sidebar-dark.png)
- [Light sidebar, 228×900](FND-06a/sidebar-light.png)
- [Light Hub popover](FND-06a/sidebar-hub.png)
- [Brand sizes and reserved native inset](FND-06a/brand-sizes-native-inset.png)
- [Component props and complete expected states](../SIDEBAR.md)

These captures come from the synthetic fixture under `e2e/`; no personal
desktop content or real measurements are included. Both themes were checked by
emulated media and the existing theme toggle. System appearance was unchanged.

## Review boundaries

Only Sidebar, HubPopover, BrandMark, and their shared styles are added to the
production component tree. Their test fixture is not a production app route.
The only local UI state is whether Hub is open; FND-05 Popover retains native
dismissal and focus behavior. No duplicate global overlay listeners are added. The shared Popover accepts an optional geometry ref while retaining its original invoker for focus. Detaching either target dismisses it safely.

The approved image replaces the legacy parent card's gradient placeholder.
The Hub CTA retains a theme-dependent token foreground to keep stronger contrast on the dark theme’s lighter accent. Shared icons and HostGlyph remain FND-06b responsibilities and can fill the
decorative slots. Hub explanatory copy is new public copy because the original
design export is unavailable. The evidence checks the technical geometry and
theme contract; it does not claim recovered-export pixel parity.

This run uses current WebKit 26.6 rather than an installed Safari 17.0 build.
Actual macOS 14 / Safari 17 qualification remains pending with FND-05. No native
window controls, native drag behavior, real connection, router, or capture
pipeline are changed or claimed tested by this component PR.
