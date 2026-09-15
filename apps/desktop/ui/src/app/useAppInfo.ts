import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';
import { useData } from '../data/DataProvider';
import { events } from '../data/ipc-names';
import { eventPrefixes } from '../data/subscribe-invalidation';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { queryKeys } from '../data/query-client';
export function useAppInfo() {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.appInfo,
    queryFn: () => source.appInfo(),
    enabled: source.kind !== 'preview',
  });
}
/** A status still moving on its own: the initial scan, or interpreter discovery. */
export const isTransientIndexStatus = (status: NativeIndexStatus | undefined) =>
  status !== undefined &&
  (status.phase.phase === 'scanning' || status.python.state === 'resolving');

export function useNativeIndexStatus() {
  const { source } = useData();
  const client = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.nativeIndex,
    queryFn: () => source.nativeIndexStatus(),
    enabled: source.kind !== 'preview',
    // The `ready` event is published once; if it lands before the event
    // listener is registered it is lost, so a transient status is polled
    // until it settles. Settled statuses refresh on the event alone.
    refetchInterval: (query) => (isTransientIndexStatus(query.state.data) ? 1000 : false),
  });
  // A lost `ready` event also carried the invalidation of the data queries:
  // when polling sees the status settle, invalidate what the event would have.
  const wasTransient = useRef(false);
  const transient = isTransientIndexStatus(query.data);
  useEffect(() => {
    if (wasTransient.current && !transient)
      for (const prefix of eventPrefixes[events.nativeIndexStatus])
        void client.invalidateQueries({ queryKey: [prefix] });
    wasTransient.current = transient;
  }, [transient, client]);
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
