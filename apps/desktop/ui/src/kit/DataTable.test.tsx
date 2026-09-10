import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { createPortal } from 'react-dom';
import { DataTable, TitleCell, NumCell, type Column } from './DataTable';
afterEach(cleanup);
const rows = [
  { id: 'one', name: 'First session', tokens: null },
  { id: 'two', name: 'Second session', tokens: 0 },
];
const columns: Column<(typeof rows)[number]>[] = [
  {
    key: 'name',
    header: 'Session',
    width: 'minmax(0,1fr)',
    render: (r) => <TitleCell title={r.name} subline={r.id} />,
    sortable: true,
  },
  {
    key: 'tokens',
    header: 'Tokens',
    width: '64px',
    align: 'right',
    render: (r) => <NumCell value={r.tokens} reason="Usage absent" />,
    sortable: true,
  },
];
const key = (r: (typeof rows)[number]) => r.id;
it('renders a typed table with aligned numeric headers, unknown versus zero and all row heights', () => {
  const view = render(<DataTable label="Sessions" columns={columns} rows={rows} getRowKey={key} />);
  expect(screen.getByRole('columnheader', { name: 'Tokens' }).style.textAlign).toBe('right');
  expect(screen.getByRole('table').style.getPropertyValue('--table-columns')).toBe(
    'minmax(0,1fr) 64px',
  );
  expect(screen.getByText('Unmeasured: Usage absent')).toBeTruthy();
  expect(screen.getByText('0')).toBeTruthy();
  for (const height of [22, 24, 32, 40, 44] as const) {
    view.rerender(
      <DataTable
        label="Sessions"
        columns={columns}
        rows={rows}
        getRowKey={key}
        rowHeight={height}
      />,
    );
    expect(screen.getAllByRole('row')[1].style.height).toBe(`${height}px`);
  }
});
it('emits actual row through text and a named native action while nested actions and expansion remain independent', () => {
  const onRowClick = vi.fn(),
    action = vi.fn(),
    expand = vi.fn();
  const cols = [
    ...columns,
    {
      key: 'action',
      header: 'Action',
      width: '64px',
      render: () => <button onClick={action}>Open</button>,
    },
  ];
  const view = render(
    <DataTable
      label="Sessions"
      columns={cols}
      rows={[rows[0]]}
      getRowKey={key}
      onRowClick={onRowClick}
      getRowActionLabel={(row) => `Open ${row.name}`}
      renderExpanded={(r) => <p>Details for {r.name}</p>}
      expandedKeys={[]}
      onExpandedChange={expand}
    />,
  );
  fireEvent.click(screen.getByText('First session'));
  expect(onRowClick).toHaveBeenLastCalledWith(rows[0]);
  const row = screen.getAllByRole('row')[1];
  expect(row.hasAttribute('tabindex')).toBe(false);
  const primaryAction = screen.getByRole('button', { name: 'Open First session' });
  expect(primaryAction.closest('[role="cell"]')).toBeTruthy();
  fireEvent.click(primaryAction);
  expect(onRowClick).toHaveBeenCalledTimes(2);
  expect(onRowClick).toHaveBeenLastCalledWith(rows[0]);
  fireEvent.click(screen.getByRole('button', { name: 'Open' }));
  fireEvent.click(screen.getByRole('button', { name: 'Expand one' }));
  expect(action).toHaveBeenCalledOnce();
  expect(onRowClick).toHaveBeenCalledTimes(2);
  expect(expand).toHaveBeenCalledExactlyOnceWith(['one']);
  expect(screen.queryByText('Details for First session')).toBeNull();
  view.rerender(
    <DataTable
      label="Sessions"
      columns={columns}
      rows={[rows[0]]}
      getRowKey={key}
      renderExpanded={(r) => <p>Details for {r.name}</p>}
      expandedKeys={['one']}
      onExpandedChange={expand}
    />,
  );
  const button = screen.getByRole('button', { name: 'Collapse one' });
  const details = document.getElementById(button.getAttribute('aria-controls')!)!;
  expect(within(details).getByRole('cell').getAttribute('aria-colspan')).toBe('3');
  expect(details.parentElement).toBe(button.closest('[role="rowgroup"]'));
  fireEvent.click(button);
  expect(expand).toHaveBeenLastCalledWith([]);
});
it('sort is controlled and toggles the clicked column without sorting source rows', () => {
  const onSort = vi.fn();
  const view = render(
    <DataTable label="Sessions" columns={columns} rows={rows} getRowKey={key} onSort={onSort} />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Tokens' }));
  expect(onSort).toHaveBeenLastCalledWith({ key: 'tokens', direction: 'asc' });
  view.rerender(
    <DataTable
      label="Sessions"
      columns={columns}
      rows={rows}
      getRowKey={key}
      onSort={onSort}
      sort={{ key: 'tokens', direction: 'asc' }}
    />,
  );
  expect(screen.getByRole('columnheader', { name: 'Tokens' }).getAttribute('aria-sort')).toBe(
    'ascending',
  );
  fireEvent.click(screen.getByRole('button', { name: 'Tokens' }));
  expect(onSort).toHaveBeenLastCalledWith({ key: 'tokens', direction: 'desc' });
});
it('empty and busy states are explicit and mutually exclusive', () => {
  const view = render(
    <DataTable
      label="Sessions"
      columns={columns}
      rows={[]}
      getRowKey={key}
      emptyMessage="No sessions"
    />,
  );
  expect(screen.getByText('No sessions')).toBeTruthy();
  view.rerender(
    <DataTable label="Sessions" columns={columns} rows={rows} getRowKey={key} loading />,
  );
  expect(screen.getByRole('status', { name: 'Loading rows' })).toBeTruthy();
  expect(screen.getByRole('table').getAttribute('aria-busy')).toBe('true');
  expect(screen.queryByText('First session')).toBeNull();
  expect(screen.queryByText('No sessions')).toBeNull();
});

it('ignores React portal events while preserving popup actions and ordinary row clicks', () => {
  const onRowClick = vi.fn(),
    popupAction = vi.fn();
  render(
    <DataTable
      label="Portal table"
      rows={[rows[0]]}
      getRowKey={key}
      onRowClick={onRowClick}
      columns={[
        ...columns,
        {
          key: 'menu',
          header: 'Menu',
          width: '80px',
          render: () =>
            createPortal(
              <div>
                <button onClick={popupAction}>Popup action</button>
                <span>Popup help text</span>
              </div>,
              document.body,
            ),
        },
      ]}
    />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Popup action' }));
  fireEvent.click(screen.getByText('Popup help text'));
  expect(popupAction).toHaveBeenCalledOnce();
  expect(onRowClick).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText('First session'));
  expect(onRowClick).toHaveBeenCalledExactlyOnceWith(rows[0]);
});
