import { QueryClient } from '@tanstack/react-query';
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { staleTime: 1000, refetchOnWindowFocus: false, retry: false } },
  });
}
export const queryKeys = {
  dashboard: (windowDays: number) => ['metrics', 'dashboard', windowDays] as const,
  tokensByHost: (windowDays: number) => ['metrics', 'tokens-by-host', windowDays] as const,
  sessions: ['sessions', 'list'],
  appInfo: ['app', 'info'],
  dbCounts: ['database', 'counts'],
  nativeIndex: ['native', 'index'],
} as const;
