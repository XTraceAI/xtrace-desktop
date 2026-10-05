import { act, cleanup, render, screen } from '@testing-library/react';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider, useData, useLiveUpdates } from './DataProvider';
import { FixtureDataSource } from './FixtureDataSource';
import type { PullRequestSessionsRequest } from './DataSource';
import { events } from './ipc-names';
import { queryKeys } from './query-client';
import { catchUpPrefixes } from './subscribe-invalidation';
import type { FixtureExport } from './generated/FixtureExport';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as unknown as FixtureExport;
const id11 = exported.pull_requests.rows.find((row) => row.pull_request.number === 11)!.pull_request
  .id;
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

/**
 * The report for one range and mode, and pull request #11's drilldown pinned
 * to that report's own window end, read the way a page would read them.
 */
function PrReaders({ days, confirmed }: { days: number; confirmed: boolean }) {
  const { source } = useData();
  const report = useQuery({
    queryKey: queryKeys.prAnalytics(days, confirmed),
    queryFn: () => source.pullRequestAnalytics(days, confirmed),
  });
  const request: PullRequestSessionsRequest | null = report.data
    ? {
        repository: 'octo-org/xtrace-fixture',
        number: 11,
        confirmedOnly: confirmed,
        windowDays: days,
        windowEndMs: report.data.window.end_ms,
      }
    : null;
  const drill = useInfiniteQuery({
    queryKey: request ? queryKeys.prSessions(request) : ['sessions', 'pr-linked', 'waiting'],
    enabled: request !== null,
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) => source.pullRequestSessions(request!, pageParam),
    getNextPageParam: (page) => page.next,
  });
  const facts = report.data?.report.eligibility;
  const titles = drill.data?.pages
    .flatMap((page) => page.rows)
    .flatMap((row) => row.pr_links)
    .filter((link) => link.number === 11)
    .map((link) => link.title ?? '-');
  return (
    <>
      <output aria-label="report">
        {facts ? `unknown ${facts.unknown_facts} outside ${facts.outside}` : '-'}
      </output>
      <output aria-label="drill">{titles ? titles.join() : '-'}</output>
    </>
  );
}

function Reconnect() {
  const live = useLiveUpdates();
  return (
    <button type="button" onClick={live.reconnect}>
      {live.state}
    </button>
  );
}

/** Every method that could refresh, cancel or read a source, trapped. */
function trap(source: FixtureDataSource) {
  return [
    vi.spyOn(source, 'cancelPullRequestRefresh'),
    vi.spyOn(source, 'sessionTranscript'),
    vi.spyOn(source, 'cancelSessionTranscript'),
    vi.spyOn(source, 'sessionsList'),
    vi.spyOn(source, 'sessionRow'),
    vi.spyOn(source, 'sessionStretches'),
    vi.spyOn(source, 'dashboard'),
    vi.spyOn(source, 'pullRequests'),
  ];
}

const settle = async () => {
  for (let step = 0; step < 3; step++)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
};
const shown = (name: string) => screen.getByRole('status', { name }).textContent;

