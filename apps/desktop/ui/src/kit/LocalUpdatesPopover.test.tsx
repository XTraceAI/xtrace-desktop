import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { Sidebar, type SidebarProps } from './Sidebar';
import type { LocalUpdateControls } from '../data/DataSource';

afterEach(cleanup);
const props: SidebarProps = {
  onNavigate: vi.fn(),
  listener: { status: 'unknown' },
  version: '0.1.0',
  theme: 'dark',
  onToggleTheme: vi.fn(),
};
function mount(controls?: LocalUpdateControls) {
  return render(
    <StrictMode>
      <Sidebar {...props} localUpdates={controls} />
    </StrictMode>,
  );
}
async function openUpdates() {
  fireEvent.click(screen.getByRole('button', { name: 'Updates' }));
  return screen.findByRole('dialog', { name: 'Updates' });
}
function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

it('puts Updates beside the version and makes no release call on startup or opening', async () => {
  const viewPublicReleases = vi.fn().mockResolvedValue(undefined);
  mount({ viewPublicReleases });
  const button = screen.getByRole('button', { name: 'Updates' });
  expect(within(button.parentElement!).getByText('v0.1.0')).toBeTruthy();
  expect(viewPublicReleases).not.toHaveBeenCalled();
  const popup = await openUpdates();
  expect(
    within(popup).getByText(
      'This is a local development build. Public releases may omit local changes. Installing public updates from this app is disabled.',
    ),
  ).toBeTruthy();
  expect(viewPublicReleases).not.toHaveBeenCalled();
});

it('closes on Escape and returns focus to Updates', async () => {
  mount({ viewPublicReleases: vi.fn() });
  const popup = await openUpdates();
  const close = within(popup).getByRole('button', { name: 'Close Updates' });
  close.focus();
  fireEvent.keyDown(close, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Updates' })).toBeNull());
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Updates' })),
  );
});

it('keeps Updates, capture, and Cloud and Team mutually exclusive in both directions', async () => {
  mount({ viewPublicReleases: vi.fn() });
  await openUpdates();
  fireEvent.click(screen.getByRole('button', { name: 'Capture status' }));
  await screen.findByRole('dialog', { name: 'Capture by surface' });
  await waitFor(() => expect(screen.getAllByRole('dialog')).toHaveLength(1));
  await openUpdates();
  await waitFor(() => expect(screen.getAllByRole('dialog')).toHaveLength(1));
  fireEvent.click(screen.getByRole('button', { name: 'XTrace Hub' }));
  await screen.findByRole('dialog', { name: 'Connect this desktop to your team.' });
  await waitFor(() => expect(screen.getAllByRole('dialog')).toHaveLength(1));
  await openUpdates();
  await waitFor(() => expect(screen.getAllByRole('dialog')).toHaveLength(1));
});

it('invokes once, disables the pending action across closing and reopening, and enables after success', async () => {
  const request = deferred();
  const viewPublicReleases = vi.fn(() => request.promise);
  mount({ viewPublicReleases });
  await openUpdates();
  const action = screen.getByRole('button', { name: 'View public releases' }) as HTMLButtonElement;
  fireEvent.click(action);
  fireEvent.click(action);
  expect(viewPublicReleases).toHaveBeenCalledExactlyOnceWith();
  expect(action.disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Close Updates' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Updates' })).toBeNull());
  await openUpdates();
  expect(
    (screen.getByRole('button', { name: 'View public releases' }) as HTMLButtonElement).disabled,
  ).toBe(true);
  await act(async () => request.resolve());
  expect(
    (screen.getByRole('button', { name: 'View public releases' }) as HTMLButtonElement).disabled,
  ).toBe(false);
  expect(screen.queryByText('Opening your browser…')).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
});

it('shows browser failure, clears it on retry, and stays disabled until the retry completes', async () => {
  const retry = deferred();
  const viewPublicReleases = vi
    .fn()
    .mockRejectedValueOnce(new Error('private launcher detail'))
    .mockImplementationOnce(() => retry.promise);
  mount({ viewPublicReleases });
  await openUpdates();
  fireEvent.click(screen.getByRole('button', { name: 'View public releases' }));
  expect(await screen.findByRole('alert')).toHaveProperty(
    'textContent',
    'Public releases could not be opened in your browser. Try again.',
  );
  fireEvent.click(screen.getByRole('button', { name: 'View public releases' }));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(
    (screen.getByRole('button', { name: 'View public releases' }) as HTMLButtonElement).disabled,
  ).toBe(true);
  await act(async () => retry.resolve());
  expect(screen.queryByRole('alert')).toBeNull();
  expect(viewPublicReleases).toHaveBeenCalledTimes(2);
});

it('omits Updates without the native capability', () => {
  mount();
  expect(screen.queryByRole('button', { name: 'Updates' })).toBeNull();
});

it('ignores an old request after the capability is replaced and returned', async () => {
  const old = deferred();
  const original = { viewPublicReleases: vi.fn(() => old.promise) };
  const replacement = { viewPublicReleases: vi.fn().mockResolvedValue(undefined) };
  const view = mount(original);
  await openUpdates();
  fireEvent.click(screen.getByRole('button', { name: 'View public releases' }));
  view.rerender(
    <StrictMode>
      <Sidebar {...props} localUpdates={replacement} />
    </StrictMode>,
  );
  view.rerender(
    <StrictMode>
      <Sidebar {...props} localUpdates={original} />
    </StrictMode>,
  );
  await act(async () => old.reject(new Error('old error')));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(
    (screen.getByRole('button', { name: 'View public releases' }) as HTMLButtonElement).disabled,
  ).toBe(false);
});

it('ignores completion after unmounting the control', async () => {
  const request = deferred();
  const view = mount({ viewPublicReleases: vi.fn(() => request.promise) });
  await openUpdates();
  fireEvent.click(screen.getByRole('button', { name: 'View public releases' }));
  view.unmount();
  await act(async () => request.reject(new Error('late browser failure')));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
});
