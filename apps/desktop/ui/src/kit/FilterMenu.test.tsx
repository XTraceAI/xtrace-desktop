import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { FilterMenu } from './FilterMenu';
vi.mock('./Popover', () => ({
  Popover: ({ open, children }: { open: boolean; children: ReactNode }) =>
    open ? <div>{children}</div> : null,
}));
afterEach(cleanup);
it('emits controlled selection without losing unknown hosts and keeps absent counts distinct from zero', () => {
  const onChange = vi.fn();
  const options = [
    { id: 'claude', label: 'Claude', count: 0 },
    { id: 'future', label: 'Future host', count: null },
  ];
  const view = render(<FilterMenu options={options} selected={['future']} onChange={onChange} />);
  expect(screen.getByTitle('Unknown host: future')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Filter hosts' }));
  expect(screen.getByText('0')).toBeTruthy();
  expect(screen.getByText('—')).toBeTruthy();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Claude 0' }));
  expect(onChange).toHaveBeenLastCalledWith(['future', 'claude']);
  expect((screen.getByRole('checkbox', { name: 'Claude 0' }) as HTMLInputElement).checked).toBe(
    false,
  );
  view.rerender(
    <FilterMenu options={options} selected={['future', 'claude']} onChange={onChange} />,
  );
  fireEvent.click(screen.getByRole('checkbox', { name: 'Future host —' }));
  expect(onChange).toHaveBeenLastCalledWith(['claude']);
});
