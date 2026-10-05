import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import {
  QueryClient as QueryClientClass,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryKey,
} from '@tanstack/react-query';
import { StrictMode, useEffect, useState, type ReactNode } from 'react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { AppRoutes } from '../app/AppRoutes';
import { ThemeProvider } from '../theme/ThemeProvider';
import { DataProvider, useData, useLiveUpdates, usePrRefresh } from './DataProvider';
import type { DataSource, Unsubscribe } from './DataSource';
import { FixtureDataSource } from './FixtureDataSource';
import type { FixtureExport } from './generated/FixtureExport';
import type { PrRefreshReport } from './generated/PrRefreshReport';
import type { SessionSourceStatus } from './generated/SessionSourceStatus';
import { events, type DataEvent } from './ipc-names';
import type { PrRefreshOperation } from './pr-refresh-operation';
import { queryKeys } from './query-client';
import { reconciledIndexStatus, registrationTimeoutMs } from './subscribe-invalidation';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const required = Object.values(events);
/** Listeners one attempt registers: one per data event. */
const n = required.length;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  localStorage.clear();
});
const settle = (ms = 0) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { resolve, reject, promise };
};

type Registration = {
  event: DataEvent;
  listener: (payload?: unknown) => void;
  settled: boolean;
  release: ReturnType<typeof vi.fn<Unsubscribe>>;
  resolve(): void;
  reject(): void;
};
/**
 * Replaces a source's `subscribe` with registrations the test settles. A
 * resolved registration is heard until released; a release that throws leaves
 * it heard, the way a native listener that could not be removed would be.
 */
function controlEvents(source: DataSource) {
  const registrations: Registration[] = [];
  const heard = new Set<Registration>();
  const control = {
    registrations,
    throwOn: undefined as DataEvent | undefined,
    releaseThrows: false,
    subscribe: vi.fn((event: DataEvent, listener: (payload?: unknown) => void) => {
      if (event === control.throwOn) throw new Error('adapter detail: listen refused');
      const done = deferred<Unsubscribe>();
      const registration: Registration = {
        event,
        listener,
        settled: false,
        release: vi.fn(() => {
          if (control.releaseThrows) throw new Error('adapter detail: unlisten refused');
          heard.delete(registration);
        }),
        resolve() {
          if (registration.settled) return;
          registration.settled = true;
          heard.add(registration);
          done.resolve(registration.release);
        },
        reject() {
          if (registration.settled) return;
          registration.settled = true;
          done.reject(new Error('adapter detail: listen rejected'));
        },
      };
      registrations.push(registration);
      return done.promise;
    }),
    /** Settles every registration not yet settled, optionally only some. */
    resolveAll(filter: (registration: Registration) => boolean = () => true) {
      registrations.filter((r) => !r.settled && filter(r)).forEach((r) => r.resolve());
    },
    emit(event: DataEvent, payload?: unknown) {
      [...heard].filter((r) => r.event === event).forEach((r) => r.listener(payload));
    },
    heard: () => heard.size,
    since: (from: number) => registrations.slice(from),
  };
  source.subscribe = control.subscribe;
  return control;
}

const seen = {
  clients: new Set<QueryClient>(),
  sources: new Set<DataSource>(),
  operations: new Set<PrRefreshOperation>(),
  mounts: 0,
};
beforeEach(() => {
  seen.clients.clear();
  seen.sources.clear();
  seen.operations.clear();
  seen.mounts = 0;
});
function Probe() {
  const live = useLiveUpdates();
  const { source } = useData();
  const client = useQueryClient();
  const [pr, operation] = usePrRefresh();
  seen.clients.add(client);
  seen.sources.add(source);
  seen.operations.add(operation);
  useEffect(() => {
    seen.mounts++;
  }, []);
  return (
    <>
      <output aria-label="live">
        {live.state}:{live.attempt}
      </output>
      <output aria-label="pr">
        {pr.phase}
        {pr.phase === 'done' && `:${pr.report.succeeded}`}
      </output>
      <button type="button" onClick={live.reconnect}>
        probe reconnect
      </button>
    </>
  );
}
const report_ = (succeeded: number, committed: boolean): PrRefreshReport => ({
  requested: succeeded,
  attempted: succeeded,
  succeeded,
  failed: 0,
  skipped: 0,
  unrecorded: 0,
  cancelled: false,
  committed,
  rows: [],
});
const liveText = () => screen.getByRole('status', { name: 'live' }).textContent;
/** The runtime's screens are not mounted yet; only its startup status shows. */
const fenced = () => {
  expect(screen.getByText('Connecting live updates…').getAttribute('role')).toBe('status');
  expect(screen.queryByRole('status', { name: 'live' })).toBeNull();
};
const reconnect = () => fireEvent.click(screen.getByRole('button', { name: 'probe reconnect' }));

