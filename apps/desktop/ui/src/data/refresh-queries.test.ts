import { QueryClient, QueryObserver, type QueryKey } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createQueryClient } from './query-client';
import { refreshQueries } from './subscribe-invalidation';

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (e: unknown) => void;
};
const deferred = <T>(): Deferred<T> => {
  let resolve!: (value: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
};
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** An active query whose reads are answered by hand, recording each read's signal. */
function reader(client: QueryClient, queryKey: QueryKey) {
  const reads: { answer: Deferred<string>; signal: AbortSignal }[] = [];
  const queryFn = vi.fn(({ signal }: { signal: AbortSignal }) => {
    const answer = deferred<string>();
    reads.push({ answer, signal });
    return answer.promise;
  });
  const observer = new QueryObserver(client, { queryKey, queryFn, retry: false, staleTime: 0 });
  const stop = observer.subscribe(() => {});
  return { reads, queryFn, observer, stop, data: () => client.getQueryData<string>(queryKey) };
}

const clients: QueryClient[] = [];
const newClient = () => {
  const client = new QueryClient();
  clients.push(client);
  return client;
};
afterEach(() => {
  clients.splice(0).forEach((client) => client.clear());
});

describe('refreshQueries', () => {
  it('keeps a running read through a burst and follows it with exactly one read', async () => {
    const client = newClient();
    const q = reader(client, ['metrics', 'dashboard', 7]);
    await settle();
    q.reads[0].answer.resolve('first');
    await settle();
    refreshQueries(client, ['metrics'], () => {});
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    for (let i = 0; i < 5; i++) refreshQueries(client, ['metrics'], () => {});
    await settle();
    // Not cancelled, not restarted: its native work would keep running anyway.
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    expect(q.reads[1].signal.aborted).toBe(false);
    q.reads[1].answer.resolve('during the burst');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(3);
    q.reads[2].answer.resolve('after the burst');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(3);
    expect(q.data()).toBe('after the burst');
    q.stop();
  });

  it('joins a first read in flight and reads once more after it lands', async () => {
    const client = newClient();
    const q = reader(client, ['sessions', 'list']);
    await settle();
    refreshQueries(client, ['sessions'], () => {});
    refreshQueries(client, ['sessions'], () => {});
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(1);
    q.reads[0].answer.resolve('before the change');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    q.reads[1].answer.resolve('after the change');
    await settle();
    expect(q.data()).toBe('after the change');
    q.stop();
  });

  it('never loses a change that arrives while the follow-up itself is reading', async () => {
    const client = newClient();
    const q = reader(client, ['metrics', 'tokens', 7]);
    await settle();
    refreshQueries(client, ['metrics'], () => {});
    q.reads[0].answer.resolve('a');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    // The final commit lands while the follow-up read is running.
    refreshQueries(client, ['metrics'], () => {});
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    q.reads[1].answer.resolve('b');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(3);
    q.reads[2].answer.resolve('final');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(3);
    expect(q.data()).toBe('final');
    q.stop();
  });

  it('follows a failed read with a fresh read', async () => {
    const client = newClient();
    const onError = vi.fn();
    const q = reader(client, ['hosts']);
    await settle();
    refreshQueries(client, ['hosts'], onError);
    q.reads[0].answer.reject(new Error('read failed'));
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    q.reads[1].answer.resolve('recovered');
    await settle();
    expect(q.data()).toBe('recovered');
    expect(onError).not.toHaveBeenCalled();
    q.stop();
  });

  it('starts nothing once disposed or once the query left its client', async () => {
    const client = newClient();
    let live = true;
    const q = reader(client, ['metrics', 'dashboard', 14]);
    await settle();
    refreshQueries(
      client,
      ['metrics'],
      () => {},
      () => live,
    );
    live = false;
    q.reads[0].answer.resolve('landed after disposal');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(1);

    const other = newClient();
    const r = reader(other, ['metrics', 'dashboard', 30]);
    await settle();
    refreshQueries(other, ['metrics'], () => {});
    r.stop();
    other.removeQueries({ queryKey: ['metrics'] });
    r.reads[0].answer.resolve('landed after removal');
    await settle();
    expect(r.queryFn).toHaveBeenCalledTimes(1);
    q.stop();
  });

  it('keeps follow-ups per query and per client, within the prefix only', async () => {
    const [a, b] = [newClient(), newClient()];
    const a7 = reader(a, ['metrics', 'dashboard', 7]);
    const a14 = reader(a, ['metrics', 'dashboard', 14]);
    const aOther = reader(a, ['prs']);
    const b7 = reader(b, ['metrics', 'dashboard', 7]);
    await settle();
    refreshQueries(a, ['metrics'], () => {});
    refreshQueries(b, ['metrics'], () => {});
    for (const q of [a7, a14, aOther, b7]) q.reads[0].answer.resolve('first');
    await settle();
    // Each running query in each client got its own follow-up; `prs` did not.
    expect([a7, a14, b7].map((q) => q.queryFn.mock.calls.length)).toEqual([2, 2, 2]);
    expect(aOther.queryFn).toHaveBeenCalledTimes(1);
    for (const q of [a7, a14, aOther, b7]) q.stop();
  });

  it('marks inactive queries stale without reading them, and reads idle active ones', async () => {
    const client = newClient();
    const active = reader(client, ['sessions', 'page']);
    await settle();
    active.reads[0].answer.resolve('page');
    const inactiveFn = vi.fn(() => Promise.resolve('inactive'));
    await client.fetchQuery({ queryKey: ['sessions', 'other'], queryFn: inactiveFn });
    await settle();
    refreshQueries(client, ['sessions'], () => {});
    await settle();
    expect(active.queryFn).toHaveBeenCalledTimes(2);
    expect(inactiveFn).toHaveBeenCalledTimes(1);
    expect(client.getQueryState(['sessions', 'other'])?.isInvalidated).toBe(true);
    active.stop();
  });

  it('accepts a longer key prefix, such as every range of one report', async () => {
    const client = newClient();
    const dashboard = reader(client, ['metrics', 'dashboard', 7]);
    const tokens = reader(client, ['metrics', 'tokens', 7]);
    await settle();
    dashboard.reads[0].answer.resolve('d');
    tokens.reads[0].answer.resolve('t');
    await settle();
    refreshQueries(client, [['metrics', 'dashboard']], () => {});
    await settle();
    expect(dashboard.queryFn).toHaveBeenCalledTimes(2);
    expect(tokens.queryFn).toHaveBeenCalledTimes(1);
    dashboard.stop();
    tokens.stop();
  });

  /**
   * The product client (1000 ms staleTime) and a read like the DataSource
   * calls: it ignores the AbortSignal, and its answers are given by hand.
   */
  function productReader(queryKey: QueryKey) {
    const client = createQueryClient();
    clients.push(client);
    const answers: Deferred<string>[] = [];
    const queryFn = vi.fn(() => {
      const answer = deferred<string>();
      answers.push(answer);
      return answer.promise;
    });
    const mount = () => new QueryObserver(client, { queryKey, queryFn }).subscribe(() => {});
    return { client, answers, queryFn, mount, data: () => client.getQueryData<string>(queryKey) };
  }

  it('keeps a change pending across unmount, so a remount within the stale time reads again', async () => {
    const q = productReader(['metrics', 'dashboard', 7]);
    const unmount = q.mount();
    await settle();
    refreshQueries(q.client, ['metrics'], () => {});
    await settle();
    // The Dashboard unmounts while its pre-change read is still running.
    unmount();
    q.answers[0].resolve('pre-commit snapshot');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(1);
    expect(q.client.getQueryState(['metrics', 'dashboard', 7])?.isInvalidated).toBe(true);
    // An immediate remount, well inside the 1000 ms stale time, reads after the change.
    const remount = q.mount();
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    q.answers[1].resolve('post-commit');
    await settle();
    expect(q.data()).toBe('post-commit');
    expect(q.client.getQueryState(['metrics', 'dashboard', 7])?.isInvalidated).toBe(false);
    remount();
  });

  it('lets a live replacement subscription keep a change the disposed one asked for', async () => {
    const q = productReader(['sessions', 'list', 7]);
    const unmount = q.mount();
    await settle();
    let first = true;
    refreshQueries(
      q.client,
      ['sessions'],
      () => {},
      () => first,
    );
    // The first subscription is disposed; its replacement on the same client
    // sees a later change while the same read is still running.
    first = false;
    const second = true;
    refreshQueries(
      q.client,
      ['sessions'],
      () => {},
      () => second,
    );
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(1);
    q.answers[0].resolve('before the change');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    q.answers[1].resolve('after the change');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    expect(q.data()).toBe('after the change');
    expect(q.client.getQueryState(['sessions', 'list', 7])?.isInvalidated).toBe(false);
    unmount();
  });

  it('keeps a change for a live earlier requester when a later one is disposed', async () => {
    const q = productReader(['metrics', 'environment', 7]);
    const unmount = q.mount();
    await settle();
    // The status hook's reconciliation (always live) and then a subscription
    // that is disposed before the read settles.
    refreshQueries(q.client, ['metrics'], () => {});
    let subscription = true;
    refreshQueries(
      q.client,
      ['metrics'],
      () => {},
      () => subscription,
    );
    subscription = false;
    q.answers[0].resolve('before the change');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(2);
    q.answers[1].resolve('after the change');
    await settle();
    expect(q.data()).toBe('after the change');
    unmount();
  });

  it('starts no read after every requester is disposed, and leaves the query stale', async () => {
    const q = productReader(['metrics', 'tokens-by-host', 7]);
    const unmount = q.mount();
    await settle();
    let live = true;
    refreshQueries(
      q.client,
      ['metrics'],
      () => {},
      () => live,
    );
    live = false;
    q.answers[0].resolve('before the change');
    await settle();
    expect(q.queryFn).toHaveBeenCalledTimes(1);
    expect(q.data()).toBe('before the change');
    // A later mount or source still sees a stale result, as ordinary stale-query behaviour.
    expect(q.client.getQueryState(['metrics', 'tokens-by-host', 7])?.isInvalidated).toBe(true);
    unmount();
  });
});
