import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, useQuery, useQueryClient } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider, useData, useLiveUpdates } from './DataProvider';
import { FixtureDataSource } from './FixtureDataSource';
import type { DataSource } from './DataSource';
import { events } from './ipc-names';
import { queryKeys } from './query-client';
import type { FixtureExport } from './generated/FixtureExport';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { resolve, promise };
};

it('keeps range results isolated when the older request resolves last', async () => {
  const old = deferred<string>(),
    current = deferred<string>();
  const source = new FixtureDataSource(exported);
  function Range({ range }: { range: string }) {
    const query = useQuery({
      queryKey: ['metrics', 'test-range', { range }],
      queryFn: () => (range === '7d' ? old.promise : current.promise),
    });
    return (
      <output aria-label="Range result">
        {range}: {query.data ?? 'loading'}
      </output>
    );
  }
  const view = render(
    <DataProvider source={source}>
      <Range range="7d" />
    </DataProvider>,
  );
  view.rerender(
    <DataProvider source={source}>
      <Range range="30d" />
    </DataProvider>,
  );
  await act(async () => {
    current.resolve('current result');
  });
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('30d: current result'));
  await act(async () => {
    old.resolve('obsolete result');
  });
  expect(screen.getByRole('status').textContent).toBe('30d: current result');
});

it('coalesces repeated events within 500ms and invalidates only explicit hierarchical prefixes', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const names = [
    'metrics',
    'sessions',
    'hosts',
    'database',
    'fires',
    'rules',
    'prs',
    'gh',
    'metrics-other',
  ];
  const queries = Object.fromEntries(names.map((name) => [name, vi.fn().mockResolvedValue(1)]));
  function Surface({ name }: { name: string }) {
    const query = useQuery({
      queryKey: [name, 'test-surface', { range: '14d' }],
      queryFn: queries[name],
      initialData: 0,
    });
    return <output aria-label={name}>{query.data}</output>;
  }
  render(
    <DataProvider source={source}>
      {names.map((name) => (
        <Surface key={name} name={name} />
      ))}
    </DataProvider>,
  );
  // The runtime opens its screens once every listener has registered; events
  // heard before then are covered by its catch-up, not by this coalescing.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    for (let repeat = 0; repeat < 20; repeat++) source.emit(events.importReceived);
    await vi.advanceTimersByTimeAsync(499);
  });
  expect(queries.metrics).not.toHaveBeenCalled();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2);
  });
  for (const name of ['metrics', 'sessions', 'hosts', 'database']) {
    expect(queries[name]).toHaveBeenCalledOnce();
    expect(screen.getByRole('status', { name }).textContent).toBe('1');
  }
  for (const name of ['fires', 'rules', 'prs', 'gh', 'metrics-other'])
    expect(queries[name]).not.toHaveBeenCalled();
  await act(async () => {
    source.emit(events.fireReceived);
    source.emit(events.prsRefreshed);
    await vi.advanceTimersByTimeAsync(501);
  });
  for (const name of ['fires', 'rules', 'prs', 'gh']) expect(queries[name]).toHaveBeenCalledOnce();
  expect(queries['metrics-other']).not.toHaveBeenCalled();
});

