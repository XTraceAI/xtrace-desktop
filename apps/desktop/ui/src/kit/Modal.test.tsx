import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { StrictMode, useRef, useState } from 'react';
import { afterEach, expect, it } from 'vitest';
import { Modal, ModalClose } from './Modal';
import { ThemeScope } from '../theme/ThemeProvider';

afterEach(cleanup);

it('opens and closes under StrictMode, preserves scope tokens, and releases its portal on unmount', async () => {
  function Fixture() {
    const trigger = useRef<HTMLButtonElement>(null);
    const [open, setOpen] = useState(false);
    return (
      <ThemeScope theme="light">
        <button ref={trigger} onClick={() => setOpen(true)}>
          Details
        </button>
        <Modal open={open} onOpenChange={setOpen} returnFocusRef={trigger} aria-label="Details">
          <ModalClose>Close details</ModalClose>
        </Modal>
      </ThemeScope>
    );
  }
  const view = render(
    <StrictMode>
      <Fixture />
    </StrictMode>,
  );
  const trigger = screen.getByRole('button', { name: 'Details' });
  fireEvent.click(trigger);
  const popup = await screen.findByRole('dialog', { name: 'Details' });
  expect(popup.closest('[data-theme]')?.getAttribute('data-theme')).toBe('light');
  expect(view.container.contains(popup)).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Close details' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(trigger);
  await screen.findByRole('dialog');
  view.unmount();
  expect(screen.queryByRole('dialog')).toBeNull();
});
