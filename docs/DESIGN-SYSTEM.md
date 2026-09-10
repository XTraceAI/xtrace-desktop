# Design foundation

The application uses the color and style contract in `design/token-contract.json`.
`styles/tokens.css` supplies equal dark/light key sets; `styles/theme.css` uses
Tailwind v4 `@theme inline` so utilities resolve the closest theme variables.
Use `bg-surface`, `text-ink`, `border-border`, `font-mono`, `rounded-card`, or
`var(--token)` in component CSS. Raw hex is rejected everywhere in `src/` except
the token file and `*.test.*`. The contract lists the values shared by components and tests.

## Appearance

Mount one `ThemeProvider` at the application root. `useTheme()` returns:

```ts
{
  theme: 'dark' | 'light';
  preference: 'system' | 'dark' | 'light';
  setPreference: (preference: ThemePreference) => void;
  toggle: () => void;
}
```

`system` is the initial preference; unavailable media support falls back to dark.
Explicit choices persist under `xt.theme`. Storage failures leave the current
choice usable for the session. Webview `prefers-color-scheme` is authoritative;
Tauri `theme()` and `onThemeChanged` request a fresh media read. Subscriptions
clean up even if native registration resolves after unmount.

`<ThemeScope theme="dark">` or `theme="light"` sets a subtree's theme. Apply token
utilities or CSS to the content. The scope also supplies React context so
portalled popovers and dialogs retain that palette, including live theme changes.

## Component behavior

Use [Base UI](https://base-ui.com/react/overview/about) directly for shared
interaction primitives. Keep XTrace layout, typography, colors and domain views
in the existing kit. Base UI owns positioning, keyboard navigation, dismissal
and focus management; consumers must not add competing document listeners.

[Shadcn's Base UI components](https://ui.shadcn.com/docs/components/base/popover)
use the same underlying primitives and can be restyled. Direct Base UI avoids
maintaining an additional set of styled wrappers while fitting our compact
custom layouts. This choice does not prescribe a table engine or chart library.
Adopt other primitives when a consumer needs them; do not prebuild a catalog.

## Popovers

Create a stable `createPopoverHandle()` with `useState`. Give the same handle to
`PopoverTrigger` and controlled `Popover`; the library associates their ARIA
attributes and focus-return target. Use `PopoverClose` for the visible close action.

```tsx
const [handle] = useState(createPopoverHandle);
const [open, setOpen] = useState(false);
<>
  <PopoverTrigger handle={handle}>Details</PopoverTrigger>
  <Popover id="details" handle={handle} open={open} onOpenChange={setOpen} aria-label="Details">
    <PopoverClose>Close</PopoverClose>
  </Popover>
</>;
```

`Popover` accepts standard div props, `side="bottom" | "right"`,
`align="start" | "end"`, and a pixel `offset` (default 8). Base UI uses fixed
positioning, an 8px collision margin, and tracks scroll/resize. Panels portal to
the document body to escape clipped and transformed ancestors. Keyboard opening
focuses the first action; Escape returns focus to the registered trigger.
Outside clicks dismiss without replaying an opening on unrelated state updates.

For a separate geometry target, pass `positionAnchor`, an element held in state
by a callback ref. Passing the element makes replacement reactive; do not cache a
DOM node from an object ref. The registered trigger remains the focus-return
target. Mount/unmount the trigger and its popup owner together.

## Modals

`Modal` accepts controlled `open`/`onOpenChange`, optional `returnFocusRef`, and
standard div props, including an accessible `aria-label` or `aria-labelledby`.
Supply a visible `ModalClose`. Pass the invoker's ref because a mouse click on a
macOS button may not focus it; without it the library uses prior focus.

Base UI Dialog owns focus containment, background interaction blocking, Escape,
and nested-dialog behavior. Pointer clicks on the backdrop do not dismiss this
wrapper. There is no application Tab-key loop. A nested dialog goes inside its
parent's React tree so Escape closes the child first and focus returns through
each layer. The shared portal uses the closest ThemeScope or the app theme.

## Fonts and evidence

All font files are local under `public/fonts`, with licenses, source hashes, and
reproduction instructions in its README. The application bundles static WOFF2;
the matching TTF weights are available for the future SVG renderer. The old
Fontsource runtime imports are removed. `assetsInlineLimit: 0` remains in Vite
so the native `font-src 'self'` policy stays valid.

The browser-only overlay fixture lives under `e2e/` and is not included in the
production bundle. Run `pnpm --dir apps/desktop/ui exec playwright test
e2e/overlays.spec.ts --project=webkit` for keyboard, themes, and font-network
evidence. See `docs/acceptance/appearance.md` for the measured results and limits.
