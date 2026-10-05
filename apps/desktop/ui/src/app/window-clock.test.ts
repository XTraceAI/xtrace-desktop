import {
  InfiniteQueryObserver,
  QueryObserver,
  type QueryClient,
  type QueryKey,
} from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createQueryClient, queryKeys } from '../data/query-client';
import {
  startWindowClock,
  windowClockAlignMs,
  windowClockKeys,
  windowClockMaxAgeMs as maxAge,
  type WindowClock,
} from './window-clock';

type Deferred = { resolve: (value: string) => void; signal: AbortSignal };

/**
 * An active query whose reads are counted. Each read is answered at once
 * unless `held`, when the test answers it by hand.
 */
function reader(client: QueryClient, queryKey: QueryKey, held = false) {
  const reads: Deferred[] = [];
  const queryFn = vi.fn(
    ({ signal }: { signal: AbortSignal }) =>
      new Promise<string>((resolve) => {
        reads.push({ resolve, signal });
        if (!held) resolve(`read ${reads.length}`);
      }),
  );
  const observer = new QueryObserver(client, { queryKey, queryFn });
  const stop = observer.subscribe(() => {});
  return { reads, queryFn, observer, stop, count: () => queryFn.mock.calls.length };
}

const flush = () => vi.advanceTimersByTimeAsync(0);
const advance = (ms: number) => vi.advanceTimersByTimeAsync(ms);
let visibility: DocumentVisibilityState = 'visible';
const setVisibility = (next: DocumentVisibilityState) => {
  visibility = next;
  document.dispatchEvent(new Event('visibilitychange'));
};
const focus = () => window.dispatchEvent(new Event('focus'));

