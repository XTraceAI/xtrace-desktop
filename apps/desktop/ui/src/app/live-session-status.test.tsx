import { act, cleanup, render, renderHook, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { DataSource } from '../data/DataSource';
import type { LiveSessionSnapshot } from '../data/generated/LiveSessionSnapshot';
import { LiveSessionBadge } from './LiveSessionBadge';
import { isLiveSessionHost, liveSessionIds, useLiveSessionStatus } from './live-session-status';

let source: { liveSessions?: DataSource['liveSessions'] };
type ReadLive = NonNullable<DataSource['liveSessions']>['read'];
type ReleaseLive = NonNullable<DataSource['liveSessions']>['release'];
type Status = LiveSessionSnapshot['states'][number]['status'];
vi.mock('../data/DataProvider', () => ({ useData: () => ({ source }) }));

function pending() {
  let resolve!: (snapshot: LiveSessionSnapshot) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<LiveSessionSnapshot>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const empty = (view_id: string): LiveSessionSnapshot => ({ view_id, states: [] });
const snapshot = (view_id: string, status: Status, id = 'exact-chat'): LiveSessionSnapshot => ({
  view_id,
  states: [{ id, status }],
});
function fakeControls(status: Status = 'idle') {
  let issued = 0;
  const read = vi.fn<ReadLive>(async (ids, token) =>
    token === null
      ? empty('native-issued-' + ++issued)
      : { view_id: token, states: ids.map((id) => ({ id, status })) },
  );
  const release = vi.fn<ReleaseLive>(async () => {});
  source.liveSessions = { read, release };
  return { read, release };
}
const settle = () => act(async () => {});
const tick = () =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
beforeEach(() => {
  vi.useFakeTimers();
  source = {};
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it('registers an empty lease before subscribing, deduplicates IDs and caps the selected read at sixteen', async () => {
  const ids = Array.from({ length: 22 }, (_, index) => 'chat-' + index);
  const wanted = [ids[2], ids[0], ids[2], ...ids];
  expect(liveSessionIds(wanted)).toEqual([ids[2], ids[0], ids[1], ...ids.slice(3, 16)]);
  const { read } = fakeControls();
  const { result } = renderHook(() => useLiveSessionStatus('sessions', wanted));
  expect(read).toHaveBeenCalledTimes(1);
  expect(read.mock.calls[0]).toEqual([[], null]);
  await settle();
  expect(read).toHaveBeenCalledTimes(2);
  expect(read.mock.calls[1]).toEqual([liveSessionIds(wanted).sort(), 'native-issued-1']);
  expect(result.current(ids[21])).toBe('unknown');
});

it('polls Running → both waiting states → Idle, then clears Running on error', async () => {
  const { read, release } = fakeControls();
  read
    .mockResolvedValueOnce(empty('native-first'))
    .mockResolvedValueOnce(snapshot('native-first', 'running'))
    .mockResolvedValueOnce(snapshot('native-first', 'waiting_approval'))
    .mockResolvedValueOnce(snapshot('native-first', 'waiting_input'))
    .mockResolvedValueOnce(snapshot('native-first', 'idle'))
    .mockResolvedValueOnce(snapshot('native-first', 'running'))
    .mockRejectedValueOnce(new Error('runtime unavailable'));
  const { result } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  expect(result.current('exact-chat')).toBe('unknown');
  await settle();
  expect(result.current('exact-chat')).toBe('running');
  for (const status of ['waiting_approval', 'waiting_input', 'idle', 'running', 'unknown']) {
    await tick();
    expect(result.current('exact-chat')).toBe(status);
  }
  expect(read.mock.calls.filter(([, token]) => token === null)).toEqual([[[], null]]);
  expect(read.mock.calls.slice(1).every(([, token]) => token === 'native-first')).toBe(true);
  expect(release).toHaveBeenCalledWith('native-first');
});

it('releases an expired lease, stays Unknown, and registers fresh only on the next poll', async () => {
  const { read, release } = fakeControls();
  read
    .mockResolvedValueOnce(empty('expired-lease'))
    .mockResolvedValueOnce(snapshot('expired-lease', 'running'))
    .mockRejectedValueOnce(new Error('expired or released lease'))
    .mockResolvedValueOnce(empty('fresh-lease'))
    .mockResolvedValueOnce(snapshot('fresh-lease', 'waiting_input'));
  const { result } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  await settle();
  expect(result.current('exact-chat')).toBe('running');
  await tick();
  expect(result.current('exact-chat')).toBe('unknown');
  expect(release).toHaveBeenLastCalledWith('expired-lease');
  expect(read).toHaveBeenCalledTimes(3);
  await tick();
  expect(read.mock.calls[3]).toEqual([[], null]);
  expect(read.mock.calls[4]).toEqual([['exact-chat'], 'fresh-lease']);
  expect(result.current('exact-chat')).toBe('waiting_input');
  await tick();
  expect(read).toHaveBeenLastCalledWith(['exact-chat'], 'fresh-lease');
  expect(release).not.toHaveBeenCalledWith('fresh-lease');
});

it('drops a late registration without any selected-ID read and serializes successor registration', async () => {
  const registration = pending();
  const { read, release } = fakeControls();
  read.mockReturnValueOnce(registration.promise);
  const { result, rerender } = renderHook(
    ({ scope }) => useLiveSessionStatus(scope, ['exact-chat']),
    { initialProps: { scope: 'old' } },
  );
  await tick();
  rerender({ scope: 'intermediate' });
  rerender({ scope: 'current' });
  await tick();
  expect(read.mock.calls).toEqual([[[], null]]);
  expect(release).not.toHaveBeenCalled();
  await act(async () => registration.resolve(empty('late-old-registration')));
  expect(release).toHaveBeenCalledWith('late-old-registration');
  expect(read.mock.calls).toEqual([
    [[], null],
    [[], null],
    [['exact-chat'], 'native-issued-1'],
  ]);
  expect(read.mock.calls.every(([, token]) => token !== 'late-old-registration')).toBe(true);
  expect(result.current('exact-chat')).toBe('idle');
  expect(release).not.toHaveBeenCalledWith('native-issued-1');
});

it('does not start a selected read while registration is pending or display registration states', async () => {
  const registration = pending();
  const selected = pending();
  const { read } = fakeControls();
  read.mockReturnValueOnce(registration.promise).mockReturnValueOnce(selected.promise);
  const { result } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  await tick();
  await tick();
  expect(read).toHaveBeenCalledTimes(1);
  await act(async () => registration.resolve(snapshot('native-token', 'running')));
  expect(result.current('exact-chat')).toBe('unknown');
  expect(read).toHaveBeenLastCalledWith(['exact-chat'], 'native-token');
  await tick();
  expect(read).toHaveBeenCalledTimes(2);
  await act(async () => selected.resolve(snapshot('native-token', 'waiting_approval')));
  expect(result.current('exact-chat')).toBe('waiting_approval');
});

it('discards a late selected reply, releases only the old token, and handles visibility changes', async () => {
  const first = pending();
  const second = pending();
  const { read, release } = fakeControls();
  read
    .mockResolvedValueOnce(empty('first-token'))
    .mockReturnValueOnce(first.promise)
    .mockResolvedValueOnce(empty('second-token'))
    .mockReturnValueOnce(second.promise);
  const { result, rerender, unmount } = renderHook(
    ({ scope, ids }) => useLiveSessionStatus(scope, ids),
    { initialProps: { scope: 'sessions:all', ids: ['exact-chat'] } },
  );
  await settle();
  await tick();
  expect(read).toHaveBeenCalledTimes(2);
  rerender({ scope: 'sessions:filtered', ids: ['exact-chat'] });
  expect(release).toHaveBeenCalledWith('first-token');
  expect(result.current('exact-chat')).toBe('unknown');
  await act(async () => first.resolve(snapshot('first-token', 'running')));
  expect(result.current('exact-chat')).toBe('unknown');
  expect(read).toHaveBeenCalledTimes(4);
  expect(read.mock.calls[2]).toEqual([[], null]);
  expect(release.mock.calls.every(([id]) => id === 'first-token')).toBe(true);
  await act(async () => second.resolve(snapshot('second-token', 'waiting_input')));
  expect(result.current('exact-chat')).toBe('waiting_input');
  rerender({ scope: 'sessions:filtered', ids: ['other-chat'] });
  expect(result.current('exact-chat')).toBe('unknown');
  expect(release).toHaveBeenCalledWith('second-token');
  await settle();
  expect(read).toHaveBeenLastCalledWith(['other-chat'], 'native-issued-1');
  expect(result.current('other-chat')).toBe('idle');
  unmount();
  expect(release).toHaveBeenLastCalledWith('native-issued-1');
});

it('releases late registration after unmount without subscribing or releasing a nonexistent token', async () => {
  const registration = pending();
  const { read, release } = fakeControls();
  read.mockReturnValueOnce(registration.promise);
  const { unmount } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  unmount();
  expect(release).not.toHaveBeenCalled();
  await act(async () => registration.resolve(empty('late-issued-token')));
  expect(release.mock.calls).toEqual([['late-issued-token']]);
  expect(read.mock.calls).toEqual([[[], null]]);
});

it('releases when IDs become empty and never restores a late Running answer', async () => {
  const selected = pending();
  const { read, release } = fakeControls();
  read.mockResolvedValueOnce(empty('empty-selection-token')).mockReturnValueOnce(selected.promise);
  const { result, rerender } = renderHook(({ ids }) => useLiveSessionStatus('sessions', ids), {
    initialProps: { ids: ['exact-chat'] },
  });
  await settle();
  rerender({ ids: [] });
  expect(release).toHaveBeenCalledWith('empty-selection-token');
  await act(async () => selected.resolve(snapshot('empty-selection-token', 'running')));
  await tick();
  expect(read).toHaveBeenCalledTimes(2);
  expect(result.current('exact-chat')).toBe('unknown');
});

it('registers again after a registration error without sending a made-up token', async () => {
  const { read, release } = fakeControls();
  read.mockRejectedValueOnce(new Error('registration unavailable'));
  const { result } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  await settle();
  expect(result.current('exact-chat')).toBe('unknown');
  expect(release).not.toHaveBeenCalled();
  await tick();
  expect(read.mock.calls).toEqual([
    [[], null],
    [[], null],
    [['exact-chat'], 'native-issued-1'],
  ]);
  expect(result.current('exact-chat')).toBe('idle');
});

it('uses new native registration on StrictMode cleanup without releasing its successor', async () => {
  const { read, release } = fakeControls('running');
  renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']), { reactStrictMode: true });
  await settle();
  expect(read.mock.calls).toEqual([
    [[], null],
    [[], null],
    [['exact-chat'], 'native-issued-2'],
  ]);
  expect(release.mock.calls).toEqual([['native-issued-1']]);
});

it('ignores unsolicited IDs and treats an unexpected status as Unknown', async () => {
  const { read } = fakeControls();
  read.mockResolvedValueOnce(empty('validated-token')).mockResolvedValueOnce({
    view_id: 'validated-token',
    states: [
      { id: 'unsolicited', status: 'running' },
      { id: 'exact-chat', status: 'unsupported' },
    ],
  } as unknown as LiveSessionSnapshot);
  const { result } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  await settle();
  expect(result.current('unsolicited')).toBe('unknown');
  expect(result.current('exact-chat')).toBe('unknown');
});

it('does not adopt a different token from a selected-read reply', async () => {
  const { read, release } = fakeControls();
  read
    .mockResolvedValueOnce(empty('issued-token'))
    .mockResolvedValueOnce(snapshot('unrequested-token', 'running'));
  const { result } = renderHook(() => useLiveSessionStatus('sessions', ['exact-chat']));
  await settle();
  expect(result.current('exact-chat')).toBe('unknown');
  expect(release.mock.calls).toEqual([['issued-token']]);
  expect(read.mock.calls).toEqual([
    [[], null],
    [['exact-chat'], 'issued-token'],
  ]);
});

it('keeps the issued lease across reordered IDs, duplicates and changes beyond the cap', async () => {
  const ids = Array.from({ length: 16 }, (_, index) => 'chat-' + index);
  const { read, release } = fakeControls();
  const { rerender } = renderHook(({ selected }) => useLiveSessionStatus('sessions', selected), {
    initialProps: { selected: [...ids, 'over-cap-one'] },
  });
  await settle();
  rerender({ selected: [...ids].reverse().concat(ids[0], 'over-cap-two') });
  expect(release).not.toHaveBeenCalled();
  expect(read).toHaveBeenCalledTimes(2);
  await tick();
  expect(read).toHaveBeenLastCalledWith([...ids].sort(), 'native-issued-1');
  expect(read.mock.calls.filter(([, token]) => token === null)).toEqual([[[], null]]);
});

it('creates no live claim without the optional capability', async () => {
  const { result } = renderHook(() => useLiveSessionStatus('fixture', ['exact-chat']));
  await tick();
  expect(result.current('exact-chat')).toBeUndefined();
  render(<LiveSessionBadge status={result.current('exact-chat')} />);
  expect(document.querySelector('[data-live-status]')).toBeNull();
});

it('supports only Claude Code and Codex hosts', () => {
  expect(isLiveSessionHost('claude')).toBe(true);
  expect(isLiveSessionHost('codex')).toBe(true);
  for (const host of ['cursor', 'unknown', 'Claude', 'constructor', 'toString']) {
    expect(isLiveSessionHost(host)).toBe(false);
  }
});

it('retains dashboard states across viewport changes, including offscreen rows, until a new answer', async () => {
  const { read } = fakeControls('running');
  const { result, rerender } = renderHook(
    ({ ids }) => useLiveSessionStatus('dashboard', ids, { keepResolved: true }),
    { initialProps: { ids: ['exact-chat'] } },
  );
  await settle();
  expect(result.current('exact-chat')).toBe('running');
  const next = pending();
  read.mockResolvedValueOnce(empty('scrolled-token')).mockReturnValueOnce(next.promise);
  rerender({ ids: ['other-chat'] });
  await settle();
  expect(result.current('exact-chat')).toBe('running');
  expect(result.current('other-chat')).toBe('unknown');
  await act(async () => next.resolve(snapshot('scrolled-token', 'running', 'other-chat')));
  expect(result.current('exact-chat')).toBe('running');
  rerender({ ids: ['exact-chat'] });
  expect(result.current('exact-chat')).toBe('running');
  await settle();
  for (const status of ['idle', 'running', 'unknown', 'running'] as const) {
    const token = read.mock.calls.at(-1)![1]!;
    read.mockResolvedValueOnce(snapshot(token, status));
    await tick();
    expect(result.current('exact-chat')).toBe(status);
    expect(result.current('other-chat')).toBe('running');
  }
  read.mockRejectedValueOnce(new Error('runtime unavailable'));
  await tick();
  expect(result.current('exact-chat')).toBe('unknown');
  expect(result.current('other-chat')).toBe('running');
  expect(read.mock.calls.every(([ids]) => ids.length <= 16)).toBe(true);
});

it.each(['scope', 'source'] as const)(
  'clears retained dashboard states on %s change and ignores late answers',
  async (change) => {
    const { read, release } = fakeControls('running');
    const { result, rerender } = renderHook(
      ({ scope }) => useLiveSessionStatus(scope, ['exact-chat'], { keepResolved: true }),
      { initialProps: { scope: 'dashboard:7d' } },
    );
    await settle();
    expect(result.current('exact-chat')).toBe('running');
    const late = pending();
    const oldToken = read.mock.calls.at(-1)![1]!;
    read.mockReturnValueOnce(late.promise);
    await tick();
    const replacement = change === 'source' ? fakeControls('idle') : { read };
    if (change === 'scope')
      read.mockImplementation(async (ids, token) => ({
        view_id: token ?? 'new-range-token',
        states: ids.map((id) => ({ id, status: 'idle' })),
      }));
    rerender({ scope: change === 'scope' ? 'dashboard:14d' : 'dashboard:7d' });
    expect(result.current('exact-chat')).toBe('unknown');
    expect(release).toHaveBeenCalledWith(oldToken);
    await act(async () => late.resolve(snapshot(oldToken, 'running')));
    expect(result.current('exact-chat')).toBe('idle');
    expect(replacement.read.mock.calls.at(-1)![0]).toEqual(['exact-chat']);
  },
);

it('reads mixed hosts under one lease and keeps missing or removed IDs Unknown', async () => {
  const { read, release } = fakeControls();
  read.mockResolvedValueOnce(empty('mixed-lease')).mockResolvedValueOnce({
    view_id: 'mixed-lease',
    states: [
      { id: 'claude-chat', status: 'idle' },
      { id: 'codex-chat', status: 'running' },
      { id: 'unsolicited-claude', status: 'running' },
    ],
  });
  const { result, rerender } = renderHook(({ ids }) => useLiveSessionStatus('mixed', ids), {
    initialProps: { ids: ['claude-chat', 'codex-chat', 'missing-claude'] },
  });
  await settle();
  expect(read.mock.calls).toEqual([
    [[], null],
    [['claude-chat', 'codex-chat', 'missing-claude'], 'mixed-lease'],
  ]);
  expect(result.current('claude-chat')).toBe('idle');
  expect(result.current('codex-chat')).toBe('running');
  expect(result.current('missing-claude')).toBe('unknown');
  expect(result.current('unsolicited-claude')).toBe('unknown');
  rerender({ ids: ['claude-chat'] });
  expect(result.current('codex-chat')).toBe('unknown');
  expect(release).toHaveBeenCalledWith('mixed-lease');
  await settle();
  expect(read).toHaveBeenLastCalledWith(['claude-chat'], 'native-issued-1');
});

it.each([
  ['running', 'Running'],
  ['waiting_approval', 'Waiting for approval'],
  ['waiting_input', 'Waiting for input'],
] as const)('shows the full textual %s label and its runtime source', (status, label) => {
  render(<LiveSessionBadge status={status} />);
  const badge = screen.getByText(label);
  expect(badge.getAttribute('data-live-status')).toBe(status);
  expect(badge.getAttribute('title')).toContain('Codex desktop runtime');
  expect(badge.getAttribute('aria-label')).toBe(`Codex · ${label}`);
});

it.each(['claude', 'codex'] as const)('labels every %s status with its own runtime', (host) => {
  const label = host === 'claude' ? 'Claude Code' : 'Codex';
  const runtime = host === 'claude' ? 'Claude Code runtime' : 'Codex desktop runtime';
  const statuses = [
    ['running', 'Running'],
    ['waiting_approval', 'Waiting for approval'],
    ['waiting_input', 'Waiting for input'],
    ['idle', 'Idle'],
    ['unknown', 'Unknown'],
  ] as const;
  const { rerender } = render(<LiveSessionBadge host={host} status={undefined} />);
  expect(document.querySelector('[data-live-status]')).toBeNull();
  for (const [status, text] of statuses) {
    rerender(<LiveSessionBadge host={host} status={status} />);
    if (status === 'idle' || status === 'unknown') {
      expect(screen.queryByRole('status')).toBeNull();
      continue;
    }
    const badge = screen.getByRole('status', { name: `${label} · ${text}` });
    expect(badge.textContent).toBe(text);
    expect(badge.getAttribute('aria-live')).toBe('off');
    expect(badge.getAttribute('title')).toBe(`${runtime} · ${text}`);
  }
});
