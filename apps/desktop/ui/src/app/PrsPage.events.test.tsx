import { act, cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { PrList } from '../data/generated/PrList';
import { events } from '../data/ipc-names';
import {
  cells,
  controlEvents,
  deferred,
  expectLocalReadsOnly,
  exported,
  mount,
  names,
  table,
  trappedSource,
} from './PrsPage.harness';
import { prRow } from './prs.synthetic';

/**
 * The mounted list against the runtime's existing event map, coalescer and
 * reconnect catch-up, none of which this page changes. A committed refresh is
 * always a batch run elsewhere (the test's own call, or a bare event); the
 * page itself never starts, cancels or polls one.
 */
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
/** One settle opens the runtime once its listeners register; the next lands the first reads. */
const open = async () => {
  await settle();
  await settle();
};

const untitled = [11, 12, 13].map((number) => `Title not cachedocto-org/xtrace-fixture#${number}`);
const titled11 = 'Synthetic fixture pull request 11octo-org/xtrace-fixture#11 ⑂ fixture/pull-11';
const titled13 = 'Synthetic fixture pull request 13octo-org/xtrace-fixture#13 ⑂ fixture/pull-13';

const before: PrList = { rows: [prRow(1)] };
const after: PrList = {
  rows: [
    prRow(1, {
      title: 'feat: committed elsewhere',
      state: 'open',
      refreshed_at_ms: Date.parse('2026-09-06T00:00:00Z'),
      last_attempted_at_ms: Date.parse('2026-09-06T00:00:00Z'),
      status: { status: 'refreshed' },
    }),
    // A link indexed meanwhile: a row the first read could not have held.
    prRow(2, { linked_sessions: 3 }),
  ],
};
const BEFORE = ['Title not cachedxtrace/app#101'];
const AFTER = ['feat: committed elsewherextrace/app#101', 'Title not cachedxtrace/app#102'];

it('re-reads the mounted list once per committed refresh or ingest, and never for one that stored nothing', async () => {
  const source = new FixtureDataSource(exported);
  // A batch run elsewhere; the spies below see only what the page itself asks.
  const commit = source.refreshPullRequests.bind(source);
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  const cancel = vi.spyOn(source, 'cancelPullRequestRefresh');
  const list = vi.spyOn(source, 'pullRequests');
  mount(source);
  await open();
  expect(names()).toEqual(untitled);
  expect(list).toHaveBeenCalledOnce();

  // Committed: the event is coalesced for 500 ms, then the list is read once.
  await act(async () => {
    expect((await commit([1])).committed).toBe(true);
    await vi.advanceTimersByTimeAsync(499);
  });
  expect(list).toHaveBeenCalledOnce();
  await settle(2);
  await settle();
  expect(list).toHaveBeenCalledTimes(2);
  expect(names()).toEqual([titled11, untitled[1], untitled[2]]);

  // The same refresh again stores nothing new: no commit, no event, no read.
  await act(async () => {
    expect((await commit([1])).committed).toBe(false);
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(list).toHaveBeenCalledTimes(2);

  // A burst of refresh events is still one read: the coalescer is unchanged.
  await act(async () => {
    expect((await commit([3])).committed).toBe(true);
    for (let repeat = 0; repeat < 20; repeat++) source.emit(events.prsRefreshed);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(list).toHaveBeenCalledTimes(3);
  expect(names()).toEqual([titled11, untitled[1], titled13]);

  // Committed ingest can add links, so an import, a turn and a settled index
  // status re-read the list: a burst of them is one coalesced read. Initial-scan
  // progress moves the status alone and reads nothing. None of it refreshes
  // anything from GitHub.
  await act(async () => {
    source.emit(events.nativeIndexStatus, {
      ...exported.native_index,
      phase: { phase: 'scanning', started_at_ms: 1, hosts: [] },
    });
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(list).toHaveBeenCalledTimes(3);
  await act(async () => {
    for (const event of [events.importReceived, events.turnCompleted, events.nativeIndexStatus])
      source.emit(event);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(list).toHaveBeenCalledTimes(4);
  expect(screen.getByTestId('prs-counted').textContent).toContain(
    'after an import, turn, backfill or index change commits new sessions or links',
  );
  expect(refresh).not.toHaveBeenCalled();
  expect(cancel).not.toHaveBeenCalled();
});

it('follows a first read that began before a committed refresh with one that began after it', async () => {
  const source = new FixtureDataSource(exported);
  const commit = source.refreshPullRequests.bind(source);
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  const read = source.pullRequests.bind(source);
  const first = deferred<void>();
  let calls = 0;
  const list = vi.spyOn(source, 'pullRequests').mockImplementation(async () => {
    calls += 1;
    // The first read takes its snapshot now, before the refresh commits, and
    // answers late.
    const snapshot = await read();
    if (calls === 1) await first.promise;
    return snapshot;
  });
  mount(source);
  await open();
  expect(list).toHaveBeenCalledOnce();
  expect(within(table()).getByRole('status', { name: 'Loading rows' })).toBeTruthy();
  await act(async () => {
    expect((await commit([1, 2, 3])).committed).toBe(true);
    await vi.advanceTimersByTimeAsync(501);
  });
  // The change joins the read already running; it is not restarted or doubled.
  expect(list).toHaveBeenCalledOnce();
  await act(async () => {
    first.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });
  await settle();
  // Exactly one read after the stale one, and its rows are the ones shown.
  expect(list).toHaveBeenCalledTimes(2);
  expect(names()).toEqual([titled11, untitled[1], titled13]);
  expect(cells()[1]![4]).toMatch(/^could not be checkedrate limited · tried /);
  await settle(5000);
  expect(list).toHaveBeenCalledTimes(2);
  expect(refresh).not.toHaveBeenCalled();
});

it('shows a re-read that failed after a commit as a failure, and reads storage again on Retry', async () => {
  const third = deferred<PrList>();
  const read = vi
    .fn<() => Promise<PrList>>()
    .mockResolvedValueOnce(before)
    .mockRejectedValueOnce(new Error('backend-specific detail'))
    .mockReturnValueOnce(third.promise);
  const source = trappedSource(read);
  const control = controlEvents(source);
  mount(source);
  control.resolveAll();
  await open();
  expect(names()).toEqual(BEFORE);
  await act(async () => {
    control.emit(events.prsRefreshed);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  // The rows read before the commit are not left standing as if current.
  const alert = screen.getByRole('alert');
  expect(alert.textContent).toBe('Indexed pull requests could not be read. Retry');
  expect(screen.queryByText(/backend-specific detail/)).toBeNull();
  expect(screen.queryByRole('table')).toBeNull();
  const retry = within(alert).getByRole('button', { name: 'Retry' });
  fireEvent.click(retry);
  await settle();
  // One read at a time: a second click cannot stack another behind it.
  expect(retry).toHaveProperty('disabled', true);
  fireEvent.click(retry);
  expect(read).toHaveBeenCalledTimes(3);
  await act(async () => {
    third.resolve(after);
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(screen.queryByRole('alert')).toBeNull();
  expect(names()).toEqual(AFTER);
  expect(read).toHaveBeenCalledTimes(3);
  expectLocalReadsOnly(source);
});

it('reconciles a change it could not hear once reconnected, with local reads only', async () => {
  const read = vi
    .fn<() => Promise<PrList>>()
    .mockResolvedValueOnce(before)
    .mockResolvedValue(after);
  const source = trappedSource(read);
  const control = controlEvents(source);
  mount(source);
  // The first attempt fails: the app opens with its notice, hearing nothing.
  control.registrations[0]!.reject();
  await open();
  expect(screen.getByRole('alert').textContent).toBe('Live updates are unavailable.');
  expect(control.heard()).toBe(0);
  expect(names()).toEqual(BEFORE);
  expect(read).toHaveBeenCalledOnce();
  expect(source.nativeIndexStatus).toHaveBeenCalledOnce();
  // A refresh commits and a link is indexed while nothing listens.
  control.emit(events.prsRefreshed);
  await settle(5000);
  expect(read).toHaveBeenCalledOnce();
  expect(names()).toEqual(BEFORE);

  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  control.resolveAll();
  await settle();
  await settle();
  // The catch-up re-reads what missed events could have changed: this list, once.
  expect(screen.queryByText(/Live updates are/)).toBeNull();
  expect(read).toHaveBeenCalledTimes(2);
  expect(names()).toEqual(AFTER);
  expect(cells()[1]![3]).toBe('3');
  // The same catch-up read the sidebar's status once more, and nothing else did.
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  // And events are heard from here on.
  read.mockResolvedValue(before);
  await act(async () => {
    control.emit(events.prsRefreshed);
    await vi.advanceTimersByTimeAsync(501);
  });
  await settle();
  expect(read).toHaveBeenCalledTimes(3);
  expect(names()).toEqual(BEFORE);
  // A committed refresh re-reads the list alone: the status stays at its two reads.
  expectLocalReadsOnly(source, 2);
});

it('follows a list read that predates the reconnect with one that began after it', async () => {
  const first = deferred<PrList>();
  const read = vi
    .fn<() => Promise<PrList>>()
    .mockReturnValueOnce(first.promise)
    .mockResolvedValue(after);
  const source = trappedSource(read);
  const control = controlEvents(source);
  mount(source);
  control.registrations[0]!.reject();
  await open();
  // The page's first read is out, holding a snapshot from before the change.
  expect(read).toHaveBeenCalledOnce();
  expect(within(table()).getByRole('status', { name: 'Loading rows' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
  control.resolveAll();
  await settle();
  // Connected says events are heard, not that this list was re-read: the
  // catch-up joined the running read, and nothing is shown as loaded yet.
  expect(screen.queryByText(/Live updates are/)).toBeNull();
  expect(read).toHaveBeenCalledOnce();
  expect(within(table()).getByRole('status', { name: 'Loading rows' })).toBeTruthy();
  // The status, which had answered, is read again by the catch-up at once.
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  await act(async () => {
    first.resolve(before);
    await vi.advanceTimersByTimeAsync(0);
  });
  await settle();
  // The older answer is followed by exactly one read that began after the
  // reconnect, and that read's rows are what stays on screen.
  expect(read).toHaveBeenCalledTimes(2);
  expect(names()).toEqual(AFTER);
  await settle(5000);
  expect(read).toHaveBeenCalledTimes(2);
  // Its initial read and the catch-up's: nothing polls the status afterwards.
  expectLocalReadsOnly(source, 2);
});