it('refreshes range-keyed Dashboard queries after imports, enrichment and index status events', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const dashboard = vi.spyOn(source, 'dashboard');
  const tokensByHost = vi.spyOn(source, 'tokensByHost');
  function Metrics() {
    const week = useQuery({
      queryKey: queryKeys.dashboard(7),
      queryFn: () => source.dashboard(7),
    });
    const month = useQuery({
      queryKey: queryKeys.dashboard(30),
      queryFn: () => source.dashboard(30),
    });
    const hosts = useQuery({
      queryKey: queryKeys.tokensByHost(7),
      queryFn: () => source.tokensByHost(7),
    });
    return (
      <output aria-label="Dashboard ranges">
        {week.data?.window.days ?? '-'}/{month.data?.window.days ?? '-'}/
        {hosts.data?.window.days ?? '-'}
      </output>
    );
  }
  render(
    <DataProvider source={source}>
      <Metrics />
    </DataProvider>,
  );
  // One settle opens the runtime once its listeners register; the next lands the first reads.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(screen.getByRole('status').textContent).toBe('7/30/7');
  expect(dashboard.mock.calls).toEqual([[7], [30]]);
  // Insertion, zero-new-record enrichment (a turn or reconcile) and receipts all
  // reach the Dashboard through the shared `metrics` prefix.
  for (const event of [events.importReceived, events.turnCompleted, events.nativeIndexStatus]) {
    dashboard.mockClear();
    tokensByHost.mockClear();
    await act(async () => {
      source.emit(event);
      await vi.advanceTimersByTimeAsync(501);
    });
    expect(dashboard.mock.calls.map(([days]) => days).sort((a, b) => a - b)).toEqual([7, 30]);
    expect(tokensByHost).toHaveBeenCalledExactlyOnceWith(7);
  }
  dashboard.mockClear();
  await act(async () => {
    source.emit(events.fireReceived);
    source.emit(events.prsRefreshed);
    await vi.advanceTimersByTimeAsync(501);
  });
  expect(dashboard).not.toHaveBeenCalled();
});

it('refreshes range-keyed Environment queries after imports, enrichment and index status events', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const environment = vi.spyOn(source, 'environment');
  function Environment() {
    const week = useQuery({
      queryKey: queryKeys.environment(7),
      queryFn: () => source.environment(7),
    });
    const month = useQuery({
      queryKey: queryKeys.environment(30),
      queryFn: () => source.environment(30),
    });
    return (
      <output aria-label="Environment ranges">
        {week.data?.window.days ?? '-'}/{month.data?.window.days ?? '-'}
      </output>
    );
  }
  render(
    <DataProvider source={source}>
      <Environment />
    </DataProvider>,
  );
  // One settle opens the runtime once its listeners register; the next lands the first reads.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(screen.getByRole('status').textContent).toBe('7/30');
  expect(queryKeys.environment(14)).toEqual(['metrics', 'environment', 14]);
  for (const event of [events.importReceived, events.turnCompleted, events.nativeIndexStatus]) {
    environment.mockClear();
    await act(async () => {
      source.emit(event);
      await vi.advanceTimersByTimeAsync(501);
    });
    expect(environment.mock.calls.map(([days]) => days).sort((a, b) => a - b)).toEqual([7, 30]);
  }
  environment.mockClear();
  await act(async () => {
    source.emit(events.fireReceived);
    source.emit(events.prsRefreshed);
    await vi.advanceTimersByTimeAsync(501);
  });
  expect(environment).not.toHaveBeenCalled();
});

it('balances delayed async subscriptions under StrictMode and cancels pending invalidations', async () => {
  vi.useFakeTimers();
  const registrations: {
    done: ReturnType<typeof deferred<() => void>>;
    listener: () => void;
    stop: ReturnType<typeof vi.fn<() => void>>;
  }[] = [];
  const source: DataSource = {
    kind: 'fixture',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    dashboard: async () => exported.dashboards[0],
    tokensByHost: async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    }),
    today: async () => exported.today,
    environment: async () => exported.environments[0],
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: async () => exported.native_index,
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: vi.fn((_event, listener) => {
      const done = deferred<() => void>();
      const stop = vi.fn();
      registrations.push({ done, stop, listener });
      return done.promise;
    }),
  };
  // Its screens never open while registration is pending, so the runtime's
  // client is watched through the class.
  const invalidated = vi.spyOn(QueryClient.prototype, 'invalidateQueries');
  const mounted = vi.fn();
  function Probe() {
    mounted();
    return null;
  }
  const view = render(
    <StrictMode>
      <DataProvider source={source}>
        <Probe />
      </DataProvider>
    </StrictMode>,
  );
  expect(registrations).toHaveLength(Object.keys(events).length * 2);
  expect(mounted).not.toHaveBeenCalled();
  registrations.forEach((registration) => registration.listener());
  view.unmount();
  await act(async () => {
    registrations.forEach((registration) => registration.done.resolve(registration.stop));
    await vi.advanceTimersByTimeAsync(1000);
  });
  registrations.forEach(({ stop }) => expect(stop).toHaveBeenCalledOnce());
  expect(invalidated).not.toHaveBeenCalled();
});

