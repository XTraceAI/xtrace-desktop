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
  sessions: (windowDays: number) => ['sessions', 'list', windowDays] as const,
  /** One session's row, read through the same list query the table uses. */
  session: (windowDays: number, id: string) => ['sessions', 'one', windowDays, id] as const,
  /** The menu-bar list's first page of recently active sessions. */
  trayRecent: (windowDays: number) => ['sessions', 'tray-recent', windowDays] as const,
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
  /** The grading skills: built in, then added; a change re-reads them. */
  gradingSkills: ['grading', 'skills'],
  /**
   * The installed skills for the Add a skill dialog. Under `grading`, so an
   * added or removed skill re-reads which rows are on Scores.
   */
  gradingInstalled: ['grading', 'installed'],
  /**
   * The Scores page's answer. Under `grading`, so every committed status
   * change and a content purge re-read it; it reads the project list fresh.
   */
  gradingOverview: ['grading', 'overview'],
  /** The recent grading runs; every committed status change re-reads them. */
  gradingRuns: ['grading', 'runs'],
  /** The automatic pull-request check's status; its own event re-reads it. */
  prAutoCheck: ['prs-auto', 'status'],
  /** A saved change re-reads every cached Dashboard range, and nothing else. */
  typingSpeed: ['settings', 'typing-speed'],
  /** Like the typing speed: a saved change re-reads every cached Dashboard range. */
  humanBreak: ['settings', 'human-break'],
  /** The telemetry state; every change answers with the state read back. */
  telemetry: ['settings', 'telemetry'],
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
  /**
   * One day's sessions in one Dashboard report: its range, its window's end
   * and the day. Under `metrics`, so committed data re-reads it; otherwise it
   * is kept, so opening the same day of the same report again reads nothing.
   * Every new report has a new window end, and so a new key; the key also
   * names the zone the report was read in, as the request does. `purges` is
   * the content-purge epoch the read began in, as for a span's detail: the
   * list carries saved titles, which "Delete stored content" removes.
   */
  daySessions: (
    purges: number,
    windowDays: number,
    windowEndMs: number,
    timezone: string,
    date: string,
  ) => ['metrics', 'day-sessions', purges, windowDays, windowEndMs, timezone, date] as const,
} as const;
