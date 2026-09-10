# Tables, host filters and activity primitives

These components render supplied data. It does not fetch inventory, compute metrics or sort/paginate source rows.

`DataTable<Row>` requires an accessible `label`, typed `columns`, `rows` and `getRowKey`. Each column supplies a stable key, header, CSS grid width, optional alignment and a row renderer. Header and body share one template and 12px column gaps. Flexible columns truncate within the available width; pass `minWidth` for an intentionally wide table. Its focusable scroll region contains horizontal overflow. `maxHeight` bounds vertical scrolling and `stickyHeader` keeps the header inside that region.

Row heights are 22, 24, 32, 40 (default) or 44px. Use single-line renderers at compact heights. `hover` selects subtle or track color. Optional `rowOpacity(row)` is clamped to 0–1, with nonfinite values treated as opaque. The cell renderer must communicate why a row is excluded; opacity alone carries no accessible meaning.

`onRowClick(row)` receives pointer clicks and Enter/Space on a focused row. Native controls, their labels and explicitly interactive descendants retain their own action. For sorting, mark a column `sortable`, supply `sort: {key, direction}` and handle `onSort`; the table emits the next direction and exposes `aria-sort` without rearranging the supplied array.

Expansion uses `renderExpanded(row)`, controlled `expandedKeys` and `onExpandedChange`. The extra details column has a native button with `aria-expanded` and a live `aria-controls` relationship. Details render in the same bordered row group. Without an expansion callback, supplied expanded content remains visible and the control is disabled. `loading` replaces rows with a named status; an empty array shows `emptyMessage`.

`TitleCell` accepts a title and optional subline with independent ellipsis and native titles. `NumCell` uses the shared MetricCell unknown/zero contract and right alignment. `MonoCell` provides secondary or muted monospace text. `EmptyState` has a message and optional labelled action callback; `LoadingRows` renders bounded static skeletons without claiming measured data.

`FilterMenu` takes host options (`id`, `label`, optional count), `selected` IDs and `onChange`. Empty selection means all hosts. It preserves unknown host IDs and distinguishes missing counts from zero. The native checkbox group uses a registered shared Base UI Popover trigger and delegates outside/Escape dismissal to Popover, returns focus to its trigger, and supports ordinary WebKit Tab navigation. It has no host discovery logic or duplicate global dismissal listener.

`SparkBars` accepts labelled days with `agent` and optional `human` values, a maximum and height 36 or 44. `single` shows a fires series. Values clamp to the declared maximum; measured zero remains a 2px mark at 0.25 opacity. Missing, negative and nonfinite observations have a separate dashed unknown mark and accessible label. An invalid or nonpositive maximum produces minimum-height marks without division by zero; supply a valid positive maximum for meaningful relative heights.

`DayStrip` requires exactly 14 labelled day observations and positive increasing thresholds. Zero uses the cell token; positive values below the first threshold use 35% accent, below the second use 65%, and values at/above the second use full accent. Missing/nonfinite/negative observations stay visibly and accessibly unknown. Invalid array length or thresholds throw a controlled contract error rather than inventing days. `Legend` renders supplied labels, optional values and semantic tone swatches or dots.

All fixtures and captures in [acceptance evidence](acceptance/tables.md) are synthetic. Screen owners supply real data, filter semantics, pagination and metric eligibility.