function Surface({ queryKey, fn }: { queryKey: QueryKey; fn: () => Promise<unknown> }) {
  const query = useQuery({ queryKey, queryFn: fn, initialData: 0 });
  return <output aria-label={JSON.stringify(queryKey)}>{String(query.data)}</output>;
}

function mount(source: DataSource, children: ReactNode = <Probe />) {
  return render(<DataProvider source={source}>{children}</DataProvider>);
}

/** Runs the first attempt to a failure through a rejected registration. */
async function failFirstAttempt(control: ReturnType<typeof controlEvents>) {
  control.registrations[0].reject();
  await settle();
  expect(liveText()).toBe('failed:0');
}

it('is connected only once every listener has registered', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  mount(source);
  expect(control.subscribe).toHaveBeenCalledTimes(n);
  expect(new Set(control.registrations.map((r) => r.event))).toEqual(new Set(required));
  fenced();
  control.resolveAll((r) => r.event !== events.contentPurged);
  await settle();
  fenced();
  control.resolveAll();
  await settle();
  expect(liveText()).toBe('connected:0');
  expect(screen.queryByText('Connecting live updates…')).toBeNull();
});

/** A screen whose query has no data yet, so it reads when it mounts. */
function Reader({ queryKey, fn }: { queryKey: QueryKey; fn: () => Promise<string> }) {
  const query = useQuery({ queryKey, queryFn: fn });
  return (
    <output aria-label={`read ${JSON.stringify(queryKey)}`}>
      {query.isError ? 'error' : (query.data ?? 'loading')}
    </output>
  );
}
const readText = (queryKey: QueryKey) =>
  screen.getByRole('status', { name: `read ${JSON.stringify(queryKey)}` }).textContent;

it('opens its screens after every listener, so each first read is once and includes a change made meanwhile', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const cancelTranscript = vi.spyOn(source, 'cancelSessionTranscript');
  const transcriptRead = vi.spyOn(source, 'sessionTranscript');
  const refreshPullRequests = vi.spyOn(source, 'refreshPullRequests');
  // What storage holds; a purge and a pull-request refresh land mid-registration.
  const stored = { content: 'before purge', prs: 'before refresh' };
  const keys: QueryKey[] = [['sessions', 'list', 7], ['prs', 'list'], queryKeys.contentRetention];
  const reads = [
    vi.fn(async () => stored.content),
    vi.fn(async () => stored.prs),
    vi.fn(async () => 'metadata_only'),
  ];
  mount(
    source,
    <>
      <Probe />
      {keys.map((key, index) => (
        <Reader key={JSON.stringify(key)} queryKey={key} fn={reads[index]} />
      ))}
    </>,
  );
  control.resolveAll((r) => r.event !== events.turnCompleted);
  await settle();
  stored.content = 'after purge';
  stored.prs = 'after refresh';
  control.emit(events.contentPurged);
  control.emit(events.prsRefreshed);
  await settle(100);
  // Nothing reads (nor opens a transcript) until the attempt settles.
  fenced();
  reads.forEach((read) => expect(read).not.toHaveBeenCalled());
  expect(transcriptRead).not.toHaveBeenCalled();
  control.resolveAll();
  await settle();
  await settle();
  expect(liveText()).toBe('connected:0');
  expect(readText(keys[0])).toBe('after purge');
  expect(readText(keys[1])).toBe('after refresh');
  expect(readText(keys[2])).toBe('metadata_only');
  // The events heard while registering are covered by the catch-up, not repeated.
  await settle(2000);
  reads.forEach((read) => expect(read).toHaveBeenCalledOnce());
  expect(refreshPullRequests).not.toHaveBeenCalled();
  expect(cancelTranscript).not.toHaveBeenCalled();
});

