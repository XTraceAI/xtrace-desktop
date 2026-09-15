import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider, useData } from './DataProvider';
import { FixtureDataSource } from './FixtureDataSource';
import type { DataSource } from './DataSource';
import { events } from './ipc-names';
import type { FixtureExport } from './generated/FixtureExport';
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

it('balances delayed async subscriptions under StrictMode and cancels pending invalidations', async () => {
  vi.useFakeTimers();
  const registrations: {
    done: ReturnType<typeof deferred<() => void>>;
    listener: () => void;
    stop: ReturnType<typeof vi.fn<() => void>>;
  }[] = [];
  const source: DataSource = {
    kind: 'fixture',
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    sessionsList: async () => ({ rows: [], next: null }),
    nativeIndexStatus: async () => exported.native_index,
    subscribe: vi.fn((_event, listener) => {
      const done = deferred<() => void>();
      const stop = vi.fn();
      registrations.push({ done, stop, listener });
      return done.promise;
    }),
  };
  let client: ReturnType<typeof useQueryClient>;
  function Probe() {
    client = useQueryClient();
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
  const invalidated = vi.spyOn(client!, 'invalidateQueries');
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
    const { source, eventError } = useData();
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
    if (event === events.importReceived) throw new Error('Adapter detail');
    return Promise.resolve(stop);
  });
  function Probe() {
    return <p>{useData().eventError ? 'Events unavailable' : 'Events ready'}</p>;
  }
  const view = render(
    <DataProvider source={source}>
      <Probe />
    </DataProvider>,
  );
  await screen.findByText('Events unavailable');
  expect(source.subscribe).toHaveBeenCalledTimes(Object.keys(events).length);
  view.unmount();
  expect(stop).toHaveBeenCalledOnce();
});