it('isolates query observers and errors when replacing the active source', async () => {
  const first = new FixtureDataSource({
      ...exported,
      app_info: { ...exported.app_info, version: 'first' },
    }),
    second = new FixtureDataSource({
      ...exported,
      app_info: { ...exported.app_info, version: 'second' },
    });
  vi.spyOn(first, 'subscribe').mockRejectedValue(new Error('backend-specific detail'));
  const clients: unknown[] = [];
  function Probe() {
    const { source } = useData();
    const eventError = useLiveUpdates().state === 'failed';
    const client = useQueryClient();
    const query = useQuery({ queryKey: ['app', 'info'], queryFn: () => source.appInfo() });
    clients.push(client);
    return (
      <>
        <p>{eventError ? 'Events unavailable' : 'Events ready'}</p>
        <p>{query.data?.version ?? 'Loading'}</p>
      </>
    );
  }
  const view = render(
    <DataProvider source={first}>
      <Probe />
    </DataProvider>,
  );
  await screen.findByText('Events unavailable');
  await screen.findByText('first');
  const before = clients.at(-1);
  view.rerender(
    <DataProvider source={second}>
      <Probe />
    </DataProvider>,
  );
  expect(screen.queryByText('Events unavailable')).toBeNull();
  expect(screen.queryByText('first')).toBeNull();
  await screen.findByText('second');
  expect(clients.at(-1)).not.toBe(before);
  expect(screen.queryByText('backend-specific detail')).toBeNull();
});

it('reports synchronous adapter failure and still cleans up successful listeners', async () => {
  const source = new FixtureDataSource(exported);
  const stop = vi.fn();
  vi.spyOn(source, 'subscribe').mockImplementation((event) => {
    if (event === events.contentPurged) throw new Error('Adapter detail');
    return Promise.resolve(stop);
  });
  function Probe() {
    return <p>{useLiveUpdates().state === 'failed' ? 'Events unavailable' : 'Events ready'}</p>;
  }
  const view = render(
    <DataProvider source={source}>
      <Probe />
    </DataProvider>,
  );
  await screen.findByText('Events unavailable');
  expect(source.subscribe).toHaveBeenCalledTimes(Object.keys(events).length);
  // The failed attempt released what it had registered without waiting for unmount.
  expect(stop).toHaveBeenCalledTimes(Object.keys(events).length - 1);
  view.unmount();
  expect(stop).toHaveBeenCalledTimes(Object.keys(events).length - 1);
});

/** The two readers that carry stored pull-request titles, plus two that do not. */
function PrTitleReaders({ source, id }: { source: DataSource; id: string }) {
  const all = { search: '', hosts: null, withPrs: false };
  const list = useQuery({
    // The Sessions list's own key shape.
    queryKey: [...queryKeys.sessions(7), '', null, false],
    queryFn: () => source.sessionsList(all, null, 7),
  });
  const row = useQuery({
    queryKey: queryKeys.session(7, id),
    queryFn: () => source.sessionRow(id, 7),
  });
  useQuery({
    queryKey: queryKeys.stretches(7, id),
    queryFn: () => source.sessionStretches(id, 7),
  });
  useQuery({ queryKey: queryKeys.environment(7), queryFn: () => source.environment(7) });
  const titles = (links: { number: number; title: string | null }[] | undefined) =>
    links?.map((link) => `#${link.number}:${link.title ?? '-'}`).join(' ') ?? 'loading';
  return (
    <>
      <output aria-label="list titles">{titles(list.data?.rows[0]?.pr_links)}</output>
      <output aria-label="row titles">{titles(row.data?.pr_links)}</output>
    </>
  );
}