it('keeps a first read’s error visible: the catch-up runs before the screens open, never after', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const read = vi
    .fn<() => Promise<string>>()
    .mockRejectedValueOnce(new Error('unavailable'))
    .mockResolvedValue('recovered');
  mount(
    source,
    <>
      <Probe />
      <Reader queryKey={['metrics', 'first']} fn={read} />
    </>,
  );
  control.resolveAll();
  await settle();
  await settle(2000);
  expect(liveText()).toBe('connected:0');
  expect(readText(['metrics', 'first'])).toBe('error');
  expect(read).toHaveBeenCalledOnce();
});

it('runs no catch-up for an attempt stopped before it registered, on unmount or under StrictMode', async () => {
  const invalidate = vi.spyOn(QueryClientClass.prototype, 'invalidateQueries');
  const catchUps = () =>
    invalidate.mock.calls.filter(([filters]) => filters?.queryKey?.[0] === 'database').length;
  // Unmounted while registering: late handles are released and nothing is read.
  const first = new FixtureDataSource(exported);
  const firstControl = controlEvents(first);
  const view = mount(first);
  view.unmount();
  firstControl.resolveAll();
  await settle(1000);
  expect(catchUps()).toBe(0);
  firstControl.registrations.forEach((r) => expect(r.release).toHaveBeenCalledOnce());
  // StrictMode's rehearsal attempt completing late adds nothing: one catch-up.
  const second = new FixtureDataSource(exported);
  const secondControl = controlEvents(second);
  render(
    <StrictMode>
      <DataProvider source={second}>
        <Probe />
      </DataProvider>
    </StrictMode>,
  );
  secondControl.resolveAll((r) => secondControl.registrations.indexOf(r) >= n);
  await settle();
  expect(liveText()).toBe('connected:0');
  secondControl.resolveAll();
  await settle(1000);
  expect(catchUps()).toBe(1);
});

it('fails a partial registration and releases what it registered, including late handles', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const metrics = vi.fn().mockResolvedValue(1);
  mount(
    source,
    <>
      <Probe />
      <Surface queryKey={['metrics', 'probe']} fn={metrics} />
    </>,
  );
  const [first, second, third, ...rest] = control.registrations;
  first.resolve();
  second.resolve();
  await settle();
  third.reject();
  await settle();
  expect(liveText()).toBe('failed:0');
  expect(first.release).toHaveBeenCalledOnce();
  expect(second.release).toHaveBeenCalledOnce();
  rest.forEach((r) => r.resolve());
  await settle();
  rest.forEach((r) => expect(r.release).toHaveBeenCalledOnce());
  expect(control.heard()).toBe(0);
  // Its callbacks are inert even when called directly.
  control.registrations.forEach((r) => r.listener());
  await settle(1000);
  expect(metrics).not.toHaveBeenCalled();
  // A failure is never retried on its own.
  await settle(registrationTimeoutMs * 3);
  expect(control.subscribe).toHaveBeenCalledTimes(n);
  expect(liveText()).toBe('failed:0');
});

it('fails on a synchronous adapter error without registering further or showing its detail', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  control.throwOn = events.prsRefreshed;
  const earlier = required.indexOf(events.prsRefreshed);
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/settings']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await settle();
  expect(control.subscribe).toHaveBeenCalledTimes(earlier + 1);
  // What registered before the refusal is released as it arrives.
  control.resolveAll();
  await settle();
  control.registrations.forEach((r) => expect(r.release).toHaveBeenCalledOnce());
  expect(screen.getByRole('alert').textContent).toBe('Live updates are unavailable.');
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeTruthy();
  expect(document.body.textContent).not.toContain('adapter detail');
});

it('fails a registration that outlasts its bounded wait, and leaves the retry to the reader', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  mount(source);
  control.resolveAll((r) => r.event !== events.turnCompleted);
  await settle(registrationTimeoutMs - 1);
  fenced();
  // The wait is bounded: the app opens, failed, with its Reconnect.
  await settle(1);
  expect(liveText()).toBe('failed:0');
  expect(control.heard()).toBe(0);
  const late = control.registrations.find((r) => r.event === events.turnCompleted)!;
  late.resolve();
  await settle();
  expect(late.release).toHaveBeenCalledOnce();
  expect(liveText()).toBe('failed:0');
  await settle(registrationTimeoutMs * 3);
  expect(control.subscribe).toHaveBeenCalledTimes(n);
});

