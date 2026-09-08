# FND-05 acceptance

This change replaces the shell's fixed dark palette with the reconciled design
token contract and persisted System/Dark/Light appearance. It supplies local
font weights and native Popover/Modal primitives for later component consumers.
The overlay fixture is test-only; the production app exposes the appearance
selector and retains the existing app-information preview.

## Verification

- `pnpm check`: typechecking (including browser tests), ESLint, hex guard,
  formatting, 14 Vitest cases, and 1 Node lint test pass.
- `pnpm test -- --run`: compatibility with the existing test entry point passes.
- `pnpm build`: Safari 17-targeted production build passes; external asset paths
  remain compatible with the native `font-src 'self'` CSP.
- `pnpm --dir apps/desktop/ui exec playwright test e2e/overlays.spec.ts
--project=webkit`: all 5 tests pass with Playwright 1.63.0 / WebKit 26.6.
- Font regeneration using pinned FontTools 4.64.0, Brotli 1.2.0, and Zopfli 0.4.3
  reproduces all 16 WOFF2/TTF binaries byte-for-byte. All eight TTFs have the
  requested static OS/2 weight, no variable axis, and source glyph coverage.

The browser tests compare every computed dark/light token with the normative
contract, verify Tailwind utilities and forced theme subtrees, persist an
override across reload, follow emulated system changes, and handle denied
storage. Unit tests cover native subscription rejection and late cleanup under
StrictMode. System appearance settings were not changed.

Popover checks cover native outside dismissal, Escape, repeated reopening,
trigger focus return, light colors in the top layer, and a 14px anchor offset
that survives scrolling. Modal checks cover Tab/Shift+Tab containment, rejected
background focus, Escape and explicit close, light-theme inheritance, and
invoker focus return. Listener registration/cleanup is balanced under
StrictMode; the wrappers add no global Escape/outside-click handlers.

## Evidence

- [Dark appearance](FND-05/dark.png)
- [Light appearance](FND-05/light.png)
- [Light popover on dark page](FND-05/popover-light-subtree.png)
- [Light modal on dark page](FND-05/modal-light-subtree.png)
- [Font request log](FND-05/offline-font-network.json): all eight weights loaded
  from local `/fonts/` assets with external requests blocked; after network was
  disabled, the page still changed theme and used both fonts.

The real CLI negative check temporarily added a scratch component containing a
raw hex value. `pnpm --dir apps/desktop/ui lint:hex` returned exit 1 and:

```text
hex-lint-acceptance-scratch.tsx:1:25: raw hex color; use a design token
```

The scratch file was removed. The Node regression also checks the exact token
allowlist, ordinary test-file exemption, and rejection of a misleading
`kit/tokens.css` filename.

## Limits

These are synthetic browser captures, not native desktop screenshots. Native
Tauri theme callbacks are covered through adapter tests; no real system
appearance switch was made. The run uses current Playwright WebKit 26.6, not an
installed Safari 17.0 build. [Native popover support starts in Safari 17.0](https://webkit.org/blog/14445/webkit-features-in-safari-17-0/#popover), and
the implementation avoids later CSS-anchor, `closedby`, and popover-source
APIs. Release qualification on the macOS 14 floor remains a native release gate.

The token reference is the reconciled FND-05 technical contract; the original
`xt-theme.css` export was unavailable. This evidence does not claim parity
against a recovered design export. Full font coverage makes the bundled font
files about 1 MB, above the plan's approximate 600 KB estimate.
