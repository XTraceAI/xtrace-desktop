import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { DataSource } from '../../data/DataSource';
import { DataProvider } from '../../data/DataProvider';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { TimeRange } from '../../kit/TopBar';
import { ActivityLanes } from './ActivityLanes';

class Observer {
  static current: Observer;
  readonly targets = new Set<Element>();
  constructor(private readonly callback: IntersectionObserverCallback) {
    Observer.current = this;
  }
  observe(target: Element) {
    this.targets.add(target);
  }
  unobserve(target: Element) {
    this.targets.delete(target);
  }
  disconnect() {
    this.targets.clear();
  }
  show(ids: readonly string[]) {
    act(() =>
      this.callback(
        [...this.targets].map(
          (target) =>
            ({
              target,
              isIntersecting: ids.includes((target as HTMLElement).dataset.visibleId ?? ''),
            }) as IntersectionObserverEntry,
        ),
        this as unknown as IntersectionObserver,
      ),
    );
  }
}
beforeEach(() => {
  vi.stubGlobal('IntersectionObserver', Observer);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function setup(withCapability = true, returnedParent = false, parentHost = 'codex') {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = structuredClone((fixture as FixtureExport).dashboards[0]);
  const parent = 'exact-parent-chat';
  const child = 'exact-child-chat';
  const own = 'exact-own-chat';
  const ids = [
    own,
    child,
    ...(returnedParent ? [parent] : []),
    'claude-chat',
    'claude-idle',
    'claude-waiting',
    'claude-unknown',
    'codex-input',
    'cursor-chat',
  ];
  const hostOf = (id: string) =>
    id === child || id === parent
      ? parentHost
      : id.startsWith('claude')
        ? 'claude'
        : id === 'cursor-chat'
          ? 'cursor'
          : 'codex';
  report.lanes = ids.map((session_id, index) => ({
    session_id,
    host: hostOf(session_id),
    start_ms: report.lane_end_ms - (index + 1) * 1000 - 100,
    end_ms: report.lane_end_ms - (index + 1) * 1000,
  }));
  report.lane_sessions = ids.map((session_id) => ({
    ...report.lane_sessions[0],
    session_id,
    host: hostOf(session_id),
    title: session_id,
    parent:
      session_id === child
        ? {
            session_id: parent,
            host: parentHost,
            title: parent,
            evidence: 'native_spawn' as const,
          }
        : null,
  }));
  report.lanes_total = ids.length;
  let issued = 0;
  const read = vi.fn<NonNullable<DataSource['liveSessions']>['read']>(
    async (requested, viewId) => ({
      view_id: viewId ?? `native-dashboard-${++issued}`,
      states: requested.map((id) => ({
        id,
        status:
          id === child || id === 'claude-waiting'
            ? 'waiting_approval'
            : id === 'claude-idle'
              ? 'idle'
              : id === 'claude-unknown'
                ? 'unknown'
                : id === 'codex-input'
                  ? 'waiting_input'
                  : 'running',
      })),
    }),
  );
  const release = vi.fn(async () => {});
  if (withCapability) Object.assign(source, { liveSessions: { read, release } });
  const page = (range: TimeRange) => (
    <MemoryRouter>
      <DataProvider source={source}>
        <ActivityLanes report={report} range={range} />
      </DataProvider>
    </MemoryRouter>
  );
  const view = render(page('7d'));
  const rowFor = async (id: string) => {
    const link = await screen.findByRole('link', { name: `Open session ${id}, ${id}` });
    return within(link.closest('[role="row"]')! as HTMLElement);
  };
  return { own, parent, child, report, read, release, view, page, rowFor };
}

it.each(['claude', 'codex'])(
  'reads only visible returned rows, never the absent %s parent, without textual badges',
  async (host) => {
    const { own, parent, child, read, release, view, page, rowFor } = setup(true, false, host);
    const ownRow = await rowFor(own);
    const parentLink = screen.getByRole('link', {
      name: `Open parent session ${parent}, ${parent}`,
    });
    expect(parentLink.closest('[role="row"]')!.querySelector('[data-live-status]')).toBeNull();
    expect(parentLink.closest('[role="row"]')!.querySelector('.xt-compaction')).toBeNull();
    const headers = screen.getAllByRole('columnheader');
    const compactionColumn = headers.findIndex((header) => header.textContent === 'Compactions');
    expect(compactionColumn).toBeGreaterThan(-1);
    expect(
      within(ownRow.getAllByRole('cell')[compactionColumn]).getByLabelText(
        'Recorded compactions: unknown',
      ),
    ).toBeTruthy();
    expect(
      ownRow.getByRole('link').closest('.xt-lane-name')!.querySelector('.xt-compaction'),
    ).toBeNull();
    expect(ownRow.queryByText('Unknown')).toBeNull();
    expect(read).not.toHaveBeenCalled();
    Observer.current.show([own, parent, 'claude-chat']);
    const logo = await ownRow.findByRole('img', { name: 'Codex · Running' });
    expect(logo.classList.contains('xt-lane-live-host')).toBe(true);
    expect(logo.querySelector('img')).toBeTruthy();
    expect(logo.getAttribute('title')).toContain('Running');
    expect(ownRow.queryByText('Running')).toBeNull();
    const claudeRow = await rowFor('claude-chat');
    const claudeLogo = await claudeRow.findByRole('img', { name: 'Claude Code · Running' });
    expect(claudeLogo.querySelector('img')!.getAttribute('src')).toBe('/hosts/claude.svg');
    expect(claudeLogo.getAttribute('title')).toBe('Claude Code runtime · Running');
    expect(read).toHaveBeenLastCalledWith(['claude-chat', own].sort(), expect.any(String));
    fireEvent.click(screen.getByRole('button', { name: `1 returned sub-session of ${parent}` }));
    const childRow = await rowFor(child);
    expect(childRow.getAllByRole('cell')[compactionColumn].textContent).toBe('');
    expect(childRow.queryByText('Unknown')).toBeNull();
    const firstId = read.mock.calls.at(-1)![1];
    Observer.current.show([own, child, 'claude-chat']);
    await waitFor(() =>
      expect(read).toHaveBeenLastCalledWith([child, own, 'claude-chat'].sort(), expect.any(String)),
    );
    expect(childRow.queryByText('Waiting for approval')).toBeNull();
    expect(release).toHaveBeenCalledWith(firstId);
    expect(parentLink.closest('[role="row"]')!.querySelector('[data-live-status]')).toBeNull();
    const expandedId = read.mock.calls.at(-1)![1];
    fireEvent.click(screen.getByRole('button', { name: `1 returned sub-session of ${parent}` }));
    await waitFor(() =>
      expect(read).toHaveBeenLastCalledWith([own, 'claude-chat'].sort(), expect.any(String)),
    );
    expect(release).toHaveBeenCalledWith(expandedId);
    expect(
      read.mock.calls.every(([ids]) => !ids.includes(parent) && !ids.includes('cursor-chat')),
    ).toBe(true);
    const beforeRangeId = read.mock.calls.at(-1)![1];
    view.rerender(page('14d'));
    await waitFor(() => expect(release).toHaveBeenCalledWith(beforeRangeId));
    await waitFor(() => expect(read.mock.calls.at(-1)![1]).not.toBe(beforeRangeId));
    const lastId = read.mock.calls.at(-1)![1];
    view.unmount();
    expect(release).toHaveBeenLastCalledWith(lastId);
  },
);

it.each(['claude', 'codex'])(
  'gives a returned %s parent and child their own exact state',
  async (host) => {
    const { parent, child, rowFor, read } = setup(true, true, host);
    const label = host === 'claude' ? 'Claude Code' : 'Codex';
    const parentRow = await rowFor(parent);
    Observer.current.show([parent]);
    await parentRow.findByRole('img', { name: `${label} · Running` });
    fireEvent.click(screen.getByRole('button', { name: `1 returned sub-session of ${parent}` }));
    const childRow = await rowFor(child);
    Observer.current.show([parent, child]);
    await waitFor(() =>
      expect(read).toHaveBeenLastCalledWith([parent, child].sort(), expect.any(String)),
    );
    expect(parentRow.getByRole('img', { name: `${label} · Running` })).toBeTruthy();
    expect(childRow.queryByRole('img', { name: `${label} · Running` })).toBeNull();
    expect(childRow.queryByText('Waiting for approval')).toBeNull();
    expect(childRow.queryByText('Running')).toBeNull();
    expect(read).toHaveBeenLastCalledWith([parent, child].sort(), expect.any(String));
  },
);

it('does not claim live state for a fixture without the capability', async () => {
  const { own, read, rowFor } = setup(false);
  await rowFor(own);
  Observer.current.show([own, 'claude-chat', 'claude-idle', 'cursor-chat']);
  await act(async () => {});
  expect(document.querySelector('[data-live-status]')).toBeNull();
  expect(read).not.toHaveBeenCalled();
});

it('keeps running glow and removes all textual live status labels for mixed hosts', async () => {
  const { own, read, rowFor } = setup();
  const ids = [
    own,
    'claude-chat',
    'claude-idle',
    'claude-waiting',
    'claude-unknown',
    'codex-input',
    'cursor-chat',
  ];
  await rowFor(own);
  Observer.current.show(ids);
  const running = await (
    await rowFor('claude-chat')
  ).findByRole('img', { name: 'Claude Code · Running' });
  expect(running.classList.contains('xt-lane-live-host')).toBe(true);
  for (const [id, label] of [
    ['claude-idle', 'Idle'],
    ['claude-waiting', 'Waiting for approval'],
    ['claude-unknown', 'Unknown'],
    ['codex-input', 'Waiting for input'],
  ]) {
    const row = await rowFor(id);
    expect(row.queryByText(label)).toBeNull();
    expect(row.queryByRole('img', { name: /Running/ })).toBeNull();
  }
  expect((await rowFor('cursor-chat')).queryByText('Running')).toBeNull();
  expect(document.querySelector('.xt-live-session-badge')).toBeNull();
  expect(read).toHaveBeenLastCalledWith(
    ids.filter((id) => id !== 'cursor-chat').sort(),
    expect.any(String),
  );
});

it('caps visible Claude and Codex Dashboard IDs together and releases when they scroll out', async () => {
  const { own, report, view, page, rowFor, read, release } = setup();
  await rowFor(own);
  const ids = Array.from({ length: 20 }, (_, index) => `mixed-${index}`);
  report.lanes = ids.map((session_id, index) => ({
    ...report.lanes[0],
    session_id,
    host: index % 2 === 0 ? 'claude' : 'codex',
  }));
  report.lane_sessions = ids.map((session_id, index) => ({
    ...report.lane_sessions[0],
    session_id,
    host: index % 2 === 0 ? 'claude' : 'codex',
    title: session_id,
    parent: null,
  }));
  view.rerender(page('7d'));
  await rowFor(ids[0]);
  Observer.current.show(ids);
  await (await rowFor(ids[0])).findByRole('img', { name: 'Claude Code · Running' });
  expect(read).toHaveBeenLastCalledWith(ids.slice(0, 16).sort(), expect.any(String));
  expect((await rowFor(ids[16])).queryByRole('img', { name: /Running/ })).toBeNull();
  const firstToken = read.mock.calls.at(-1)![1];
  Observer.current.show(ids.slice(16));
  await (await rowFor(ids[16])).findByRole('img', { name: 'Claude Code · Running' });
  expect(read).toHaveBeenLastCalledWith(ids.slice(16).sort(), expect.any(String));
  expect(release).toHaveBeenCalledWith(firstToken);
  expect((await rowFor(ids[0])).getByRole('img', { name: 'Claude Code · Running' })).toBeTruthy();
  const lastToken = read.mock.calls.at(-1)![1];
  Observer.current.show([]);
  await waitFor(() => expect(release).toHaveBeenCalledWith(lastToken));
  expect(read.mock.calls.every(([selected]) => selected.length <= 16)).toBe(true);
});

it('keeps offscreen running glow and removes it when polling reports idle, unknown or failure', async () => {
  const { own, read, rowFor } = setup();
  const ownRow = await rowFor(own);
  vi.useFakeTimers();
  try {
    Observer.current.show([own]);
    await act(async () => {});
    const glow = () => ownRow.queryByRole('img', { name: 'Codex · Running' });
    expect(glow()).toBeTruthy();
    Observer.current.show(['claude-chat']);
    await act(async () => {});
    expect(glow()).toBeTruthy();
    Observer.current.show([own]);
    await act(async () => {});
    for (const status of ['idle', 'running', 'unknown', 'running'] as const) {
      const token = read.mock.calls.at(-1)![1]!;
      read.mockResolvedValueOnce({ view_id: token, states: [{ id: own, status }] });
      await act(async () => vi.advanceTimersByTimeAsync(2000));
      expect(Boolean(glow())).toBe(status === 'running');
      expect(document.querySelector('.xt-live-session-badge')).toBeNull();
    }
    read.mockRejectedValueOnce(new Error('runtime unavailable'));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(glow()).toBeNull();
  } finally {
    cleanup();
    vi.useRealTimers();
  }
});
