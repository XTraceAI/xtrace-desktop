import { useQuery } from '@tanstack/react-query';
import { useData } from '../data/DataProvider';
import { queryKeys } from '../data/query-client';
export function useAppInfo() {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.appInfo,
    queryFn: () => source.appInfo(),
    enabled: source.kind !== 'preview',
  });
}
export function useNativeIndexStatus() {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.nativeIndex,
    queryFn: () => source.nativeIndexStatus(),
    enabled: source.kind !== 'preview',
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
