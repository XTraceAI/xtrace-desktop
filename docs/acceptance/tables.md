# Tables, host filters and activity acceptance

Issue #20 adds reusable presentation components. The live synthetic preview is `e2e/table-overflow.html` on the UI development server. It is separate from the installed application's screens.

| Case           | Action                                                                                               | Expected result                                                                                                                                                                                                                                                                        |
| -------------- | ---------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Table layout   | Render eight-column and six-column tables, all five row heights, zero and missing usage              | Headers and cells share their grid; heights are 22/24/32/40/44px; numeric cells align right; missing usage has an em dash and accessible reason.                                                                                                                                       |
| Sorting        | Click a sortable header twice                                                                        | Controlled direction and `aria-sort` update; the preview sorts supplied rows and keeps missing observations last. The component itself does not reorder data.                                                                                                                          |
| Row actions    | Click text, press Enter/Space on its named action button, activate a nested button or checkbox label | The button is discoverable by role/name, Enter/Space each call back once, and static rows are not Tab stops. The exact row reaches the callback; nested actions retain their behavior without opening the row, including buttons and noninteractive text inside a portaled cell popup. |
| Expansion      | Expand, collapse and sort keyed rows                                                                 | A native disclosure button reports its state and controls a details row in the same bordered group. Expansion follows the row key.                                                                                                                                                     |
| Scrolling      | Scroll a wide table horizontally and vertically                                                      | Header and numeric cells remain aligned; the sticky header stays visible; hidden accessibility text stays inside the scroll area.                                                                                                                                                      |
| Narrow preview | Open at 360px, then switch both themes and open the filter                                           | Tables scroll internally without page overflow; the popover stays inside the viewport and receives the active palette.                                                                                                                                                                 |
| Host filter    | Open by keyboard, press Space, Tab, Escape, then reopen and click outside                            | Native checkboxes update controlled selection. Escape restores trigger focus. Outside interaction dismisses the popover. Unknown hosts and counts remain distinct from known zero.                                                                                                     |
| Microcharts    | Supply zero, missing, invalid, maximum and out-of-range observations                                 | Zero is a visible 2px mark at 25% opacity; unknown is dashed; finite bars clamp safely. All 14 day labels and threshold boundaries are correct.                                                                                                                                        |
| Empty/loading  | Render no rows or loading data; click Detect again                                                   | Named empty/loading states replace data, and the supplied action runs. Skeletons are static.                                                                                                                                                                                           |

## Automated verification

```sh
pnpm check
pnpm e2e
pnpm build
pnpm check:native --base <reviewed-main-commit>
```

Type, lint, formatting, one Node lint regression, all 61 UI unit tests (10 focused on these components), all 42 Chromium/WebKit browser cases (8 focused here), and the production build pass. Browser tests run with Playwright 1.58.2, local fonts, synthetic data, both palettes and 1120px/360px viewports. Tests write evidence to their own result directories; reviewed WebKit captures are copied below.

The local native check covers Rust formatting, Clippy, tests, dependency notices/SBOM, secret scanning, the debug bundle and launch. The PR records its exact candidate, baseline and result. Native release-floor compatibility, signing and notarization are separate release checkpoints. No hosted macOS run is added by this change.

## Visual review

- [Dark tables and states](tables/table-dark.png), [light tables and states](tables/table-light.png)
- [Dark host filter](tables/filter-dark.png), [light host filter](tables/filter-light.png)
- [Dark design fragments](tables/design-dark.png), [light design fragments](tables/design-light.png)

The independent references render supplied design fragments for PR and Rulebook grids, the Sessions host menu, and small activity/day charts with synthetic values. They use the source styles rather than the new component CSS. The raw design export and its original sample content are not included.

Shared dimensions, typography, palettes, menu spacing and chart geometry were compared. The reusable table standardizes a 12px gap (the PR reference uses 10px), adds explicit keyboard-accessible disclosure and row-action controls, and lets screens supply their own column widths and content. The preview deliberately includes extra controls and stress states; it is not a reconstruction of a complete screen. The implementation also retains the approved light backing for Cursor's icon and makes unknown chart values visibly distinct, unlike the older reference.

Full screen composition, real inventory, metric calculation, pagination and large interactive charts remain later integrations.
