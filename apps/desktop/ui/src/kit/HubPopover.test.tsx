import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { createPopoverHandle } from './Popover';
import { afterEach, expect, it, vi } from 'vitest';
import { HubPopover } from './HubPopover';
afterEach(cleanup);
const base = { id: 'hub', open: true, onOpenChange: vi.fn(), handle: createPopoverHandle() };

it('keeps connection disabled until wired and only calls the supplied action', () => {
  const view = render(<HubPopover {...base} />);
  expect(
    (screen.getByRole('button', { name: 'Connect XTrace Hub' }) as HTMLButtonElement).disabled,
  ).toBe(true);
  const connect = vi.fn();
  view.rerender(<HubPopover {...base} onConnect={connect} />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect XTrace Hub' }));
  expect(connect).toHaveBeenCalledOnce();
});

it('uses supplied connection/team state and delegates dismissal to the shared close control', () => {
  render(<HubPopover {...base} connected teamLabel="Example team" />);
  expect(screen.getByText('Connected to Example team.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Connect XTrace Hub' })).toBeNull();
  const close = screen.getByRole('button', { name: 'Close XTrace Hub' });
  fireEvent.click(close);
  expect(base.onOpenChange).toHaveBeenCalledWith(
    false,
    expect.objectContaining({ reason: 'close-press' }),
  );
});