it('admits one new attempt per failure, however quickly Reconnect is pressed', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  mount(source);
  await failFirstAttempt(control);
  act(() => {
    reconnect();
    reconnect();
    reconnect();
  });
  expect(liveText()).toBe('connecting:1');
  expect(control.subscribe).toHaveBeenCalledTimes(2 * n);
  // Pressing it while connecting does nothing either.
  reconnect();
  await settle();
  expect(control.subscribe).toHaveBeenCalledTimes(2 * n);
  control.since(n)[0].reject();
  await settle();
  expect(liveText()).toBe('failed:1');
  reconnect();
  control.resolveAll((r) => control.registrations.indexOf(r) >= 2 * n);
  await settle();
  expect(liveText()).toBe('connected:2');
  reconnect();
  await settle();
  expect(control.subscribe).toHaveBeenCalledTimes(3 * n);
});

it('keeps a replaced attempt from changing the state or refreshing anything', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const metrics = vi.fn().mockResolvedValue(1);
  let client!: QueryClient;
  function Client() {
    client = useQueryClient();
    return null;
  }
  mount(
    source,
    <>
      <Probe />
      <Client />
      <Surface queryKey={['metrics', 'probe']} fn={metrics} />
    </>,
  );
  // The first attempt times out with half of its registrations unanswered.
  control.resolveAll((r) => control.registrations.indexOf(r) < 4);
  await settle(registrationTimeoutMs);
  expect(liveText()).toBe('failed:0');
  const stale = control.registrations.slice(0, n);
  reconnect();
  await settle();
  // Its late success cannot connect the new attempt, nor its late error fail it.
  stale.slice(4, 6).forEach((r) => r.resolve());
  stale.slice(6).forEach((r) => r.reject());
  await settle();
  expect(liveText()).toBe('connecting:1');
  stale.slice(4, 6).forEach((r) => expect(r.release).toHaveBeenCalledOnce());
  control.resolveAll();
  await settle();
  expect(liveText()).toBe('connected:1');
  metrics.mockClear();
  // Its callbacks are inert; the current attempt's alone refresh.
  stale.forEach((r) => r.listener(exported.native_index));
  // Not even the index status the client was reconciled with.
  expect(reconciledIndexStatus.get(client)).toBeUndefined();
  await settle(1000);
  expect(metrics).not.toHaveBeenCalled();
  control.emit(events.importReceived);
  await settle(501);
  expect(metrics).toHaveBeenCalledOnce();
});

it('cannot accumulate effective listeners when releasing them fails', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const metrics = vi.fn().mockResolvedValue(1);
  mount(
    source,
    <>
      <Probe />
      <Surface queryKey={['metrics', 'probe']} fn={metrics} />
    </>,
  );
  control.releaseThrows = true;
  for (let attempt = 0; attempt < 3; attempt++) {
    // Each attempt registers all but its last listener, which then fails.
    control.resolveAll((r) => r.event !== events.contentPurged);
    control.registrations.filter((r) => !r.settled).forEach((r) => r.reject());
    await settle();
    expect(liveText()).toBe(`failed:${attempt}`);
    reconnect();
    await settle();
  }
  control.resolveAll();
  await settle();
  expect(liveText()).toBe('connected:3');
  // Three attempts' unreleasable listeners are still heard, and ignored.
  expect(control.heard()).toBe(3 * (n - 1) + n);
  metrics.mockClear();
  control.emit(events.importReceived);
  await settle(501);
  expect(metrics).toHaveBeenCalledOnce();
  expect(liveText()).toBe('connected:3');
});

