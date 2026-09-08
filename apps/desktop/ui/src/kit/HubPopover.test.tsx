import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { HubPopover } from './HubPopover';
import type { PopoverProps } from './Popover';

// Native dismissal is exercised in WebKit; these tests own Hub content/actions.
vi.mock('./Popover', () => ({
  Popover: ({ open, children }: PopoverProps) => (open ? <div>{children}</div> : null),
}));
afterEach(cleanup);
const base = { id: 'hub', open: true, onOpenChange: vi.fn(), anchorRef: createRef<HTMLElement>() };

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

it('uses supplied connection/team state and delegates dismissal to the native target', () => {
  render(<HubPopover {...base} connected teamLabel="Example team" />);
  expect(screen.getByText('Connected to Example team.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Connect XTrace Hub' })).toBeNull();
  const close = screen.getByRole('button', { name: 'Close XTrace Hub' });
  expect(close.getAttribute('popovertarget')).toBe('hub');
  expect(close.getAttribute('popovertargetaction')).toBe('hide');
});