it('reads the report and an open drilldown again after a committed refresh, never after none', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const report = vi.spyOn(source, 'pullRequestAnalytics');
  const drill = vi.spyOn(source, 'pullRequestSessions');
  const traps = trap(source);
  const commit = source.refreshPullRequests.bind(source);
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  render(
    <DataProvider source={source}>
      <PrReaders days={7} confirmed={false} />
    </DataProvider>,
  );
  await settle();
  // Before any refresh a fixture database has no cached facts or titles.
  expect(shown('report')).toBe('unknown 3 outside 0');
  expect(shown('drill')).toBe('-');
  expect([report.mock.calls.length, drill.mock.calls.length]).toEqual([1, 1]);

  await act(async () => {
    expect((await commit([id11])).committed).toBe(true);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect([report.mock.calls.length, drill.mock.calls.length]).toEqual([2, 2]);
  // #11 merged exactly at the window end, so it is known outside it now.
  expect(shown('report')).toBe('unknown 2 outside 1');
  expect(shown('drill')).toBe('Synthetic fixture pull request 11');
  // Pinned to the report's end either way: the anchor did not move.
  const anchors = drill.mock.calls.map(([request]) => request.windowEndMs);
  expect(new Set(anchors)).toEqual(new Set([exported.pr_analytics[0]!.window.end_ms]));

  // The same refresh stores nothing new: no commit, no event, no read.
  await act(async () => {
    expect((await commit([id11])).committed).toBe(false);
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect([report.mock.calls.length, drill.mock.calls.length]).toEqual([2, 2]);
  expect(refresh).not.toHaveBeenCalled();
  for (const spy of traps) expect(spy).not.toHaveBeenCalled();
});

it('reads them again after committed imports and a settled index, never for scan progress', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const report = vi.spyOn(source, 'pullRequestAnalytics');
  const drill = vi.spyOn(source, 'pullRequestSessions');
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  const traps = trap(source);
  render(
    <DataProvider source={source}>
      <PrReaders days={14} confirmed={true} />
    </DataProvider>,
  );
  await settle();
  const counts = () => [report.mock.calls.length, drill.mock.calls.length];
  expect(counts()).toEqual([1, 1]);
  const status = (phase: string) => ({ phase: { phase }, python: { state: 'resolving' } });
  await act(async () => {
    source.emit(events.nativeIndexStatus, status('scanning'));
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(counts()).toEqual([1, 1]);
  for (const event of [events.importReceived, events.turnCompleted, events.backfillProgress]) {
    await act(async () => {
      source.emit(event);
      await vi.advanceTimersByTimeAsync(501);
    });
    await settle();
  }
  expect(counts()).toEqual([4, 4]);
  await act(async () => {
    source.emit(events.nativeIndexStatus, status('ready'));
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(counts()).toEqual([5, 5]);
  expect(refresh).not.toHaveBeenCalled();
  for (const spy of traps) expect(spy).not.toHaveBeenCalled();
});

it('follows a first report read that began before a committed refresh with one after it', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const read = source.pullRequestAnalytics.bind(source);
  const first = deferred<void>();
  let calls = 0;
  const report = vi.spyOn(source, 'pullRequestAnalytics').mockImplementation(async (...args) => {
    calls += 1;
    // The first read takes its snapshot now, before the refresh commits, and
    // answers late.
    const page = await read(...args);
    if (calls === 1) await first.promise;
    return page;
  });
  render(
    <DataProvider source={source}>
      <PrReaders days={7} confirmed={false} />
    </DataProvider>,
  );
  await settle();
  await act(async () => {
    expect((await source.refreshPullRequests([id11])).committed).toBe(true);
    await vi.advanceTimersByTimeAsync(501);
  });
  // The running read is kept, not restarted.
  expect(report).toHaveBeenCalledOnce();
  await act(async () => {
    first.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });
  await settle();
  // One read after the one in flight, so the old snapshot does not stay.
  expect(report).toHaveBeenCalledTimes(2);
  expect(shown('report')).toBe('unknown 2 outside 1');
});

it('catches both reads up when live updates reconnect after a failed attempt', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const subscribe = source.subscribe.bind(source);
  // The first attempt fails to register one listener; the second succeeds.
  let failed = false;
  vi.spyOn(source, 'subscribe').mockImplementation(async (event, listener) => {
    if (!failed) {
      failed = true;
      throw new Error('listener registration failed');
    }
    return subscribe(event, listener);
  });
  const report = vi.spyOn(source, 'pullRequestAnalytics');
  const drill = vi.spyOn(source, 'pullRequestSessions');
  const traps = trap(source);
  render(
    <DataProvider source={source}>
      <Reconnect />
      <PrReaders days={30} confirmed={false} />
    </DataProvider>,
  );
  await settle();
  expect(screen.getByRole('button').textContent).toBe('failed');
  expect([report.mock.calls.length, drill.mock.calls.length]).toEqual([1, 1]);
  await act(async () => {
    screen.getByRole('button').click();
  });
  await settle();
  expect(screen.getByRole('button').textContent).toBe('connected');
  // Events sent while nothing listened may have changed either read.
  expect([report.mock.calls.length, drill.mock.calls.length]).toEqual([2, 2]);
  expect(
    [queryKeys.prAnalytics(7, true), queryKeys.prSessions(drill.mock.calls[0]![0])].every((key) =>
      catchUpPrefixes.some((prefix) => (typeof prefix === 'string' ? prefix === key[0] : false)),
    ),
  ).toBe(true);
  for (const spy of traps) expect(spy).not.toHaveBeenCalled();
});

it('reads locally on mount, range and mode changes, starting nothing else', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const report = vi.spyOn(source, 'pullRequestAnalytics');
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  const traps = trap(source);
  const view = render(
    <DataProvider source={source}>
      <PrReaders days={7} confirmed={false} />
    </DataProvider>,
  );
  await settle();
  for (const [days, confirmed] of [
    [30, false],
    [30, true],
    [7, true],
  ] as const) {
    view.rerender(
      <DataProvider source={source}>
        <PrReaders days={days} confirmed={confirmed} />
      </DataProvider>,
    );
    await settle();
    expect(report).toHaveBeenLastCalledWith(days, confirmed);
  }
  expect(report).toHaveBeenCalledTimes(4);
  expect(refresh).not.toHaveBeenCalled();
  for (const spy of traps) expect(spy).not.toHaveBeenCalled();
});

it('refreshes Human counts in the report and drilldown on a committed sub-session change', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  const report = vi.spyOn(source, 'pullRequestAnalytics');
  const drill = vi.spyOn(source, 'pullRequestSessions');
  const inventory = vi.spyOn(source, 'pullRequests');
  const list = vi.spyOn(source, 'sessionsList');
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  const filter = { search: '', hosts: null, withPrs: false };
  function Others() {
    useQuery({ queryKey: queryKeys.pullRequests, queryFn: () => source.pullRequests() });
    useQuery({
      queryKey: [...queryKeys.sessions(7), '', 'all', false],
      queryFn: () => source.sessionsList(filter, null, 7),
    });
    return null;
  }
  render(
    <DataProvider source={source}>
      <PrReaders days={7} confirmed={false} />
      <Others />
    </DataProvider>,
  );
  await settle();
  const counts = () => [
    report.mock.calls.length,
    drill.mock.calls.length,
    inventory.mock.calls.length,
    list.mock.calls.length,
  ];
  expect(counts()).toEqual([1, 1, 1, 1]);
  // A relation changes Human eligibility in the report and drilldown;
  // the stored pull-request inventory is unchanged.
  await act(async () => {
    source.emit(events.sessionCreationsChanged);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(counts()).toEqual([2, 2, 1, 2]);
  // Committed ingest still re-reads all four; neither event refreshes GitHub.
  await act(async () => {
    source.emit(events.importReceived);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(counts()).toEqual([3, 3, 2, 3]);
  expect(refresh).not.toHaveBeenCalled();
});
