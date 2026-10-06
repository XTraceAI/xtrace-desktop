import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { check } from '@tauri-apps/plugin-updater';
import { afterEach, expect, it, vi } from 'vitest';
import { UpdateNotice, UpdateNoticeView } from './UpdateNotice';
afterEach(cleanup);
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }));

it('keeps an unselected updater hidden and performs no update request', async () => {
  const { unmount } = render(<UpdateNotice enabled={false} />);
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

it('uses the selected public mode with DEV=true, downloads, and installs only on click', async () => {
  vi.stubEnv('DEV', true);
  const resource = {
    version: '0.1.2',
    download: vi.fn().mockResolvedValue(undefined),
    install: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
  };
  vi.mocked(check).mockResolvedValue(resource as unknown as Awaited<ReturnType<typeof check>>);
  vi.mocked(invoke).mockResolvedValue(undefined);
  try {
    render(<UpdateNotice enabled />);
    const restart = await screen.findByRole('button', { name: 'Restart to update' });
    expect(check).toHaveBeenCalledTimes(1);
    expect(resource.download).toHaveBeenCalledTimes(1);
    expect(resource.install).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
    fireEvent.click(restart);
    await waitFor(() => expect(resource.install).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(invoke).toHaveBeenCalledExactlyOnceWith('restart_after_update'));
  } finally {
    window.dispatchEvent(new Event('pagehide'));
    vi.unstubAllEnvs();
  }
});
