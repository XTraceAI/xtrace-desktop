import {
  replaceEqualDeep,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query';
import { useEffect } from 'react';
import { useData } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { events } from '../data/ipc-names';
import {
  eventPrefixes,
  heardEpoch,
  isTransientIndexStatus,
  reconciledIndexStatus,
  refreshQueries,
} from '../data/subscribe-invalidation';
import { queryKeys } from '../data/query-client';
import { sidebarIndexFacts } from './sidebar-index-status';
export function useAppInfo() {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.appInfo,
    queryFn: () => source.appInfo(),
    enabled: source.kind !== 'preview',
  });
}
export { isTransientIndexStatus };

/**
 * The heard-epoch at which the read behind a status result began, kept with the
 * result object itself. It is attached where the cache accepts a result, so it
 * needs no listener to be present when a read succeeds: whatever mounts later
 * finds it on the cached result.
 */
const beganAt = new WeakMap<object, number>();
const isObject = (value: unknown): value is object => typeof value === 'object' && value !== null;

/**
 * Shares an accepted status with the cached one as the cache always does, and
 * carries the epoch its read began at onto whichever object the cache keeps.
 * The cache calls this only for a result it accepts: a failed or cancelled read
 * never reaches it. A value this module did not read (a manual cache write, a
 * selected value) has no epoch and gains none, so it can never be current. An
 * identical answer normally keeps the cached object; when its read began in a
 * later epoch the same content becomes a new object, so observers see that it
 * is current as of that read.
 */
function acceptStatus(cached: unknown, next: unknown): unknown {
  const shared = replaceEqualDeep(cached, next);
  const began = isObject(next) ? beganAt.get(next) : undefined;
  if (began === undefined) return shared;
  if (shared !== cached) {
    if (isObject(shared)) beganAt.set(shared, began);
    return shared;
  }
  if (!isObject(cached) || beganAt.get(cached) === began) return cached;
  const renewed = { ...cached };
  beganAt.set(renewed, began);
  return renewed;
}

/** The one status read: its key, its command and the preview gate, shared by every reader. */
const nativeIndexQuery = (source: DataSource, client: QueryClient) => ({
  queryKey: queryKeys.nativeIndex,
  queryFn: async () => {
    // Called exactly when a read begins; a read that is joined never calls it.
    const began = heardEpoch.get(client) ?? 0;
    // A copy, so the epoch belongs to this read's answer alone even when a
    // source hands the same object back every time.
    const status = { ...(await source.nativeIndexStatus()) };
    beganAt.set(status, began);
    return status;
  },
  structuralSharing: acceptStatus,
  enabled: source.kind !== 'preview',
});

/** What the sidebar observes: the facts it shows, and the epoch the cached status was read in. */
const observed = (status: NativeIndexStatus) => ({
  facts: sidebarIndexFacts(status),
  began: beganAt.get(status),
});

/**
 * The status as the Shell's sidebar shows it: the same cached read the pages
 * own, only observed. It starts no timer and reconciles no data, so it adds no
 * request beside a page's own: it reads once where no page has, and again only
 * when the status event (or a page's polling) refreshes the shared query. It
 * selects the facts the sidebar shows, so scan progress re-renders nothing,
 * and it subscribes to nothing beyond that query.
 */
export function useNativeIndexObservation() {
  const { source } = useData();
  const client = useQueryClient();
  const query = useQuery({ ...nativeIndexQuery(source, client), select: observed });
  return {
    status: query.data?.facts,
    readFailed: query.isError,
    // Registered listeners mean events are heard from then on, not that the
    // status shown was read since. It is current only when the read behind the
    // cached result began in the epoch the listeners last registered in: a read
    // asked for before them can answer after them, and its answer does not count.
    caughtUp: query.data?.began === (heardEpoch.get(client) ?? 0),
  };
}

export function useNativeIndexStatus() {
  const { source } = useData();
  const client = useQueryClient();
  const query = useQuery({
    ...nativeIndexQuery(source, client),
    // The `ready` event is published once; if it lands before the event
    // listener is registered it is lost, so a transient status is polled
    // until it settles. Settled statuses refresh on the event alone.
    refetchInterval: (query) => (isTransientIndexStatus(query.state.data) ? 1000 : false),
  });
  // A lost `ready` event also carried the invalidation of the data queries:
  // when the status is first seen settled, or seen to settle after polling,
  // invalidate what the event would have. (The counts query may have answered
  // mid-scan just before a first response that is already settled.) What was
  // seen is the client's, shared with the status event, so another route
  // mounting, or the status query seeing a settle whose event already
  // invalidated the data, does not repeat it.
  const observed =
    query.data === undefined
      ? 'none'
      : isTransientIndexStatus(query.data)
        ? 'transient'
        : 'settled';
  useEffect(() => {
    if (observed === 'none') return;
    if (observed === 'settled' && reconciledIndexStatus.get(client) !== 'settled')
      // The status query itself was just observed; only the data it describes is refetched.
      refreshQueries(
        client,
        eventPrefixes[events.nativeIndexStatus].filter(
          (prefix) => prefix !== queryKeys.nativeIndex[0],
        ),
        () => {},
      );
    reconciledIndexStatus.set(client, observed);
  }, [observed, client]);
  return query;
}
export function useDbCounts() {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.dbCounts,
    queryFn: () => source.dbCounts(),
    enabled: source.kind !== 'preview',
  });
}
