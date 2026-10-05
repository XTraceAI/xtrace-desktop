import { QueryClient, QueryObserver } from '@tanstack/react-query';
import { afterEach, expect, it } from 'vitest';
import type { DataSource, Unsubscribe } from './DataSource';
import { events } from './ipc-names';
import { queryKeys } from './query-client';
import {
  heardEpoch,
  subscribeInvalidation,
  type SubscriptionState,
} from './subscribe-invalidation';

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const clients: QueryClient[] = [];
const newClient = () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  clients.push(client);
  return client;
};
afterEach(() => {
  clients.splice(0).forEach((client) => client.clear());
});

/** Only `subscribe` is used by the subscription; registration is answered by the test. */
function source(register: () => Promise<Unsubscribe>) {
  return { kind: 'native', subscribe: register } as unknown as DataSource;
}
/** An active status query that notes the epoch it finds each time a read of it begins. */
function statusReader(client: QueryClient) {
  const began: (number | undefined)[] = [];
  const observer = new QueryObserver(client, {
    queryKey: queryKeys.nativeIndex,
    queryFn: async () => {
      began.push(heardEpoch.get(client));
      return 'status';
    },
  });
  const stop = observer.subscribe(() => {});
  return { began, stop };
}

it('is raised once per complete registration, before the catch-up read begins and before connected is announced', async () => {
  const client = newClient();
  const reader = statusReader(client);
  await settle();
  // Read before anything registered: no epoch yet.
  expect(reader.began).toEqual([undefined]);
  const announced: [SubscriptionState, number | undefined][] = [];
  const stop = subscribeInvalidation(
    source(async () => () => {}),
    client,
    (state) => announced.push([state, heardEpoch.get(client)]),
  );
  await settle();
  // `connected` is announced with the epoch already raised, and the catch-up
  // read it started began in that epoch: no read can begin between the two.
  expect(announced).toEqual([
    ['connecting', undefined],
    ['connected', 1],
  ]);
  expect(reader.began).toEqual([undefined, 1]);
  stop();
  // A later complete registration, as a manual reconnect makes, raises it again.
  const again = subscribeInvalidation(
    source(async () => () => {}),
    client,
    () => {},
  );
  await settle();
  expect(heardEpoch.get(client)).toBe(2);
  expect(reader.began).toEqual([undefined, 1, 2]);
  again();
  reader.stop();
});

it('is not raised by an attempt that fails, whether at once or after some listeners registered', async () => {
  const client = newClient();
  const reader = statusReader(client);
  await settle();
  const states: SubscriptionState[] = [];
  subscribeInvalidation(
    source(async () => {
      throw new Error('listener registration failed');
    }),
    client,
    (state) => states.push(state),
  );
  await settle();
  expect(states).toEqual(['connecting', 'failed']);
  let registered = 0;
  subscribeInvalidation(
    source(async () => {
      // Every listener but the last registers.
      if (++registered === Object.values(events).length) throw new Error('last one failed');
      return () => {};
    }),
    client,
    (state) => states.push(state),
  );
  await settle();
  expect(states).toEqual(['connecting', 'failed', 'connecting', 'failed']);
  expect(heardEpoch.get(client)).toBeUndefined();
  // Nothing was caught up either: the only read is the reader's own first one.
  expect(reader.began).toEqual([undefined]);
  reader.stop();
});

it('belongs to its client: another runtime registering leaves it alone', async () => {
  const first = newClient();
  const second = newClient();
  const stopFirst = subscribeInvalidation(
    source(async () => () => {}),
    first,
    () => {},
  );
  await settle();
  expect(heardEpoch.get(first)).toBe(1);
  expect(heardEpoch.get(second)).toBeUndefined();
  const stopSecond = subscribeInvalidation(
    source(async () => () => {}),
    second,
    () => {},
  );
  await settle();
  expect(heardEpoch.get(first)).toBe(1);
  expect(heardEpoch.get(second)).toBe(1);
  stopFirst();
  stopSecond();
});
