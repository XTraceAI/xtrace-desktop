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
utilities or CSS to the content. A native top-layer overlay remains in that DOM
subtree, so its theme survives opening and page preference changes.

## Popovers

`Popover` accepts `id`, controlled `open`/`onOpenChange`, `anchorRef`, standard div
props, `side="bottom" | "right"`, `align="start" | "end"`, and a pixel `offset`
(default 8). It clamps to an 8px viewport margin and recomputes on resize, scroll,
and size changes. This uses fixed positioning supported at the Safari 17 floor,
without newer CSS anchor positioning.

```tsx
const trigger = useRef<HTMLButtonElement>(null);
const [open, setOpen] = useState(false);
<>
  <button ref={trigger} popoverTarget="details" aria-expanded={open}>
    Details
  </button>
  <Popover id="details" anchorRef={trigger} open={open} onOpenChange={setOpen}>
    <button popoverTarget="details" popoverTargetAction="hide" tabIndex={0}>
      Close
    </button>
  </Popover>
</>;
```

Use a unique ID and an accessible label/role appropriate to the content. Native
`popover="auto"` owns Escape, light dismissal, and the top layer; consumers must
not add global Escape/outside-click listeners. Explicit `tabIndex={0}` on a
popover action makes it reachable with ordinary Tab in WebKit even when macOS
skips button focus. Dismissal returns focus to the trigger when focus would
otherwise be lost, while an outside focused control keeps focus.

## Modals

`Modal` accepts controlled `open`/`onOpenChange`, optional `returnFocusRef`, and
standard dialog props, including an accessible `aria-label` or `aria-labelledby`.
Supply a visible close action. Pass the invoker's ref because a mouse click on
a macOS button may not focus it. Without the ref, the prior focused element is
restored.

`showModal()` owns background inertness, Escape, and top-layer placement. A local
Tab handler cycles visible enabled controls because WebKit can otherwise send
Tab to browser chrome. There are no document-level keyboard or focus listeners.
Closing or unmounting restores focus if the invoker remains connected. These
primitives cover ordinary form controls; a consumer with a composite widget
owns that widget's arrow-key/roving-tabindex behavior.

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
