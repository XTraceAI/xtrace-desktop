import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { act, cleanup, render, screen } from '@testing-library/react';
import { StrictMode, useState } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider, useData } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { events } from '../data/ipc-names';
import { queryKeys } from '../data/query-client';
import { useNativeIndexStatus } from './useAppInfo';

const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const ready: NativeIndexStatus = {
  ...exported.native_index,
  phase: { phase: 'ready' },
  python: { state: 'available', path: 'python3' },
};
const scanning: NativeIndexStatus = { ...ready, phase: { phase: 'scanning' } };
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
};
const flush = (ms = 0) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
/** The runtime mounts its screens once every listener has registered: one settle after render. */
const opened = () => flush();

/** A native source whose status is set by the test and whose events it emits with the Tauri payload. */
function nativeSource(initial: NativeIndexStatus = ready) {
  const state = { status: initial };
  const listeners = new Map<string, Set<(payload?: unknown) => void>>();
  const source = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: vi.fn(async () => exported.db_counts),
    dashboard: vi.fn(async (days: number): Promise<DashboardMetrics> => {
      void days;
      return exported.dashboards[0];
    }),
    tokensByHost: async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    }),
    today: async () => exported.today,
    environment: async () => exported.environments[0],
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: vi.fn(async () => state.status),
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    // Pull-request refresh is not exercised by this test.
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: vi.fn(async (event: string, listener: (payload?: unknown) => void) => {
      const set = listeners.get(event) ?? new Set();
      set.add(listener);
      listeners.set(event, set);
      return () => void set.delete(listener);
    }),
  } satisfies DataSource;
  const emit = (payload: unknown = state.status) =>
    listeners.get(events.nativeIndexStatus)?.forEach((listener) => listener(payload));
  return { source, state, emit, listeners };
}

/** A route that reads the index status and the Dashboard, as the Dashboard route does. */
function Route() {
  const { source } = useData();
  const index = useNativeIndexStatus();
  const dashboard = useQuery({
    queryKey: queryKeys.dashboard(7),
    queryFn: () => source.dashboard(7),
  });
  return (
    <p>
      {index.data?.phase.phase ?? 'status'}:{dashboard.data?.window.days ?? 'loading'}
    </p>
  );
}
let client: QueryClient | undefined;
function Probe() {
  client = useQueryClient();
  return null;
}
function App({ routes = 1 }: { routes?: number }) {
  const [shown, setShown] = useState(true);
  return (
    <>
      <Probe />
      <button onClick={() => setShown((value) => !value)}>toggle</button>
      {shown && Array.from({ length: routes }, (_, i) => <Route key={i} />)}
    </>
  );
}
const toggle = () => act(() => screen.getByRole('button', { name: 'toggle' }).click());
const ingestInvalidations = (spy: { mock: { calls: unknown[][] } }) =>
  spy.mock.calls.filter(
    ([filters]) => (filters as { queryKey: string[] }).queryKey[0] === 'metrics',
  ).length;

it('reconciles a first-seen settled status once per client, not per route mount', async () => {
  const { source } = nativeSource();
  render(
    <DataProvider source={source}>
      <App routes={2} />
    </DataProvider>,
  );
  await opened();
  await flush();
  // Missed-ready recovery: the first settled status refetches the data once,
  // although two routes read it.
  expect(source.dbCounts).not.toHaveBeenCalled();
  expect(source.dashboard).toHaveBeenCalledTimes(2);
  const invalidate = vi.spyOn(client!, 'invalidateQueries');
  // Unmount and remount inside the stale time: the cache answers, nothing is invalidated.
  await toggle();
  await flush(100);
  await toggle();
  await flush();
  expect(ingestInvalidations(invalidate)).toBe(0);
  expect(source.dashboard).toHaveBeenCalledTimes(2);
  // After the stale time an ordinary remount refetches the stale query once, without an extra invalidation.
  await toggle();
  await flush(1500);
  await toggle();
  await flush();
  expect(ingestInvalidations(invalidate)).toBe(0);
  expect(source.dashboard).toHaveBeenCalledTimes(3);
  expect(screen.getAllByText('ready:7')).toHaveLength(2);
});

