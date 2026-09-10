# Top bar and host logo acceptance

The kit provides a controlled top bar, shared host logos and decorative icons.
The sidebar consumes the shared host logo component. The production app shell
will adopt the top bar in its integration work.

## Expected behavior

| Case               | Setup and action                                                                           | Expected result                                                                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controlled presets | Render 7d, select 30d, then rerender with 30d.                                             | One preset callback; selection stays 7d until the caller updates it.                                                                                        |
| Keyboard           | Arrow across presets and wrap, then Tab through enabled controls in Chromium and WebKit.   | One preset Tab stop, arrow selection, calendar then action; disabled calendar skipped.                                                                      |
| Custom dates       | Open the picker, confirm from the caller, reopen, then select a preset.                    | Opening preserves current selection. Only confirmation displays custom as selected. Reopening works; custom never reaches the preset callback.              |
| Actions            | Activate share, scan and copy; render unwired/blank actions and hidden range.              | Each supplied action dispatches once; unwired controls are disabled and omitted controls leave no focusable ghosts.                                         |
| Host identity      | Render known, missing and unfamiliar hosts, including object-property names.               | Known logos use the existing bundled assets and accessible names; unknown remains `?`, never another host's identity.                                       |
| Geometry           | Render all four sizes, stacked logos, both themes and existing sidebar.                    | 16/18/20/30px squares; 4px overlap; Cursor retains a white backing. Logos load and preserve aspect ratio. Sidebar geometry and interactions pass unchanged. |
| Minimum width      | Render 1200×44 and long breadcrumbs at a 1120px viewport, reserving 228px for the sidebar. | 892×44 content area; breadcrumbs truncate, controls remain in bounds, no horizontal page overflow.                                                          |

## Verification

- `pnpm check`: typecheck, lint, format, component and token tests.
- `pnpm e2e`: Chromium and WebKit, including existing sidebar and overlay coverage.
- `pnpm build`: production build targeting Safari 17.
- `pnpm check:native --base <base-sha>`: clean committed-source build and main-window smoke on local macOS. Exact source/base identities and results are recorded in the PR.

## Visual evidence

| Theme | Component                          | Independent design reference      | Minimum frame                       | Host sizes and icons             |
| ----- | ---------------------------------- | --------------------------------- | ----------------------------------- | -------------------------------- |
| Dark  | [Top bar](topbar/topbar-dark.png)  | [Design](topbar/design-dark.png)  | [Minimum](topbar/minimum-dark.png)  | [Logos](topbar/glyphs-dark.png)  |
| Light | [Top bar](topbar/topbar-light.png) | [Design](topbar/design-light.png) | [Minimum](topbar/minimum-light.png) | [Logos](topbar/glyphs-light.png) |

All captures contain synthetic component data. References render the retrieved
TopBar template and its own theme CSS with the same synthetic breadcrumb/action
values and bundled font files; they do not import candidate component styles.
The blank strip reserves sidebar width, without depicting native controls.

The top bar follows the reference geometry and artwork. Semantic buttons,
focus rings, disabled feedback, confirmed custom selection and long-text
truncation add production behavior to the static design. The application's
reviewed contrast-adjusted token values remain authoritative; no claim of
pixel-identical colors or font rasterization is made.

## Limits

The kit performs no date calculation, metric recomputation, routing or network
request. The custom-date picker and primary action workflows belong to their
consuming pages. Browser fixtures prove kit interactions; the native check
proves the existing app launches from the same source. Downloadable releases
own macOS 14/Safari 17 floor qualification.

See [component API](../TOPBAR.md) for props and asset provenance.
