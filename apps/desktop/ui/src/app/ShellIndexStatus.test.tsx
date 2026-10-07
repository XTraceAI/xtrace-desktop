import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { Link, MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { events } from '../data/ipc-names';
import { queryKeys } from '../data/query-client';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
import { LiveUpdatesNotice } from './LiveUpdatesNotice';
import { Shell } from './Shell';
import { useNativeIndexStatus } from './useAppInfo';
import type { LocalIndexStatus } from '../kit/Sidebar';
import { sidebarIndexStatus } from './sidebar-index-status';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
// The real presenter, counted: the Shell calls it once for each of its renders.
vi.mock('./sidebar-index-status', async (original) => {
  const actual = await original<typeof import('./sidebar-index-status')>();
  return { ...actual, sidebarIndexStatus: vi.fn(actual.sidebarIndexStatus) };
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  localStorage.clear();
});

/** Pull-request refresh is not exercised by these tests. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
const scanned = (
  host: string,
  state: NativeIndexStatus['hosts'][number]['state'],
  needs_attention = false,
) => ({
  host,
  state,
  needs_attention,
  detail: null,
  sessions_imported: 1,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 3,
  records_enriched: 0,
  diagnostics: 0,
});
// Synthetic statuses: no local history is read by any of these tests.
const updating: NativeIndexStatus = {
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/synthetic/python3' },
  readers: { state: 'verified', commit: '0'.repeat(40), plugin_version: '0.0.0' },
  hosts: [scanned('claude', 'complete')],
  needs_attention: false,
  reconciles: 1,
  files_scanned: 1,
};
const scanning: NativeIndexStatus = {
  ...updating,
  phase: { phase: 'scanning' },
  freshness: { freshness: 'unknown' },
  hosts: [scanned('claude', 'pending')],
  reconciles: 0,
};

/** A native source whose every read is counted, and whose index event can be sent. */
function nativeSource(read: () => Promise<NativeIndexStatus>, listening = false) {
  const listeners = new Map<string, (payload?: unknown) => void>();
  const source = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: vi.fn(async () => ({ ...exported.app_info, fixture: null, listening })),
    dbCounts: vi.fn(async () => exported.db_counts),
    dashboard: vi.fn(async () => exported.dashboards[0]),
    tokensByHost: vi.fn(async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    })),
    today: vi.fn(async () => exported.today),
    environment: vi.fn(async () => exported.environments[0]),
    sessionsList: vi.fn(async () => ({
      window: exported.sessions[0].window,
      rows: [],
      next: null,
    })),
    nativeIndexStatus: vi.fn(read),
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' as const }),
    sessionTranscript: async () => ({
      state: 'unavailable' as const,
      reason: { reason: 'not_indexed' as const },
    }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: vi.fn(async (event: string, callback: (payload?: unknown) => void) => {
      listeners.set(event, callback);
      return () => {};
    }),
  } satisfies DataSource;
  return { source, send: (event: string, payload?: unknown) => listeners.get(event)?.(payload) };
}
function mount(path: string, source: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}
/** Under fake timers: the runtime mounts its screens once every listener has registered. */
async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(50);
  });
}
const trigger = () => screen.getByRole('button', { name: /^Local index:/ });
const openPanel = async () => {
  fireEvent.click(trigger());
  return screen.findByRole('dialog', { name: 'Local index and plugin status' });
};

it('describes the local index, not the off receiver, from one read with no report, poll or listener of its own', async () => {
  const { source } = nativeSource(async () => updating);
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  expect(screen.queryByText(/plugin · /)).toBeNull();
  const panel = await openPanel();
  // An off receiver is its own line: it neither stops indexing nor means
  // a plugin is missing, and it invents no port, surface or capture state.
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverOff',
  );
  expect(within(panel).getByText('Plugin delivery').parentElement?.textContent).toBe(
    'Plugin deliveryUnknown',
  );
  expect(within(panel).getByText('Claude Code').parentElement?.textContent).toBe('Claude CodeRead');
  expect(panel.textContent).not.toMatch(/capturing|install|47421|:\d/i);
  // One status read. No Dashboard, Environment or counts request is made for
  // the sidebar, and only the runtime's own listeners exist.
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  expect(source.dashboard).not.toHaveBeenCalled();
  expect(source.environment).not.toHaveBeenCalled();
  expect(source.dbCounts).not.toHaveBeenCalled();
  expect(source.subscribe).toHaveBeenCalledTimes(Object.values(events).length);
  // The period read remains separate from account usage.
  expect(screen.getByRole('region', { name: 'Account usage' })).toBeTruthy();
  expect(source.tokensByHost).toHaveBeenCalledTimes(1);
});

