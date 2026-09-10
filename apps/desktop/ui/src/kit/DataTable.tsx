import { useId, type CSSProperties, type ReactNode } from 'react';
import { MetricCell, type MetricCellProps } from './MetricCell';
import { EmptyState, LoadingRows } from './EmptyState';
import '../styles/tables.css';

export interface Column<Row> {
  key: string;
  header: ReactNode;
  width: string;
  align?: 'left' | 'right';
  render: (row: Row) => ReactNode;
  sortable?: boolean;
}
export interface SortOrder {
  key: string;
  direction: 'asc' | 'desc';
}
export interface DataTableProps<Row> {
  label: string;
  columns: readonly Column<Row>[];
  rows: readonly Row[];
  getRowKey: (row: Row) => string;
  rowHeight?: 22 | 24 | 32 | 40 | 44;
  onRowClick?: (row: Row) => void;
  getRowActionLabel?: (row: Row) => string;
  hover?: 'subtle' | 'track';
  rowOpacity?: (row: Row) => number;
  sort?: SortOrder;
  onSort?: (sort: SortOrder) => void;
  expandedKeys?: readonly string[];
  onExpandedChange?: (keys: string[]) => void;
  renderExpanded?: (row: Row) => ReactNode;
  stickyHeader?: boolean;
  maxHeight?: CSSProperties['maxHeight'];
  minWidth?: CSSProperties['minWidth'];
  loading?: boolean;
  emptyMessage?: string;
}

const interactive = (target: EventTarget | null, row: HTMLElement) => {
  if (!(target instanceof Element) || target === row) return false;
  const control = target.closest(
    'button, a, input, select, textarea, label, summary, [role="button"], [role="checkbox"], [role="switch"], [role="radio"], [contenteditable="true"], [tabindex]',
  );
  return control !== null && control !== row && row.contains(control);
};

export function DataTable<Row>({
  label,
  columns,
  rows,
  getRowKey,
  rowHeight = 40,
  onRowClick,
  getRowActionLabel,
  hover = 'subtle',
  rowOpacity,
  sort,
  onSort,
  expandedKeys = [],
  onExpandedChange,
  renderExpanded,
  stickyHeader = false,
  maxHeight,
  minWidth = '100%',
  loading = false,
  emptyMessage = 'No rows in this window',
}: DataTableProps<Row>) {
  const id = useId();
  const expandable = !!renderExpanded;
  const template = [
    ...(expandable ? ['28px'] : []),
    ...columns.map((column) => column.width),
    ...(onRowClick ? ['28px'] : []),
  ].join(' ');
  const totalColumns = columns.length + Number(expandable) + Number(!!onRowClick);
  return (
    <div
      className="xt-table-scroll"
      style={{ maxHeight }}
      tabIndex={0}
      role="region"
      aria-label={`${label} scroll area`}
    >
      <div
        className="xt-data-table"
        role="table"
        aria-label={label}
        aria-busy={loading}
        style={{ minWidth, '--table-columns': template } as CSSProperties}
      >
        <div role="rowgroup" className="xt-table-head" data-sticky={stickyHeader || undefined}>
          <div role="row" className="xt-grid-row">
            {expandable && (
              <div role="columnheader">
                <span className="sr-only">Details</span>
              </div>
            )}
            {columns.map((column) => (
              <div
                key={column.key}
                role="columnheader"
                style={{ textAlign: column.align }}
                aria-sort={
                  sort?.key === column.key
                    ? sort.direction === 'asc'
                      ? 'ascending'
                      : 'descending'
                    : undefined
                }
              >
                {column.sortable && onSort ? (
                  <button
                    type="button"
                    tabIndex={0}
                    onClick={() =>
                      onSort({
                        key: column.key,
                        direction:
                          sort?.key === column.key && sort.direction === 'asc' ? 'desc' : 'asc',
                      })
                    }
                  >
                    {column.header}
                    {sort?.key === column.key && (
                      <span aria-hidden="true"> {sort.direction === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </button>
                ) : (
                  column.header
                )}
              </div>
            ))}
            {onRowClick && (
              <div role="columnheader">
                <span className="sr-only">Open row</span>
              </div>
            )}
          </div>
        </div>
        {loading ? (
          <div role="rowgroup">
            <div role="row">
              <div role="cell" aria-colspan={totalColumns}>
                <LoadingRows />
              </div>
            </div>
          </div>
        ) : rows.length === 0 ? (
          <div role="rowgroup">
            <div role="row">
              <div role="cell" aria-colspan={totalColumns}>
                <EmptyState message={emptyMessage} />
              </div>
            </div>
          </div>
        ) : (
          rows.map((row) => {
            const key = getRowKey(row);
            const expanded = expandedKeys.includes(key);
            const detailsId = `${id}-${encodeURIComponent(key)}`;
            const opacity = rowOpacity?.(row);
            return (
              <div role="rowgroup" className="xt-table-block" key={key}>
                <div
                  role="row"
                  className="xt-grid-row xt-data-row"
                  data-hover={hover}
                  style={{
                    height: rowHeight,
                    opacity:
                      opacity === undefined || !Number.isFinite(opacity)
                        ? 1
                        : Math.max(0, Math.min(1, opacity)),
                  }}
                  data-actionable={!!onRowClick || undefined}
                  onClick={(event) => {
                    if (!interactive(event.target, event.currentTarget)) onRowClick?.(row);
                  }}
                >
                  {expandable && (
                    <div role="cell">
                      <button
                        className="xt-expand-row"
                        type="button"
                        tabIndex={0}
                        aria-label={`${expanded ? 'Collapse' : 'Expand'} ${key}`}
                        aria-expanded={expanded}
                        aria-controls={expanded ? detailsId : undefined}
                        disabled={!onExpandedChange}
                        onClick={() =>
                          onExpandedChange?.(
                            expanded
                              ? expandedKeys.filter((item) => item !== key)
                              : [...expandedKeys, key],
                          )
                        }
                      >
                        <span aria-hidden="true">{expanded ? '⌄' : '›'}</span>
                      </button>
                    </div>
                  )}
                  {columns.map((column) => (
                    <div key={column.key} role="cell" style={{ textAlign: column.align }}>
                      {column.render(row)}
                    </div>
                  ))}
                  {onRowClick && (
                    <div role="cell">
                      <button
                        className="xt-row-action"
                        type="button"
                        tabIndex={0}
                        aria-label={getRowActionLabel?.(row) ?? `Open row ${key}`}
                        onClick={() => onRowClick(row)}
                      >
                        <span aria-hidden="true">↗</span>
                      </button>
                    </div>
                  )}
                </div>
                {expanded && renderExpanded && (
                  <div role="row" id={detailsId} className="xt-expanded-row">
                    <div role="cell" aria-colspan={totalColumns}>
                      {renderExpanded(row)}
                    </div>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

export function TitleCell({ title, subline }: { title: string; subline?: string }) {
  return (
    <span className="xt-title-cell">
      <span title={title}>{title}</span>
      {subline && <small title={subline}>{subline}</small>}
    </span>
  );
}
export function NumCell(props: MetricCellProps) {
  return <MetricCell {...props} align="right" />;
}
export function MonoCell({ children, muted = false }: { children: ReactNode; muted?: boolean }) {
  return (
    <span className="xt-mono-cell" data-muted={muted || undefined}>
      {children}
    </span>
  );
}