it('re-reads the Sessions list and exact row after a committed refresh that lands while mounted', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const id = exported.sessions[0].rows[0].id;
  const list = vi.spyOn(source, 'sessionsList');
  const row = vi.spyOn(source, 'sessionRow');
  const stretches = vi.spyOn(source, 'sessionStretches');
  const environment = vi.spyOn(source, 'environment');
  const dashboard = vi.spyOn(source, 'dashboard');
  const refresh = source.refreshPullRequests.bind(source);
  const gate = deferred<void>();
  vi.spyOn(source, 'refreshPullRequests').mockImplementation(async (ids) => {
    // A batch started elsewhere (the Dashboard) that commits later.
    await gate.promise;
    return refresh(ids);
  });
  render(
    <DataProvider source={source}>
      <PrTitleReaders source={source} id={id} />
    </DataProvider>,
  );
  // One settle opens the runtime once its listeners register; the next lands the first reads.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  // Before any refresh a fixture database has no stored titles.
  const shown = (name: string) => screen.getByRole('status', { name }).textContent;
  expect(shown('list titles')).toBe('#11:- #12:- #13:-');
  expect(shown('row titles')).toBe('#11:- #12:- #13:-');
  for (const spy of [list, row, stretches, environment, dashboard]) spy.mockClear();

  // The pull request #11 alone, deferred: nothing changes until it commits.
  const running = source.refreshPullRequests([1]);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(600);
  });
  expect(list).not.toHaveBeenCalled();
  await act(async () => {
    gate.resolve();
    expect((await running).committed).toBe(true);
    await vi.advanceTimersByTimeAsync(501);
  });
  expect(list).toHaveBeenCalledOnce();
  expect(row).toHaveBeenCalledOnce();
  // Titles are what the PR list now holds, for exactly the refreshed identity.
  expect(shown('list titles')).toBe('#11:Synthetic fixture pull request 11 #12:- #13:-');
  expect(shown('row titles')).toBe('#11:Synthetic fixture pull request 11 #12:- #13:-');
  // Nothing whose facts a title cannot change is read again.
  for (const spy of [stretches, environment, dashboard]) expect(spy).not.toHaveBeenCalled();

  // The same refresh again stores nothing new: no commit, no event, no read.
  list.mockClear();
  row.mockClear();
  await act(async () => {
    expect((await source.refreshPullRequests([1])).committed).toBe(false);
    await vi.advanceTimersByTimeAsync(501);
  });
  // A failed request commits nothing either.
  vi.mocked(source.refreshPullRequests).mockRejectedValueOnce(new Error('unavailable'));
  await act(async () => {
    await expect(source.refreshPullRequests([3])).rejects.toThrow('unavailable');
    await vi.advanceTimersByTimeAsync(501);
  });
  expect(list).not.toHaveBeenCalled();
  expect(row).not.toHaveBeenCalled();
  expect(shown('list titles')).toBe('#11:Synthetic fixture pull request 11 #12:- #13:-');
});

it('reads a Sessions list again when its first read began before a committed refresh', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const id = exported.sessions[0].rows[0].id;
  const read = source.sessionsList.bind(source);
  const first = deferred<void>();
  let calls = 0;
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (...args) => {
    calls += 1;
    // The first read takes its snapshot now, before the refresh commits, and
    // answers late.
    const page = await read(...args);
    if (calls === 1) await first.promise;
    return page;
  });
  render(
    <DataProvider source={source}>
      <PrTitleReaders source={source} id={id} />
    </DataProvider>,
  );
  // One settle opens the runtime once its listeners register; the next starts the first read.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
    expect((await source.refreshPullRequests([1, 2, 3])).committed).toBe(true);
    await vi.advanceTimersByTimeAsync(501);
  });
  expect(list).toHaveBeenCalledOnce();
  await act(async () => {
    first.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  // The helper joins the in-flight first read, then reads once more after it.
  expect(list).toHaveBeenCalledTimes(2);
  expect(screen.getByRole('status', { name: 'list titles' }).textContent).toBe(
    '#11:Synthetic fixture pull request 11 #12:- #13:Synthetic fixture pull request 13',
  );
});