it('re-reads what missed events could have changed once reconnected, and only local reads', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const refreshPullRequests = vi.spyOn(source, 'refreshPullRequests');
  const cancelPullRequestRefresh = vi.spyOn(source, 'cancelPullRequestRefresh');
  const cancelTranscript = vi.spyOn(source, 'cancelSessionTranscript');
  // One key under every prefix an event names, and the retention mode a purge follows.
  const keys: QueryKey[] = [
    ['metrics', 'dashboard', 7],
    ['sessions', 'list', 7],
    ['hosts', 'probe'],
    ['database', 'counts'],
    ['app', 'info'],
    ['fires', 'probe'],
    ['rules', 'probe'],
    ['prs', 'list'],
    ['gh', 'probe'],
    ['native', 'index'],
    queryKeys.contentRetention,
  ];
  const reads = keys.map(() => vi.fn().mockResolvedValue(1));
  const unrelated = vi.fn().mockResolvedValue(1);
  let client!: QueryClient;
  function Client() {
    client = useQueryClient();
    return null;
  }
  // A first read still in flight when the reconnect lands, as on a fresh route.
  const firstRead = deferred<string>();
  const initial = vi
    .fn<() => Promise<string>>()
    .mockReturnValueOnce(firstRead.promise)
    .mockResolvedValue('after');
  function Initial() {
    const query = useQuery({ queryKey: ['sessions', 'initial'], queryFn: initial });
    return <output aria-label="initial">{query.data ?? 'loading'}</output>;
  }
  function Screen() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button type="button" onClick={() => setOpen(true)}>
          open initial
        </button>
        {open && <Initial />}
      </>
    );
  }
  mount(
    source,
    <>
      <Probe />
      <Client />
      <Screen />
      {keys.map((key, index) => (
        <Surface key={JSON.stringify(key)} queryKey={key} fn={reads[index]} />
      ))}
      <Surface queryKey={['settings', 'unrelated']} fn={unrelated} />
    </>,
  );
  await failFirstAttempt(control);
  // Missed while failed: nothing is heard, so nothing is read.
  control.emit(events.contentPurged);
  control.emit(events.importReceived);
  await settle(1000);
  reads.forEach((read) => expect(read).not.toHaveBeenCalled());
  // Cached but unobserved queries are marked stale by the catch-up, not read.
  client.setQueryData(['sessions', 'inactive'], 1);
  client.setQueryData([...queryKeys.pullRequests, 'inactive'], 1);
  fireEvent.click(screen.getByRole('button', { name: 'open initial' }));
  await settle();
  expect(initial).toHaveBeenCalledOnce();
  reconnect();
  control.resolveAll((r) => control.registrations.indexOf(r) >= n);
  await settle();
  expect(liveText()).toBe('connected:1');
  reads.forEach((read) => expect(read).toHaveBeenCalledOnce());
  expect(unrelated).not.toHaveBeenCalled();
  expect(client.getQueryState(['sessions', 'inactive'])?.isInvalidated).toBe(true);
  expect(client.getQueryState([...queryKeys.pullRequests, 'inactive'])?.isInvalidated).toBe(true);
  expect(client.getQueryState(['settings', 'unrelated'])?.isInvalidated).toBe(false);
  // The read that began before the catch-up is followed by one after it.
  await act(async () => firstRead.resolve('before'));
  await settle();
  expect(initial).toHaveBeenCalledTimes(2);
  expect(screen.getByRole('status', { name: 'initial' }).textContent).toBe('after');
  // Local reads only: no GitHub refresh, cancel or transcript cancel.
  expect(refreshPullRequests).not.toHaveBeenCalled();
  expect(cancelPullRequestRefresh).not.toHaveBeenCalled();
  expect(cancelTranscript).not.toHaveBeenCalled();
  // Events flow as before, with initial-scan progress still narrowed to the status.
  reads.forEach((read) => read.mockClear());
  control.emit(events.nativeIndexStatus, {
    ...exported.native_index,
    phase: { phase: 'scanning', done: 1, total: 3 },
  });
  await settle(501);
  keys.forEach((key, index) =>
    key[0] === 'native'
      ? expect(reads[index]).toHaveBeenCalledOnce()
      : expect(reads[index]).not.toHaveBeenCalled(),
  );
});

it('keeps the runtime, its pull-request batch and its screens across a reconnect', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const batch = deferred<PrRefreshReport>();
  const refreshPullRequests = vi
    .spyOn(source, 'refreshPullRequests')
    .mockReturnValue(batch.promise);
  const cancelPullRequestRefresh = vi.spyOn(source, 'cancelPullRequestRefresh');
  let client!: QueryClient;
  function Client() {
    client = useQueryClient();
    return null;
  }
  function Choice() {
    const [chosen, setChosen] = useState(0);
    return (
      <button type="button" onClick={() => setChosen((value) => value + 1)}>
        choice {chosen}
      </button>
    );
  }
  mount(
    source,
    <>
      <Probe />
      <Client />
      <Choice />
    </>,
  );
  await failFirstAttempt(control);
  const cancelQueries = vi.spyOn(client, 'cancelQueries');
  const [operation] = seen.operations;
  act(() => void operation.start([1, 2]));
  expect(screen.getByRole('status', { name: 'pr' }).textContent).toBe('running');
  // Something the reader chose on screen, kept by a screen that is not remounted.
  fireEvent.click(screen.getByRole('button', { name: 'choice 0' }));
  reconnect();
  control.resolveAll((r) => control.registrations.indexOf(r) >= n);
  await settle();
  expect(liveText()).toBe('connected:1');
  expect(seen.clients.size).toBe(1);
  expect(seen.sources).toEqual(new Set([source]));
  expect(seen.operations).toEqual(new Set([operation]));
  expect(seen.mounts).toBe(1);
  expect(screen.getByRole('button', { name: 'choice 1' })).toBeTruthy();
  expect(cancelQueries).not.toHaveBeenCalled();
  // The batch is neither cancelled nor restarted, and its report still lands.
  expect(screen.getByRole('status', { name: 'pr' }).textContent).toBe('running');
  expect(cancelPullRequestRefresh).not.toHaveBeenCalled();
  const report = report_(2, false);
  await act(async () => batch.resolve(report));
  expect(screen.getByRole('status', { name: 'pr' }).textContent).toBe('done:2');
  expect(refreshPullRequests).toHaveBeenCalledOnce();
  expect(operation.getState()).toMatchObject({ phase: 'done', run: 1, report });
});

