import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { UpdateNotice, UpdateNoticeView } from './UpdateNotice';
afterEach(cleanup);

it('keeps browser/development notices hidden and performs no update request', async () => {
  const { unmount } = render(<UpdateNotice />);
  expect(screen.queryByRole('button', { name: 'Restart to update' })).toBeNull();
  unmount();
});

it('offers an accessible explicit restart only when ready', () => {
  const restart = vi.fn();
  render(
    <UpdateNoticeView
      state={{ phase: 'ready', version: '0.2.0' }}
      onRestart={restart}
      onRetry={vi.fn()}
    />,
  );
  expect(screen.getByRole('status').textContent).toContain('Your local data stays');
  expect(restart).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Restart to update' }));
  expect(restart).toHaveBeenCalledTimes(1);
});

it.each(['idle', 'disabled'] as const)('renders nothing when %s', (phase) => {
  const { container } = render(
    <UpdateNoticeView state={{ phase }} onRestart={vi.fn()} onRetry={vi.fn()} />,
  );
  expect(container.childElementCount).toBe(0);
});

it('keeps the ready action compact while announcing the version and retained data', () => {
  render(
    <UpdateNoticeView
      state={{ phase: 'ready', version: '0.2.0' }}
      onRestart={vi.fn()}
      onRetry={vi.fn()}
    />,
  );
  const status = screen.getByRole('status');
  expect(status.textContent).toContain('Update 0.2.0 is ready. Your local data stays on this Mac.');
  expect(status.querySelector('.sr-only')?.textContent).toContain('Update 0.2.0 is ready');
  expect(status.className).not.toContain('xt-shell-notice');
});

it('shows accessible determinate and unknown-length progress without install controls', () => {
  const { rerender } = render(
    <UpdateNoticeView
      state={{ phase: 'downloading', version: '0.2.0', downloaded: 50, total: 100 }}
      onRestart={vi.fn()}
      onRetry={vi.fn()}
    />,
  );
  const progress = screen.getByRole('progressbar', { name: 'Update download' });
  expect(progress.getAttribute('value')).toBe('50');
  expect(progress.getAttribute('max')).toBe('100');
  expect(screen.queryByRole('button')).toBeNull();
  rerender(
    <UpdateNoticeView
      state={{ phase: 'downloading', version: '0.2.0', downloaded: 50 }}
      onRestart={vi.fn()}
      onRetry={vi.fn()}
    />,
  );
  expect(progress.hasAttribute('value')).toBe(false);
});

it('announces errors with an accessible retry button', () => {
  const retry = vi.fn();
  render(
    <UpdateNoticeView
      state={{
        phase: 'error',
        message: 'Update installation failed. Retry installation.',
        retry: 'install',
      }}
      onRestart={vi.fn()}
      onRetry={retry}
    />,
  );
  expect(screen.getByRole('alert').textContent).toContain('installation failed');
  fireEvent.click(screen.getByRole('button', { name: 'Retry update' }));
  expect(retry).toHaveBeenCalledTimes(1);
});

it('has no actionable control during installation', () => {
  render(
    <UpdateNoticeView
      state={{ phase: 'installing', version: '0.2.0' }}
      onRestart={vi.fn()}
      onRetry={vi.fn()}
    />,
  );
  expect(screen.getByRole('status').textContent).toContain('Installing update');
  expect(screen.queryByRole('button')).toBeNull();
});