const dashboard = windowClockKeys({
  path: '/dashboard',
  sessionId: undefined,
  inventory: false,
  range: '7d',
});
const clients: QueryClient[] = [];
const clocks: WindowClock[] = [];
/** The app's own client: focus refetch off, so only the clock reads on focus. */
const newClient = () => {
  const client = createQueryClient();
  clients.push(client);
  return client;
};
const start = (client: QueryClient) => {
  const clock = startWindowClock(client);
  clocks.push(clock);
  return clock;
};

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  vi.setSystemTime(new Date('2026-09-23T12:00:00Z'));
  visibility = 'visible';
  vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility);
});
afterEach(() => {
  clocks.splice(0).forEach((clock) => clock.stop());
  clients.splice(0).forEach((client) => client.clear());
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('windowClockKeys', () => {
  it('names only the selected-window reads each route shows', () => {
    expect(dashboard).toEqual([
      queryKeys.dashboard(7),
      queryKeys.tokensByHost(7),
      queryKeys.environment(7),
    ]);
    expect(
      windowClockKeys({ path: '/prs', sessionId: undefined, inventory: false, range: '30d' }),
    ).toEqual([['metrics', 'pr-analytics', 30], queryKeys.tokensByHost(30)]);
    expect(
      windowClockKeys({ path: '/sessions', sessionId: undefined, inventory: false, range: '7d' }),
    ).toEqual([queryKeys.dashboard(7), queryKeys.tokensByHost(7), queryKeys.sessions(7)]);
    expect(
      windowClockKeys({
        path: '/sessions/:sessionId',
        sessionId: 's1',
        inventory: false,
        range: '14d',
      }),
    ).toEqual([
      queryKeys.session(14, 's1'),
      queryKeys.stretches(14, 's1'),
      queryKeys.tokensByHost(14),
    ]);
    // The inventory and every other route are never read by the clock.
    for (const route of [
      { path: '/prs', inventory: true },
      { path: '/settings', inventory: false },
      { path: '/rulebook', inventory: false },
      { path: undefined, inventory: false },
    ])
      expect(windowClockKeys({ ...route, sessionId: undefined, range: '7d' })).toEqual([]);
  });
});

describe('startWindowClock', () => {
  it('reads nothing just before the deadline and one route refresh at it', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7));
    const tokens = reader(client, queryKeys.tokensByHost(7));
    const environment = reader(client, queryKeys.environment(7));
    // Active beside them, and never the clock's to read.
    const excluded = [
      reader(client, queryKeys.dashboard(30)),
      reader(client, queryKeys.today),
      reader(client, queryKeys.pullRequests),
      reader(client, [...queryKeys.sessions(7), '', '', false]),
    ];
    await flush();
    start(client).target(dashboard);
    await flush();
    const routed = [report, tokens, environment];
    expect(routed.map((q) => q.count())).toEqual([1, 1, 1]);
    await advance(maxAge - 1);
    focus();
    await flush();
    expect(routed.map((q) => q.count())).toEqual([1, 1, 1]);
    await advance(1);
    expect(routed.map((q) => q.count())).toEqual([2, 2, 2]);
    await advance(maxAge - 1);
    expect(routed.map((q) => q.count())).toEqual([2, 2, 2]);
    await advance(1);
    expect(routed.map((q) => q.count())).toEqual([3, 3, 3]);
    expect(excluded.map((q) => q.count())).toEqual([1, 1, 1, 1]);
  });

  it('reads a report begun within the alignment window together with the due one', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7));
    await flush();
    await advance(windowClockAlignMs - 1);
    const tokens = reader(client, queryKeys.tokensByHost(7));
    await flush();
    start(client).target(dashboard);
    await advance(maxAge - windowClockAlignMs + 1);
    expect([report.count(), tokens.count()]).toEqual([2, 2]);
  });

  it('reads nothing while hidden and catches up once when shown and focused together', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7));
    await flush();
    start(client).target(dashboard);
    setVisibility('hidden');
    await advance(3 * maxAge);
    expect(report.count()).toBe(1);
    // A wake delivers both events; they coalesce into one catch-up read.
    setVisibility('visible');
    focus();
    focus();
    await flush();
    expect(report.count()).toBe(2);
    // The pace starts again from the catch-up.
    await advance(maxAge - 1);
    expect(report.count()).toBe(2);
    await advance(1);
    expect(report.count()).toBe(3);
  });

  it('catches up once after the wall clock moves backward', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7));
    await flush();
    start(client).target(dashboard);
    await advance(5 * 60_000);
    vi.setSystemTime(Date.now() - 60 * 60_000);
    focus();
    focus();
    await flush();
    expect(report.count()).toBe(2);
    setVisibility('visible');
    await flush();
    expect(report.count()).toBe(2);
    // The next read is due a full age after the catch-up, on the moved clock.
    await advance(maxAge - 1);
    expect(report.count()).toBe(2);
    await advance(1);
    expect(report.count()).toBe(3);
  });

  it('keeps a slow read running and follows it with exactly one read', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7), true);
    await flush();
    report.reads[0].resolve('first');
    await flush();
    start(client).target(dashboard);
    await advance(maxAge);
    expect(report.count()).toBe(2);
    // Still running a whole age later: not cancelled, not restarted.
    await advance(maxAge);
    focus();
    await advance(maxAge);
    expect(report.count()).toBe(2);
    expect(report.reads[1].signal.aborted).toBe(false);
    report.reads[1].resolve('slow');
    await flush();
    expect(report.count()).toBe(3);
    report.reads[2].resolve('after');
    await flush();
    expect(report.count()).toBe(3);
    expect(client.getQueryData(queryKeys.dashboard(7))).toBe('after');
  });

  it('retries a first read that failed before the clock started once at its deadline', async () => {
    const client = newClient();
    const queryFn = vi.fn(() => Promise.reject(new Error('unavailable')));
    const stop = new QueryObserver(client, { queryKey: queryKeys.dashboard(7), queryFn }).subscribe(
      () => {},
    );
    await flush();
    expect(client.getQueryState(queryKeys.dashboard(7))?.status).toBe('error');
    start(client).target(dashboard);
    await advance(maxAge - 1);
    focus();
    await flush();
    expect(queryFn).toHaveBeenCalledTimes(1);
    await advance(1);
    expect(queryFn).toHaveBeenCalledTimes(2);
    // Failing again, it is tried at the same pace and no faster.
    await advance(maxAge - 1);
    expect(queryFn).toHaveBeenCalledTimes(2);
    await advance(1);
    expect(queryFn).toHaveBeenCalledTimes(3);
    stop();
  });

  it('adds no follow-up to a read already running over old data when the clock starts', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7), true);
    await flush();
    report.reads[0].resolve('old');
    await flush();
    await advance(2 * maxAge);
    void report.observer.refetch();
    await flush();
    expect(report.count()).toBe(2);
    start(client).target(dashboard);
    focus();
    await flush();
    report.reads[1].resolve('current');
    await flush();
    expect(report.count()).toBe(2);
    // Once it lands, the ordinary deadline runs from when the clock found it.
    await advance(maxAge - 1);
    expect(report.count()).toBe(2);
    await advance(1);
    expect(report.count()).toBe(3);
  });

  it('refreshes the one loaded Sessions page with its summary and host tokens', async () => {
    const client = newClient();
    // The list shows the Dashboard summary and host tokens beside its pages.
    const report = reader(client, queryKeys.dashboard(7));
    const tokens = reader(client, queryKeys.tokensByHost(7));
    const list = reader(client, [...queryKeys.sessions(7), '', '', false]);
    await flush();
    const clock = start(client);
    clock.target(
      windowClockKeys({ path: '/sessions', sessionId: undefined, inventory: false, range: '7d' }),
    );
    await advance(maxAge);
    expect([report.count(), tokens.count(), list.count()]).toEqual([2, 2, 2]);
    setVisibility('hidden');
    await advance(3 * maxAge);
    expect([report.count(), tokens.count(), list.count()]).toEqual([2, 2, 2]);
    setVisibility('visible');
    focus();
    await flush();
    expect([report.count(), tokens.count(), list.count()]).toEqual([3, 3, 3]);
    // The Dashboard's own reads have just been refreshed on the list.
    list.stop();
    const environment = reader(client, queryKeys.environment(7));
    await flush();
    clock.target(dashboard);
    await flush();
    expect([report.count(), tokens.count(), environment.count()]).toEqual([3, 3, 1]);
  });

  it('replays three loaded Sessions pages once, and does not request an unseen fourth page', async () => {
    const client = newClient();
    const listKey = [...queryKeys.sessions(7), '', null, false];
    const read = vi.fn(async ({ pageParam }: { pageParam: number }) => ({
      page: pageParam,
      next: pageParam < 3 ? pageParam + 1 : undefined,
    }));
    const observer = new InfiniteQueryObserver(client, {
      queryKey: listKey,
      queryFn: read,
      initialPageParam: 0,
      getNextPageParam: (page) => page.next,
    });
    const stop = observer.subscribe(() => {});
    await flush();
    await observer.fetchNextPage();
    await observer.fetchNextPage();
    expect(read.mock.calls.map(([arg]) => arg.pageParam)).toEqual([0, 1, 2]);
    start(client).target(
      windowClockKeys({ path: '/sessions', sessionId: undefined, inventory: false, range: '7d' }),
    );
    await advance(maxAge);
    expect(read.mock.calls.map(([arg]) => arg.pageParam)).toEqual([0, 1, 2, 0, 1, 2]);
    expect(observer.getCurrentResult().data?.pages).toHaveLength(3);
    stop();
  });

  it('reads one session and the host tokens, never its pinned or inventory neighbours', async () => {
    const client = newClient();
    const row = reader(client, queryKeys.session(7, 's1'));
    const stretches = reader(client, queryKeys.stretches(7, 's1'));
    const tokens = reader(client, queryKeys.tokensByHost(7));
    const other = reader(client, queryKeys.session(7, 's2'));
    const pinned = reader(
      client,
      queryKeys.prSessions({
        repository: 'example/project',
        number: 1,
        windowDays: 7,
        windowEndMs: Date.now(),
        confirmedOnly: false,
      }),
    );
    await flush();
    start(client).target(
      windowClockKeys({
        path: '/sessions/:sessionId',
        sessionId: 's1',
        inventory: false,
        range: '7d',
      }),
    );
    await advance(maxAge);
    expect([row, stretches, tokens, other, pinned].map((q) => q.count())).toEqual([2, 2, 2, 1, 1]);
  });

  it('reads the observed PR report mode and host tokens, never the inventory or a cached mode', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.prAnalytics(7, false));
    const tokens = reader(client, queryKeys.tokensByHost(7));
    const inventory = reader(client, queryKeys.pullRequests);
    const confirmed = reader(client, queryKeys.prAnalytics(7, true));
    await flush();
    confirmed.stop();
    const clock = start(client);
    clock.target(
      windowClockKeys({ path: '/prs', sessionId: undefined, inventory: false, range: '7d' }),
    );
    await advance(maxAge);
    expect([report, tokens, inventory, confirmed].map((q) => q.count())).toEqual([2, 2, 1, 1]);
    clock.target(
      windowClockKeys({ path: '/prs', sessionId: undefined, inventory: true, range: '7d' }),
    );
    await advance(3 * maxAge);
    expect([report, tokens, inventory].map((q) => q.count())).toEqual([2, 2, 1]);
  });

  it('reads nothing once stopped', async () => {
    const client = newClient();
    const report = reader(client, queryKeys.dashboard(7));
    await flush();
    const clock = start(client);
    clock.target(dashboard);
    clock.stop();
    await advance(3 * maxAge);
    focus();
    await flush();
    expect(report.count()).toBe(1);
  });
});