it('releases the StrictMode rehearsal attempt and hears through the real one alone', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const metrics = vi.fn().mockResolvedValue(1);
  const view = render(
    <StrictMode>
      <DataProvider source={source}>
        <Probe />
        <Surface queryKey={['metrics', 'probe']} fn={metrics} />
      </DataProvider>
    </StrictMode>,
  );
  // StrictMode's rehearsal attempt and the real one.
  expect(control.subscribe).toHaveBeenCalledTimes(2 * n);
  control.resolveAll();
  await settle();
  expect(liveText()).toBe('connected:0');
  control.registrations.slice(0, n).forEach((r) => expect(r.release).toHaveBeenCalledOnce());
  expect(control.heard()).toBe(n);
  // The rehearsal's callbacks are inert; the real attempt's refresh once.
  control.registrations.slice(0, n).forEach((r) => r.listener());
  control.emit(events.importReceived);
  await settle(501);
  expect(metrics).toHaveBeenCalledOnce();
  view.unmount();
  control.registrations.forEach((r) => expect(r.release).toHaveBeenCalledOnce());
  expect(control.heard()).toBe(0);
});

it('reconnects under StrictMode with one attempt that it releases on unmount', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const view = render(
    <StrictMode>
      <DataProvider source={source}>
        <Probe />
      </DataProvider>
    </StrictMode>,
  );
  control.registrations[n].reject();
  await settle();
  expect(liveText()).toBe('failed:0');
  reconnect();
  await settle();
  expect(control.subscribe).toHaveBeenCalledTimes(3 * n);
  expect(liveText()).toBe('connecting:1');
  control.resolveAll();
  await settle();
  expect(liveText()).toBe('connected:1');
  view.unmount();
  // Every listener that registered was released exactly once; the refused one never was.
  control.registrations.forEach((r, index) =>
    expect(r.release).toHaveBeenCalledTimes(index === n ? 0 : 1),
  );
  expect(control.heard()).toBe(0);
});

it('ends a replaced source’s pending attempt, reads and batch, and starts the new one clean', async () => {
  const first = new FixtureDataSource(exported);
  const second = new FixtureDataSource(exported);
  const firstControl = controlEvents(first);
  const secondControl = controlEvents(second);
  const batch = deferred<PrRefreshReport>();
  vi.spyOn(first, 'refreshPullRequests').mockReturnValue(batch.promise);
  let client!: QueryClient;
  function Client() {
    client = useQueryClient();
    return null;
  }
  const view = mount(
    first,
    <>
      <Probe />
      <Client />
    </>,
  );
  await failFirstAttempt(firstControl);
  const firstClient = client;
  const cancelQueries = vi.spyOn(firstClient, 'cancelQueries');
  const [operation] = seen.operations;
  act(() => void operation.start([1]));
  reconnect();
  // The reconnect is half registered when the source is replaced.
  firstControl.resolveAll((r) => firstControl.registrations.indexOf(r) >= n + 4);
  await settle();
  expect(liveText()).toBe('connecting:1');
  view.rerender(
    <DataProvider source={second}>
      <Probe />
      <Client />
    </DataProvider>,
  );
  await settle();
  // A new source is a new runtime, behind a fence of its own.
  fenced();
  expect(cancelQueries).toHaveBeenCalled();
  expect(firstControl.heard()).toBe(0);
  firstControl.resolveAll();
  firstControl.registrations.forEach((r) => r.listener());
  await settle(registrationTimeoutMs);
  expect(firstControl.heard()).toBe(0);
  // The replaced runtime's batch answers into nothing.
  await act(async () => batch.resolve(report_(1, true)));
  expect(operation.getState().phase).toBe('running');
  expect(secondControl.subscribe).toHaveBeenCalledTimes(n);
  expect(liveText()).toBe('failed:0');
  expect(client).not.toBe(firstClient);
});