it('never polls for the sidebar, even while the status is transient', async () => {
  vi.useFakeTimers();
  const { source } = nativeSource(async () => scanning);
  mount('/prs', source);
  await settle();
  expect(trigger().textContent).toBe('index · scanning');
  await act(async () => {
    await vi.advanceTimersByTimeAsync(10_000);
  });
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
});

it('shares a page’s own read and polling instead of adding a second interval', async () => {
  vi.useFakeTimers();
  const { source } = nativeSource(async () => scanning);
  mount('/settings', source);
  await settle();
  // Settings and the sidebar mounted together: one read between them.
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  expect(trigger().textContent).toBe('index · scanning');
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3000);
  });
  // The page's one-second recovery polling alone: three polls in three seconds.
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(4);
});

it('does not read again when routes without a status reader remount', async () => {
  const { source } = nativeSource(async () => updating);
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  // Rulebook is the placeholder route the sidebar can still open (Leaderboard
  // is its disabled "soon" row), so the placeholder is left and re-entered.
  for (const name of ['Rulebook', 'Pull requests', 'Rulebook']) {
    fireEvent.click(screen.getByRole('button', { name }));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
  }
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  expect(source.subscribe).toHaveBeenCalledTimes(Object.values(events).length);
});

it('follows the status event through the shared query, with one read per change', async () => {
  let status = updating;
  const { source, send } = nativeSource(async () => status);
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  vi.useFakeTimers();
  status = {
    ...updating,
    freshness: { freshness: 'degraded', reason: 'watcher lost' },
    hosts: [scanned('claude', 'complete'), scanned('codex', 'reader_failed', true)],
    needs_attention: true,
  };
  await act(async () => {
    send(events.nativeIndexStatus, status);
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  await waitFor(() => expect(trigger().textContent).toBe('index · degraded'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-attention');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  expect(source.dashboard).not.toHaveBeenCalled();
  expect(source.environment).not.toHaveBeenCalled();
  const panel = await openPanel();
  expect(within(panel).getByText('Updates interrupted')).toBeTruthy();
  expect(
    within(panel).getByText(/Last scan not complete for Codex \(reader failed\)/),
  ).toBeTruthy();
});

it('does not re-render the Shell for scan progress the sidebar does not show', async () => {
  let status = scanning;
  const { source, send } = nativeSource(async () => status);
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · scanning'));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  const renders = vi.mocked(sidebarIndexStatus).mock.calls.length;
  vi.useFakeTimers();
  status = { ...scanning, files_scanned: 250, reconciles: 4 };
  await act(async () => {
    send(events.nativeIndexStatus, status);
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  await waitFor(() => expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  // Progress refreshed the status alone; the facts shown did not change, so
  // the Shell, and every page under it, was not rendered again.
  expect(vi.mocked(sidebarIndexStatus).mock.calls.length).toBe(renders);
  expect(trigger().textContent).toBe('index · scanning');
  expect(source.tokensByHost).toHaveBeenCalledTimes(1);
});

it('shows a cached status as last known while reconnecting fails, and as current once events are heard', async () => {
  let reachable = false;
  const { source } = nativeSource(async () => updating);
  const subscribe = source.subscribe.getMockImplementation()!;
  source.subscribe.mockImplementation(async (event, callback) => {
    if (!reachable) throw new Error('listener registration failed');
    return subscribe(event, callback);
  });
  mount('/prs', source);
  expect(await screen.findByText('Live updates are unavailable.')).toBeTruthy();
  // The read itself answered “ready, live”: that is not proof this renderer
  // hears the index, so the row is not green and says it is last known.
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  expect(trigger().getAttribute('aria-label')).toBe('Local index: Last known: Updating');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-attention');
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  expect(await screen.findByText('Live updates are still unavailable.')).toBeTruthy();
  expect(trigger().textContent).toBe('index · updating?');
  const panel = await openPanel();
  expect(
    within(panel).getByText(/Live updates are unavailable, so this is the last status/),
  ).toBeTruthy();
  fireEvent.keyDown(panel, { key: 'Escape' });
  const reads = source.nativeIndexStatus.mock.calls.length;
  reachable = true;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  // The reconnect's own catch-up read, and nothing from the sidebar.
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(reads + 1);
});

// ── Reconnect recovery ──────────────────────────────────────────────────────
// Registered listeners mean events are heard from then on. They do not mean the
// status shown was read since: only a read that began after the listeners
// registered, and succeeded, makes a cached status current again.

const degraded: NativeIndexStatus = {
  ...updating,
  freshness: { freshness: 'degraded', reason: 'watcher lost' },
};
/** Status reads the test answers itself: each call waits until it is settled. */
function heldReads() {
  const waiting: {
    resolve: (status: NativeIndexStatus) => void;
    reject: (error: Error) => void;
  }[] = [];
  const read = () =>
    new Promise<NativeIndexStatus>((resolve, reject) => {
      waiting.push({ resolve, reject });
    });
  return { read, waiting };
}
/** A native source whose listeners cannot register until the test lets them. */
function unheardSource(read: () => Promise<NativeIndexStatus>) {
  const made = nativeSource(read);
  const subscribe = made.source.subscribe.getMockImplementation()!;
  const listeners = { reachable: false };
  made.source.subscribe.mockImplementation(async (event, callback) => {
    if (!listeners.reachable) throw new Error('listener registration failed');
    return subscribe(event, callback);
  });
  return { ...made, listeners };
}
const heard = () =>
  waitFor(() =>
    expect(screen.queryByText(/Live updates are|Reconnecting live updates/)).toBeNull(),
  );
/** Lets pending promise continuations and React's scheduled renders run. */
const idle = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 25));
  });
/** What every Shell render since `from` told the sidebar to show. */
const shownSince = (from: number) =>
  vi
    .mocked(sidebarIndexStatus)
    .mock.results.slice(from)
    .map((result) => result.value as LocalIndexStatus);

it('keeps a cached status last known after the listeners reconnect, until the catch-up read succeeds', async () => {
  const reads = heldReads();
  let hold = false;
  const { source, listeners } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  mount('/prs', source);
  expect(await screen.findByText('Live updates are unavailable.')).toBeTruthy();
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  hold = true;
  listeners.reachable = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  // Every listener has registered and the notice is gone, while the read that
  // catches the status up is still out.
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  expect(trigger().getAttribute('aria-label')).toBe('Local index: Last known: Updating');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-attention');
  // No render in between drew the cached “ready, live” as current.
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  const panel = await openPanel();
  expect(within(panel).getByText(/is being read again/)).toBeTruthy();
  fireEvent.keyDown(panel, { key: 'Escape' });
  // The catch-up answer is a changed status, shown as current, from one read.
  await act(async () => reads.waiting[0].resolve(degraded));
  await waitFor(() => expect(trigger().textContent).toBe('index · degraded'));
  expect(trigger().getAttribute('aria-label')).toBe('Local index: Updates interrupted');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('stays last known when the catch-up read fails, and recovers on the next read that succeeds', async () => {
  const reads = heldReads();
  let hold = false;
  const { source, listeners, send } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  hold = true;
  listeners.reachable = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await act(async () => reads.waiting[0].reject(new Error('backend-specific detail')));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-attention');
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  // Nothing is read again on its own: no retry, poll or second catch-up.
  await idle();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  // The next status event's read began after the listeners registered.
  vi.useFakeTimers();
  await act(async () => {
    send(events.nativeIndexStatus, updating);
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  await waitFor(() => expect(reads.waiting).toHaveLength(2));
  expect(trigger().textContent).toBe('index · updating?');
  await act(async () => reads.waiting[1].resolve(updating));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(3);
});

it('does not let a status read that began before the reconnect certify the status', async () => {
  // The very first read is still out when the listeners register.
  const reads = heldReads();
  const { source, listeners } = unheardSource(reads.read);
  mount('/prs', source);
  expect(await screen.findByText('Live updates are unavailable.')).toBeTruthy();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  expect(trigger().textContent).toBe('index · checking');
  listeners.reachable = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  // Its answer lands after the listeners registered, but was asked for before:
  // a change between the two would have been missed, so it is last known.
  await act(async () => reads.waiting[0].resolve(updating));
  await waitFor(() => expect(reads.waiting).toHaveLength(2));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-attention');
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  // The one follow-up read the runtime already owes began after them. Its
  // answer is identical, and still makes the status current.
  await act(async () => reads.waiting[1].resolve(updating));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  await idle();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('holds a cached status last known while a read from before the reconnect is still out', async () => {
  const reads = heldReads();
  let hold = false;
  const { source, listeners } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  vi.useFakeTimers({ toFake: ['Date'] });
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  // A page with its own status reader opens once the cached status is stale,
  // so a second read starts, and is still out, while nothing is heard.
  hold = true;
  vi.setSystemTime(Date.now() + 5000);
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  listeners.reachable = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  await act(async () => reads.waiting[0].resolve(degraded));
  await waitFor(() => expect(reads.waiting).toHaveLength(2));
  await idle();
  // The old read's changed answer is shown, still as last known.
  expect(trigger().textContent).toBe('index · degraded?');
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  await act(async () => reads.waiting[1].resolve(updating));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(3);
});

it('never marks an ordinary status event’s read as last known while it is out', async () => {
  // Heard from the start: the qualification is for a reconnect, not for reading.
  const reads = heldReads();
  let hold = false;
  const { source, send } = nativeSource(() => (hold ? reads.read() : Promise.resolve(updating)));
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  hold = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  vi.useFakeTimers();
  await act(async () => {
    send(events.nativeIndexStatus, degraded);
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await idle();
  // The event's read is out: the row neither flickers to “?” nor re-renders.
  expect(trigger().textContent).toBe('index · updating');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  expect(shownSince(from)).toEqual([]);
  await act(async () => reads.waiting[0].resolve(degraded));
  await waitFor(() => expect(trigger().textContent).toBe('index · degraded'));
  expect(shownSince(from).filter((shown) => shown.label.endsWith('?'))).toEqual([]);
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('makes the status current on an identical catch-up answer', async () => {
  const reads = heldReads();
  let hold = false;
  const { source, listeners } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  hold = true;
  listeners.reachable = true;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  // Nothing about the index changed: the answer is what was cached, from a
  // different object. It still says the status is current as of this read.
  await act(async () => reads.waiting[0].resolve(structuredClone(updating)));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

// The runtime's QueryClient, for writing to and cancelling in the cache as no
// product code does to the status: neither may make a status current.
let runtimeClient: QueryClient | undefined;
function ExposeClient() {
  runtimeClient = useQueryClient();
  return null;
}
/** Reads the status as a page does, keeping its query active outside the Shell. */
function StatusReader() {
  useNativeIndexStatus();
  return null;
}
/** A page outside the Shell: it can reconnect, and may read the status itself. */
function Outside({ children }: { children?: ReactNode }) {
  return (
    <>
      <LiveUpdatesNotice className="" buttonClassName="" />
      {children}
      <Link to="/in">enter</Link>
    </>
  );
}
/** The Shell on one route and pages without it on others, so it can unmount and remount. */
function tree(source: DataSource, at = '/in') {
  return (
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[at]}>
          <ExposeClient />
          <Routes>
            <Route element={<Shell />}>
              <Route
                path="/in"
                element={
                  <>
                    <Link to="/out">leave</Link>
                    <Link to="/quiet">leave quietly</Link>
                  </>
                }
              />
            </Route>
            <Route
              path="/out"
              element={
                <Outside>
                  <StatusReader />
                </Outside>
              }
            />
            <Route path="/quiet" element={<Outside />} />
          </Routes>
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  );
}
const noShell = () => expect(screen.queryByRole('button', { name: /^Local index:/ })).toBeNull();

it('ignores a manual cache write and a cancelled read while recovering', async () => {
  const reads = heldReads();
  let hold = false;
  const { source, listeners, send } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  render(tree(source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  hold = true;
  listeners.reachable = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  // A write into the cache is shown, but it is not a read: identical or
  // changed, it does not make the status current.
  await act(async () => {
    runtimeClient!.setQueryData(queryKeys.nativeIndex, structuredClone(updating));
  });
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  await act(async () => {
    runtimeClient!.setQueryData(queryKeys.nativeIndex, degraded);
  });
  await waitFor(() => expect(trigger().textContent).toBe('index · degraded?'));
  // The catch-up read is cancelled, and answers afterwards: a stale answer the
  // cache never accepts.
  await act(async () => {
    await runtimeClient!.cancelQueries({ queryKey: queryKeys.nativeIndex });
  });
  await act(async () => reads.waiting[0].resolve(updating));
  await idle();
  expect(trigger().textContent).toBe('index · degraded?');
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  // Only a read that began since the listeners registered, and was accepted, does.
  vi.useFakeTimers();
  await act(async () => {
    send(events.nativeIndexStatus, updating);
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  await waitFor(() => expect(reads.waiting).toHaveLength(2));
  await act(async () => reads.waiting[1].resolve(updating));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(3);
});

it('finds a cached status current when the Shell remounts, with nothing read and nothing to be notified of', async () => {
  // A frozen clock: a cached status never goes stale, so a remount cannot read.
  vi.useFakeTimers({ toFake: ['Date'] });
  const { source } = nativeSource(async () => updating);
  render(tree(source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  fireEvent.click(screen.getByRole('link', { name: 'leave' }));
  noShell();
  fireEvent.click(screen.getByRole('link', { name: 'enter' }));
  expect(trigger().textContent).toBe('index · updating');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  await idle();
  expect(trigger().textContent).toBe('index · updating');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
});

it('finds the status current when the catch-up read was accepted while the Shell was unmounted', async () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  const reads = heldReads();
  let hold = false;
  const { source, listeners } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  render(tree(source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  fireEvent.click(screen.getByRole('link', { name: 'leave' }));
  noShell();
  // Reconnected, caught up and accepted, all with no sidebar mounted to hear of it.
  hold = true;
  listeners.reachable = true;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await act(async () => reads.waiting[0].resolve(structuredClone(updating)));
  await idle();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  // The Shell mounts on a cached, fresh result: no read follows, so no success
  // will ever be announced to it. What it needs is on the cached result.
  fireEvent.click(screen.getByRole('link', { name: 'enter' }));
  expect(trigger().textContent).toBe('index · updating');
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  await idle();
  expect(trigger().textContent).toBe('index · updating');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('remounts as last known while the catch-up read is still out, and current once it is accepted', async () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  const reads = heldReads();
  let hold = false;
  const { source, listeners } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  render(tree(source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  fireEvent.click(screen.getByRole('link', { name: 'leave' }));
  hold = true;
  listeners.reachable = true;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('link', { name: 'enter' }));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  await act(async () => reads.waiting[0].resolve(structuredClone(updating)));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('lets the remounting Shell’s own read catch the status up when nothing read it meanwhile', async () => {
  const reads = heldReads();
  let hold = false;
  const { source, listeners } = unheardSource(() =>
    hold ? reads.read() : Promise.resolve(updating),
  );
  render(tree(source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  // A page with no status reader: the reconnect's catch-up finds no active
  // status query, so it marks the cached status stale and reads nothing.
  fireEvent.click(screen.getByRole('link', { name: 'leave quietly' }));
  noShell();
  hold = true;
  listeners.reachable = true;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await idle();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  // The Shell's own mount read is then the first to begin since the listeners
  // registered: last known while it is out, current when accepted, read once.
  fireEvent.click(screen.getByRole('link', { name: 'enter' }));
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  await act(async () => reads.waiting[0].resolve(structuredClone(updating)));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  await idle();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('starts again with a replaced source: the old runtime’s late answer and epoch do not reach the new one', async () => {
  // The old source never heard events, and its first read is still out.
  const old = heldReads();
  const before = unheardSource(old.read);
  const view = render(tree(before.source));
  expect(await screen.findByText('Live updates are unavailable.')).toBeTruthy();
  await waitFor(() => expect(old.waiting).toHaveLength(1));
  expect(trigger().textContent).toBe('index · checking');
  // Replaced by a source that is heard from the start.
  const after = nativeSource(async () => updating);
  view.rerender(tree(after.source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(trigger().querySelector('i')?.className).toBe('xt-status-live');
  // The old runtime's read answers now, into a cache nobody shows.
  await act(async () => old.waiting[0].resolve(degraded));
  await idle();
  expect(trigger().textContent).toBe('index · updating');
  expect(after.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  expect(before.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
});

it('gives a replacing source its own epoch: a current status before it certifies nothing after', async () => {
  const first = nativeSource(async () => updating);
  const view = render(tree(first.source));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  // The replacement's listeners cannot register: nothing of the first
  // runtime's registration carries over, so its status is last known.
  const reads = heldReads();
  let hold = false;
  const second = unheardSource(() => (hold ? reads.read() : Promise.resolve(updating)));
  view.rerender(tree(second.source));
  expect(await screen.findByText('Live updates are unavailable.')).toBeTruthy();
  await waitFor(() => expect(trigger().textContent).toBe('index · updating?'));
  // Its own reconnect is caught up by its own read, and by nothing earlier.
  hold = true;
  second.listeners.reachable = true;
  const from = vi.mocked(sidebarIndexStatus).mock.results.length;
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  await heard();
  await waitFor(() => expect(reads.waiting).toHaveLength(1));
  await idle();
  expect(trigger().textContent).toBe('index · updating?');
  expect(shownSince(from).filter((shown) => shown.tone === 'live')).toEqual([]);
  await act(async () => reads.waiting[0].resolve(structuredClone(updating)));
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  expect(second.source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  expect(first.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
});

it('says the state is unknown when the first status read fails, and invents none', async () => {
  const { source } = nativeSource(async () => {
    throw new Error('backend-specific detail');
  });
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · unknown'));
  const panel = await openPanel();
  expect(within(panel).getByText('Status unavailable')).toBeTruthy();
  expect(within(panel).getByText('No host scan reported')).toBeTruthy();
  expect(panel.textContent).not.toContain('backend-specific detail');
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
});

it('reports a listening receiver without a port, and an unread one as unknown', async () => {
  const listening = nativeSource(async () => updating, true);
  const view = mount('/prs', listening.source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  let panel = await openPanel();
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverListening',
  );
  // A receiver is not delivery: coverage stays unknown and no port is shown.
  expect(within(panel).getByText('Plugin delivery').parentElement?.textContent).toBe(
    'Plugin deliveryUnknown',
  );
  expect(panel.textContent).not.toMatch(/:\d|47421/);
  view.unmount();
  const unread = nativeSource(async () => updating);
  unread.source.appInfo.mockRejectedValue(new Error('app info unavailable'));
  mount('/prs', unread.source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  panel = await openPanel();
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverUnknown',
  );
});

it('opens Settings from the panel, where the full diagnostics are', async () => {
  const { source } = nativeSource(async () => updating);
  mount('/prs', source);
  await waitFor(() => expect(trigger().textContent).toBe('index · updating'));
  const panel = await openPanel();
  fireEvent.click(within(panel).getByRole('button', { name: 'Index details in Settings' }));
  expect(await screen.findByRole('heading', { name: 'Settings' })).toBeTruthy();
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(await screen.findByText('Ready (1 reconciliations)')).toBeTruthy();
});

it('shows the fixture’s disabled index as disabled, with its reported reason', async () => {
  mount('/prs', new FixtureDataSource(exported));
  await waitFor(() => expect(trigger().textContent).toBe('index · disabled'));
  const panel = await openPanel();
  expect(
    within(panel).getByText(
      'Local history is not being indexed: fixture mode uses a disposable database.',
    ),
  ).toBeTruthy();
  expect(within(panel).getByText('No host scan reported')).toBeTruthy();
});

it('keeps the plain plugin row in the browser preview, which reads no index', async () => {
  const { source } = nativeSource(async () => updating);
  mount('/prs', { ...source, kind: 'preview' });
  expect(await screen.findByText('plugin · unknown')).toBeTruthy();
  expect(screen.queryByRole('button', { name: /^Local index:/ })).toBeNull();
  expect(source.nativeIndexStatus).not.toHaveBeenCalled();
});
