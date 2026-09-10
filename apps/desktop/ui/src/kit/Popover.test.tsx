import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { StrictMode, useState } from 'react';
import { afterEach, expect, it } from 'vitest';
import { createPopoverHandle, Popover, PopoverClose, PopoverTrigger } from './Popover';
import { ThemeScope, type Theme } from '../theme/ThemeProvider';

afterEach(cleanup);

function Fixture({ theme }: { theme: Theme }) {
  const [handle] = useState(createPopoverHandle);
  const [open, setOpen] = useState(false);
  return (
    <ThemeScope theme={theme}>
      <PopoverTrigger handle={handle}>Details</PopoverTrigger>
      <Popover handle={handle} id="details" open={open} onOpenChange={setOpen} aria-label="Details">
        <PopoverClose>Close details</PopoverClose>
      </Popover>
    </ThemeScope>
  );
}

it('associates its trigger, closes through the library, and carries live scope tokens through a portal', async () => {
  const view = render(
    <StrictMode>
      <Fixture theme="light" />
    </StrictMode>,
  );
  const trigger = screen.getByRole('button', { name: 'Details' });
  fireEvent.click(trigger);
  const popup = await screen.findByRole('dialog', { name: 'Details' });
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  expect(trigger.getAttribute('aria-controls')).toBe(popup.id);
  expect(view.container.contains(popup)).toBe(false);
  expect(popup.closest('[data-theme]')?.getAttribute('data-theme')).toBe('light');
  view.rerender(
    <StrictMode>
      <Fixture theme="dark" />
    </StrictMode>,
  );
  expect(popup.closest('[data-theme]')?.getAttribute('data-theme')).toBe('dark');
  fireEvent.click(screen.getByRole('button', { name: 'Close details' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(trigger);
  await screen.findByRole('dialog');
  view.unmount();
  expect(screen.queryByRole('dialog')).toBeNull();
});