const transcript: SessionSourceStatus = {
  state: 'loaded',
  generation: { generation: 'indexed', appended: false },
  sources: 1,
  dropped_records: 0,
  gaps: [],
  records: [
    {
      id: 'aaaaaaaa-0000-4000-8000-00000000000a',
      role: 'user',
      at: null,
      blocks: [{ kind: 'text', index: 0, text: 'a synthetic question' }],
    },
  ],
};
function mountApp(source: DataSource, path: string) {
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

it('leaves an open transcript and its read alone, and still cancels it on navigation', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  const id = exported.sessions.find((page) => page.window.days === 7)!.rows[0].id;
  const read = deferred<SessionSourceStatus>();
  const opened = vi.spyOn(source, 'sessionTranscript').mockReturnValue(read.promise);
  const cancelled = vi.spyOn(source, 'cancelSessionTranscript');
  mountApp(source, `/sessions/${encodeURIComponent(id)}`);
  await settle();
  // Nothing opens the transcript before the first attempt settles.
  expect(opened).not.toHaveBeenCalled();
  await failFirstAttempt2(control);
  await settle();
  expect(opened).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  control.resolveAll((r) => control.registrations.indexOf(r) >= n);
  await settle();
  expect(screen.queryByRole('button', { name: 'Reconnect' })).toBeNull();
  // The read begun before the reconnect is neither cancelled nor repeated, and lands.
  expect(cancelled).not.toHaveBeenCalled();
  await act(async () => read.resolve(transcript));
  await settle();
  expect(screen.getByText('a synthetic question')).toBeTruthy();
  expect(opened).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('link', { name: '← All sessions' }));
  await settle();
  expect(cancelled).toHaveBeenCalledOnce();
  // Exactly the read this open started, named by this open.
  const [readId] = cancelled.mock.calls[0] as unknown as [string];
  expect(readId).toBe((opened.mock.calls[0] as unknown as [string, string])[1]);
});

/** Fails the first attempt of an app mounted without the probe. */
async function failFirstAttempt2(control: ReturnType<typeof controlEvents>) {
  control.registrations[0].reject();
  await settle();
  expect(screen.getByRole('alert').textContent).toBe('Live updates are unavailable.');
}

it('announces the reconnect and its outcome, keeping focus on the one button', async () => {
  const source = new FixtureDataSource(exported);
  const control = controlEvents(source);
  mountApp(source, '/settings');
  await settle();
  // The first attempt shows only the runtime's startup status while it connects.
  expect(screen.getByText('Connecting live updates…').getAttribute('role')).toBe('status');
  expect(screen.queryByRole('button', { name: 'Reconnect' })).toBeNull();
  await failFirstAttempt2(control);
  const button = screen.getByRole('button', { name: 'Reconnect' });
  button.focus();
  fireEvent.click(button);
  expect(screen.getByText('Reconnecting live updates…').getAttribute('role')).toBe('status');
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBe(button);
  expect(button.getAttribute('aria-disabled')).toBe('true');
  expect(document.activeElement).toBe(button);
  fireEvent.click(button);
  expect(control.subscribe).toHaveBeenCalledTimes(2 * n);
  control.since(n)[3].reject();
  await settle();
  expect(screen.getByRole('alert').textContent).toBe('Live updates are still unavailable.');
  expect(screen.queryByText('Reconnecting live updates…')).toBeNull();
  expect(button.getAttribute('aria-disabled')).toBe('false');
  expect(document.activeElement).toBe(button);
  expect(document.body.textContent).not.toContain('adapter detail');
  fireEvent.click(button);
  control.resolveAll((r) => control.registrations.indexOf(r) >= 2 * n);
  await settle();
  expect(screen.queryByText(/live updates/i)).toBeNull();
  expect(screen.queryByRole('button', { name: 'Reconnect' })).toBeNull();
});
