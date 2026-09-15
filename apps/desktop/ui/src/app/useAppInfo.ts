import { useQuery } from '@tanstack/react-query';
import { useData } from '../data/DataProvider';
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
  return useQuery({
    queryKey: queryKeys.nativeIndex,
    queryFn: () => source.nativeIndexStatus(),
    enabled: source.kind !== 'preview',
    // The `ready` event is published once; if it lands before the event
    // listener is registered it is lost, so a transient status is polled
    // until it settles. Settled statuses refresh on the event alone.
    refetchInterval: (query) => (isTransientIndexStatus(query.state.data) ? 1000 : false),
  });
}
export function useDbCounts() {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.dbCounts,
    queryFn: () => source.dbCounts(),
    enabled: source.kind !== 'preview',
  });
}
