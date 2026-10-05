import { QueryClient } from '@tanstack/react-query';
import type { PullRequestSessionsRequest } from './DataSource';
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { staleTime: 1000, refetchOnWindowFocus: false, retry: false } },
  });
}
export const queryKeys = {
  accountUsage: ['account-usage'],
  dashboard: (windowDays: number) => ['metrics', 'dashboard', windowDays] as const,
  tokensByHost: (windowDays: number) => ['metrics', 'tokens-by-host', windowDays] as const,
  environment: (windowDays: number) => ['metrics', 'environment', windowDays] as const,
  sessions: (windowDays: number) => ['sessions', 'list', windowDays] as const,
  /** One session's row, read through the same list query the table uses. */
  session: (windowDays: number, id: string) => ['sessions', 'one', windowDays, id] as const,
  /**
   * One session's M-09 stretches for one window. Metadata only, so it is
   * cached like the row beside it and invalidated with it by every ingest
   * event (the `sessions` prefix). The transcript is deliberately not a query
   * at all: changing the range re-reads this and never re-reads the text.
   */
  stretches: (windowDays: number, id: string) => ['sessions', 'stretches', windowDays, id] as const,
  appInfo: ['app', 'info'],
  dbCounts: ['database', 'counts'],
  nativeIndex: ['native', 'index'],
  pullRequests: ['prs', 'list'],
  /**
   * The PRs page report for one preset and confidence mode. Under `metrics`,
   * so committed imports, enrichment and reconnects refresh it; a committed
   * pull-request refresh refreshes it by its own prefix.
   */
  prAnalytics: (windowDays: number, confirmedOnly: boolean) =>
    ['metrics', 'pr-analytics', windowDays, confirmedOnly] as const,
  /**
   * One pull request's linked sessions over one report's pinned window; the
   * cursor is the infinite query's page parameter. Under `sessions`, like
   * the list whose rows and measurements it shares.
   */
  prSessions: ({
    repository,
    number,
    windowDays,
    windowEndMs,
    confirmedOnly,
  }: PullRequestSessionsRequest) =>
    ['sessions', 'pr-linked', repository, number, windowDays, windowEndMs, confirmedOnly] as const,
  contentRetention: ['settings', 'content-retention'],
  /** A saved change re-reads every cached Dashboard range, and nothing else. */
  typingSpeed: ['settings', 'typing-speed'],
  /** The prefix of every Dashboard range, for a committed typing-speed change. */
  dashboards: ['metrics', 'dashboard'],
  /** Under `metrics`, so committed-data events refresh it like the Dashboard. */
  today: ['metrics', 'today'],
  /**
   * One lane span's detail. Under `metrics`, so committed imports re-read it;
   * otherwise it is kept, so pointing at the same span again reads nothing. A
   * span that grew has new endpoints, and so a new key. `purges` is the
   * content-purge epoch the read began in: a purge discards every answer read
   * before it (see `content-purges.ts`).
   */
  spanDetail: (purges: number, sessionId: string, startMs: number, endMs: number) =>
    ['metrics', 'span-detail', purges, sessionId, startMs, endMs] as const,
} as const;