it('keeps the reconciliation record per client', async () => {
  const first = nativeSource();
  const second = nativeSource();
  const view = render(
    <DataProvider source={first.source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush();
  expect(first.source.dashboard).toHaveBeenCalledTimes(2);
  // A new source gets a new client, which reconciles its own first settled status.
  view.rerender(
    <DataProvider source={second.source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush();
  expect(second.source.dashboard).toHaveBeenCalledTimes(2);
  expect(first.source.dashboard).toHaveBeenCalledTimes(2);
});

it('reconciles once when a remounted route sees a transient status settle', async () => {
  const { source, state } = nativeSource(scanning);
  render(
    <DataProvider source={source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush();
  expect(source.dashboard).toHaveBeenCalledTimes(1);
  // Unmounted while scanning: the ready event is lost, and polling stops with the observer.
  await toggle();
  state.status = ready;
  await flush(3000);
  await toggle();
  // The remount refetches the stale Dashboard (read 2) and the cached scanning
  // status; seeing that settle reconciles the data the lost event would have (read 3).
  await flush();
  expect(screen.getByText('ready:7')).toBeTruthy();
  await flush(20);
  expect(source.dashboard).toHaveBeenCalledTimes(3);
  // Unmount/remount again inside the stale time: nothing is repeated.
  await toggle();
  await toggle();
  await flush(20);
  expect(source.dashboard).toHaveBeenCalledTimes(3);
});

it('refreshes only the status during scan progress, then the data once when it settles', async () => {
  const { source, state, emit } = nativeSource(scanning);
  render(
    <DataProvider source={source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush();
  const reads = source.dashboard.mock.calls.length;
  const statuses = source.nativeIndexStatus.mock.calls.length;
  for (let i = 0; i < 40; i++) {
    await flush(250);
    emit({ ...scanning, files_scanned: i + 1 });
  }
  await flush(600);
  expect(source.dashboard).toHaveBeenCalledTimes(reads);
  expect(source.dbCounts).not.toHaveBeenCalled();
  // The status stays live: the burst refetched it (polling alone would be ~10 reads).
  expect(source.nativeIndexStatus.mock.calls.length - statuses).toBeGreaterThan(15);
  state.status = ready;
  emit();
  await flush(600);
  await flush(20);
  // One read for `ready`: the status query seeing it settle does not repeat it.
  expect(source.dashboard).toHaveBeenCalledTimes(reads + 1);
  expect(screen.getByText('ready:7')).toBeTruthy();
});

it('applies a read issued after a settling event even when one was in flight', async () => {
  const { source, state, emit } = nativeSource(scanning);
  const stale = {
    ...exported.dashboards[0],
    window: { ...exported.dashboards[0].window, days: 14 },
  };
  const inFlight = deferred<DashboardMetrics>();
  source.dashboard.mockImplementationOnce(() => inFlight.promise);
  render(
    <DataProvider source={source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush();
  expect(source.dashboard).toHaveBeenCalledTimes(1);
  // The initial scan commits its last batch and settles while the first read is in flight.
  state.status = ready;
  emit();
  await flush(600);
  // A first read in flight is joined, not restarted, so it is read again once it lands.
  expect(source.dashboard).toHaveBeenCalledTimes(1);
  await act(async () => inFlight.resolve(stale));
  await flush(20);
  expect(source.dashboard).toHaveBeenCalledTimes(2);
  expect(screen.getByText('ready:7')).toBeTruthy();
  // A steady reconcile during a later read is not restarted either: the read
  // keeps running and one trailing read follows it.
  const second = deferred<DashboardMetrics>();
  source.dashboard.mockImplementationOnce(() => second.promise);
  emit({ ...ready, reconciles: ready.reconciles + 1 });
  await flush(600);
  emit({ ...ready, reconciles: ready.reconciles + 2 });
  await flush(600);
  expect(source.dashboard).toHaveBeenCalledTimes(3);
  await act(async () => second.resolve(stale));
  await flush(20);
  expect(source.dashboard).toHaveBeenCalledTimes(4);
  expect(screen.getByText('ready:7')).toBeTruthy();
});

it('treats a status event without a readable status as a data change', async () => {
  const { source, emit } = nativeSource();
  render(
    <DataProvider source={source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush(20);
  for (const payload of [
    undefined,
    null,
    'ready',
    { phase: 'scanning' },
    { phase: { phase: 'scanning' } },
  ]) {
    source.dashboard.mockClear();
    emit(payload);
    await flush(600);
    expect(source.dashboard).toHaveBeenCalledTimes(1);
  }
});

it('does not reconcile on a failed status read, and recovers when it succeeds', async () => {
  const { source } = nativeSource();
  source.nativeIndexStatus.mockRejectedValueOnce(new Error('unavailable'));
  render(
    <DataProvider source={source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush(20);
  expect(screen.getByText('status:7')).toBeTruthy();
  expect(source.dashboard).toHaveBeenCalledTimes(1);
  await toggle();
  await toggle();
  await flush(20);
  expect(screen.getByText('ready:7')).toBeTruthy();
  expect(source.dashboard).toHaveBeenCalledTimes(2);
});

it('drops a pending settling event when the runtime unmounts', async () => {
  const { source, emit, listeners } = nativeSource(scanning);
  const view = render(
    <DataProvider source={source}>
      <App />
    </DataProvider>,
  );
  await opened();
  await flush();
  const invalidate = vi.spyOn(client!, 'invalidateQueries');
  emit(ready);
  view.unmount();
  await flush(1000);
  expect(invalidate).not.toHaveBeenCalled();
  expect(listeners.get(events.nativeIndexStatus)?.size).toBe(0);
});

it('invalidates once per event under StrictMode double subscription', async () => {
  const { source, emit, listeners } = nativeSource();
  render(
    <StrictMode>
      <DataProvider source={source}>
        <App />
      </DataProvider>
    </StrictMode>,
  );
  await opened();
  await flush(20);
  expect(listeners.get(events.nativeIndexStatus)?.size).toBe(1);
  source.dashboard.mockClear();
  emit({ ...ready, reconciles: ready.reconciles + 1 });
  await flush(600);
  expect(source.dashboard).toHaveBeenCalledTimes(1);
});
