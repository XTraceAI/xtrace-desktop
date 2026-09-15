import { QueryClient } from '@tanstack/react-query';
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { staleTime: 1000, refetchOnWindowFocus: false, retry: false } },
  });
}
export const queryKeys = {
  appInfo: ['app', 'info'],
  dbCounts: ['database', 'counts'],
  nativeIndex: ['native', 'index'],
} as const;
