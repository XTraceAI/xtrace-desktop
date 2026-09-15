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
  // when the status is first seen settled, or seen to settle after polling,
  // invalidate what the event would have. (The counts query may have answered
  // mid-scan just before a first response that is already settled.)
  const seen = useRef<'none' | 'transient' | 'settled'>('none');
  const observed =
    query.data === undefined
      ? 'none'
      : isTransientIndexStatus(query.data)
        ? 'transient'
        : 'settled';
  useEffect(() => {
    if (observed === 'none') return;
    if (observed === 'settled' && seen.current !== 'settled')
      for (const prefix of eventPrefixes[events.nativeIndexStatus])
        // The status query itself was just observed; only the data it describes is refetched.
        if (prefix !== queryKeys.nativeIndex[0])
          void client.invalidateQueries({ queryKey: [prefix] });
    seen.current = observed;
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
