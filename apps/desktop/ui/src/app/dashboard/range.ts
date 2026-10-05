import { useQuery } from '@tanstack/react-query';
import { useOutletContext } from 'react-router';
import { useData } from '../../data/DataProvider';
import { queryKeys } from '../../data/query-client';
import type { TimeRange } from '../../kit/TopBar';
import { rangeDays } from './present';

/** The Shell owns one selected range, shared by its TopBar, Sidebar and routed pages. */
export type ShellOutletContext = { range: TimeRange };

export function useSelectedRange(): TimeRange {
  return useOutletContext<ShellOutletContext | undefined>()?.range ?? '7d';
}

export function useDashboardReport(range: TimeRange) {
  const { source } = useData();
  const days = rangeDays[range];
  return useQuery({
    queryKey: queryKeys.dashboard(days),
    queryFn: () => source.dashboard(days),
    enabled: source.kind !== 'preview',
  });
}

export function useTokensByHost(range: TimeRange) {
  const { source } = useData();
  const days = rangeDays[range];
  return useQuery({
    queryKey: queryKeys.tokensByHost(days),
    queryFn: () => source.tokensByHost(days),
    enabled: source.kind !== 'preview',
  });
}

/** M-17 usage beside configured facts; keyed by range, refreshed with every `metrics` query. */
export function useEnvironmentReport(range: TimeRange) {
  const { source } = useData();
  const days = rangeDays[range];
  return useQuery({
    queryKey: queryKeys.environment(days),
    queryFn: () => source.environment(days),
    enabled: source.kind !== 'preview',
  });
}
