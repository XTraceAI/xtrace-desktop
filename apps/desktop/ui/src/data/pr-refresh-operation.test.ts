import { QueryClient, QueryObserver } from '@tanstack/react-query';
import { afterEach, expect, it, vi } from 'vitest';
import type { DataSource } from './DataSource';
import type { PrRefreshReport } from './generated/PrRefreshReport';
import { createPrRefreshOperation } from './pr-refresh-operation';
import { queryKeys } from './query-client';

/** Nothing here reaches GitHub: every answer is a deferred synthetic one. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const report = (committed: boolean): PrRefreshReport => ({
  requested: 1,
  attempted: 1,
  succeeded: 1,
  failed: 0,
  skipped: 0,
  unrecorded: 0,
  cancelled: false,
  committed,
  rows: [],
});
function source(overrides: Partial<DataSource> = {}) {
  return {
    refreshPullRequests: vi.fn(),
    cancelPullRequestRefresh: vi.fn(async () => true),
    ...overrides,
  } as unknown as DataSource & {
    refreshPullRequests: ReturnType<typeof vi.fn>;
    cancelPullRequestRefresh: ReturnType<typeof vi.fn>;
  };
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const clients: QueryClient[] = [];
const client = () => {
  const created = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  clients.push(created);
  return created;
};
afterEach(() => {
  for (const created of clients.splice(0)) created.clear();
});

it('refuses a second batch while one runs, then accepts the next', async () => {
  const batch = deferred<PrRefreshReport>();
  const data = source({ refreshPullRequests: vi.fn(() => batch.promise) });
  const operation = createPrRefreshOperation(data, client());
  expect(operation.start([1])).toBe(true);
  expect(operation.start([2])).toBe(false);
  expect(operation.start([])).toBe(false);
  expect(data.refreshPullRequests).toHaveBeenCalledExactlyOnceWith([1]);
  batch.resolve(report(false));
  await settle();
  expect(operation.getState()).toMatchObject({ phase: 'done', ids: [1] });
  data.refreshPullRequests.mockResolvedValueOnce(report(false));
  expect(operation.start([2])).toBe(true);
  expect(data.refreshPullRequests).toHaveBeenLastCalledWith([2]);
});

it('cancels only a running batch and keeps a late answer on its own run', async () => {
  const batch = deferred<PrRefreshReport>();
  const answer = deferred<boolean>();
  const data = source({
    refreshPullRequests: vi.fn(() => batch.promise),
    cancelPullRequestRefresh: vi.fn(() => answer.promise),
  });
  const operation = createPrRefreshOperation(data, client());
  operation.cancel();
  expect(data.cancelPullRequestRefresh).not.toHaveBeenCalled();
  operation.start([1]);
  operation.cancel();
  operation.cancel();
  expect(data.cancelPullRequestRefresh).toHaveBeenCalledOnce();
  expect(operation.getState()).toMatchObject({ phase: 'running', cancel: 'asked' });
  batch.resolve({ ...report(false), cancelled: true });
  await settle();
  // A new batch starts before the old cancel answers: that answer is ignored.
  data.refreshPullRequests.mockReturnValueOnce(new Promise(() => {}));
  operation.start([2]);
  answer.resolve(false);
  await settle();
  expect(operation.getState()).toMatchObject({ phase: 'running', ids: [2], cancel: 'none' });
});

it('ignores answers once detached, as when its source runtime is replaced', async () => {
  const batch = deferred<PrRefreshReport>();
  const data = source({ refreshPullRequests: vi.fn(() => batch.promise) });
  const queries = client();
  const invalidate = vi.spyOn(queries, 'invalidateQueries');
  const operation = createPrRefreshOperation(data, queries);
  const listener = vi.fn();
  operation.subscribe(listener);
  operation.start([1]);
  listener.mockClear();
  operation.detach();
  batch.resolve(report(true));
  await settle();
  expect(operation.getState()).toMatchObject({ phase: 'running' });
  expect(listener).not.toHaveBeenCalled();
  expect(invalidate).not.toHaveBeenCalled();
});

it('states a refused batch with its message', async () => {
  const data = source({
    refreshPullRequests: vi.fn(async () => {
      throw new Error('A refresh is already running.');
    }),
  });
  const operation = createPrRefreshOperation(data, client());
  operation.start([1]);
  await settle();
  expect(operation.getState()).toMatchObject({
    phase: 'failed',
    error: 'A refresh is already running.',
  });
});

it('reads a Dashboard begun after the commit even when the first read began before it', async () => {
  const queries = client();
  const reads: ReturnType<typeof deferred<string>>[] = [];
  const observer = new QueryObserver(queries, {
    queryKey: queryKeys.dashboard(14),
    queryFn: () => {
      const read = deferred<string>();
      reads.push(read);
      return read.promise;
    },
  });
  const seen: (string | undefined)[] = [];
  const stop = observer.subscribe((result) => seen.push(result.data));
  // The first Dashboard read is in flight, with no cached data, when the batch commits.
  expect(reads).toHaveLength(1);
  const batch = deferred<PrRefreshReport>();
  const data = source({ refreshPullRequests: vi.fn(() => batch.promise) });
  const operation = createPrRefreshOperation(data, queries);
  operation.start([1]);
  batch.resolve(report(true));
  await settle();
  // That read predates the commit; it lands with the old snapshot.
  reads[0]!.resolve('before the commit');
  await settle();
  await settle();
  expect(reads).toHaveLength(2);
  reads[1]!.resolve('after the commit');
  await settle();
  expect(observer.getCurrentResult().data).toBe('after the commit');
  expect(seen.at(-1)).toBe('after the commit');
  stop();
});

it('re-reads every Dashboard range, and nothing else, only after a committed batch', async () => {
  const queries = client();
  const invalidate = vi.spyOn(queries, 'invalidateQueries');
  const batch = deferred<PrRefreshReport>();
  const data = source({ refreshPullRequests: vi.fn(() => batch.promise) });
  const operation = createPrRefreshOperation(data, queries);
  operation.start([1]);
  batch.resolve(report(false));
  await settle();
  expect(invalidate).not.toHaveBeenCalled();
  data.refreshPullRequests.mockResolvedValueOnce(report(true));
  operation.start([1]);
  await settle();
  expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
    ['metrics', 'dashboard'],
  ]);
});
