import { cleanup, render } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Modal } from './Modal';

beforeEach(() => {
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
    configurable: true,
    value: vi.fn(function (this: HTMLDialogElement) {
      this.open = true;
    }),
  });
  Object.defineProperty(HTMLDialogElement.prototype, 'close', {
    configurable: true,
    value: vi.fn(function (this: HTMLDialogElement) {
      this.open = false;
      this.dispatchEvent(new Event('close'));
    }),
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it('ignores a queued close from a previous opening and restores the invoker on unmount', () => {
  const trigger = document.createElement('button');
  document.body.append(trigger);
  trigger.focus();
  const changed = vi.fn();
  const view = render(
    <StrictMode>
      <Modal open onOpenChange={changed} aria-label="Details">
        <button>Close</button>
      </Modal>
    </StrictMode>,
  );
  const dialog = view.getByRole('dialog') as HTMLDialogElement;
  changed.mockClear();
  dialog.dispatchEvent(new Event('close'));
  expect(changed).not.toHaveBeenCalled();
  view.getByText('Close').focus();
  view.unmount();
  expect(document.activeElement).toBe(trigger);
  expect(dialog.open).toBe(false);
  trigger.remove();
});
