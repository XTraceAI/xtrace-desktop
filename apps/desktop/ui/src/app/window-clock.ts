import { useQueryClient, type Query, type QueryClient, type QueryKey } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';
import { queryKeys } from '../data/query-client';
import { refreshQueries } from '../data/subscribe-invalidation';
import type { TimeRange } from '../kit/TopBar';
import { rangeDays } from './dashboard/present';

/**
 * The oldest a selected-window read shown on a windowed route may grow while
 * the window is visible before it is read again. A native report takes its
 * window as `[now − N days, now)` when its read begins, so a cached answer
 * keeps the window it was read over; nothing else re-reads it when no data
 * event arrives. Coarse on purpose: the reports are day-keyed, and a read
 * every few minutes would only add native work.
 *
 * The bound is on when a read began, not on when its answer is drawn: a slow
 * read, or a timer the system delays, adds to it. A read that fails still
 * counts as a read, so a failing report is tried again at this pace and no
 * faster.
 */
export const windowClockMaxAgeMs = 15 * 60_000;

/**
 * Reads begun within this of each other are read again together once the
 * oldest of them is due, so one route's reports keep one pace rather than
 * refreshing moments apart.
 */
export const windowClockAlignMs = 60_000;

export type WindowClockRoute = {
  /** The matched route pattern, such as `/sessions/:sessionId`. */
  path: string | undefined;
  sessionId: string | undefined;
  /** The PRs page's cached inventory view, whose lifetime list no range filters. */
  inventory: boolean;
  range: TimeRange;
};

/**
 * The selected-window reads one route shows, as key prefixes. Only these are
 * ever read again by the clock, and only while a mounted screen observes them.
 *
 * Deliberately left out: the PRs inventory and a pinned pull request's
 * fixed-window sessions, the transcript (not a query), the Today timer, the
 * index status and Settings. The Sessions list re-reads only the pages the
 * user has loaded, alongside its summary and period read. Every other route
 * is left out too, so the period read catches up when a windowed route is entered.
 */
export function windowClockKeys({
  path,
  sessionId,
  inventory,
  range,
}: WindowClockRoute): QueryKey[] {
  const days = rangeDays[range];
  switch (path) {
    case '/dashboard':
      return [queryKeys.dashboard(days), queryKeys.tokensByHost(days), queryKeys.environment(days)];
    case '/sessions':
      return [queryKeys.dashboard(days), queryKeys.tokensByHost(days), queryKeys.sessions(days)];
    case '/prs':
      // Either confidence mode: only the one the page observes is active.
      return inventory
        ? []
        : [queryKeys.prAnalytics(days, false).slice(0, 3), queryKeys.tokensByHost(days)];
    case '/sessions/:sessionId':
      return sessionId
        ? [
            queryKeys.session(days, sessionId),
            queryKeys.stretches(days, sessionId),
            queryKeys.tokensByHost(days),
          ]
        : [];
    default:
      return [];
  }
}

export type WindowClock = {
  /** The reads the visible route shows; an empty list stops every clock read. */
  target: (keys: readonly QueryKey[]) => void;
  stop: () => void;
};

/**
 * Keeps the targeted reads no older than {@link windowClockMaxAgeMs} while the
 * document is visible. It reads nothing while the document is hidden, and on
 * becoming visible or focused it reads once whatever fell due meanwhile, or
 * whatever was read "after" a wall clock that has since moved backward.
 * Reads go through {@link refreshQueries}, so a read already running is kept
 * and gets one follow-up, never cancelled or restarted. Any read counts, not
 * only the clock's own: one a data event or a screen's mount began resets
 * that query's age, so the clock only reads what nothing else has.
 */
export function startWindowClock(client: QueryClient, maxAgeMs = windowClockMaxAgeMs): WindowClock {
  const cache = client.getQueryCache();
  // When each query's newest read began, overwritten in the order reads
  // begin, so a read begun after a backward jump replaces one "from the
  // future". Kept for the clock's whole life: a screen's mount read begins
  // before the route is targeted and must still count.
  const began = new WeakMap<Query, number>();
  let keys: readonly QueryKey[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  const unsubscribe = cache.subscribe((event) => {
    if (event.type === 'updated' && event.action.type === 'fetch')
      began.set(event.query, Date.now());
  });
  // A read already running when the clock starts began at an unknown moment
  // since its query last settled. It is dated now rather than by that older
  // answer, so it is not followed up as if it were already a full age old;
  // the clock starts in the same commit as the screens whose reads these are.
  const started = Date.now();
  for (const query of cache.getAll())
    if (query.state.fetchStatus !== 'idle') began.set(query, started);
  // A query that settled before the clock started is dated by its last
  // answer, a failure included, so a failed first read is retried at the
  // clock's pace like any other.
  const settledAt = (query: Query) =>
    Math.max(query.state.dataUpdatedAt, query.state.errorUpdatedAt);
  const readAt = (query: Query) => began.get(query) ?? settledAt(query);
  const shown = () => [
    ...new Set(
      keys
        .flatMap((queryKey) => cache.findAll({ queryKey, type: 'active' }))
        .filter((query) => began.has(query) || settledAt(query) > 0),
    ),
  ];
  const check = () => {
    clearTimeout(timer);
    timer = undefined;
    if (stopped || keys.length === 0 || document.visibilityState === 'hidden') return;
    const now = Date.now();
    const queries = shown();
    const ages = queries.map((query) => now - readAt(query));
    // A negative age is a read begun after now: the wall clock moved backward.
    if (ages.some((age) => age < 0 || age >= maxAgeMs)) {
      const due = queries.filter((_, i) => ages[i] < 0 || ages[i] >= maxAgeMs - windowClockAlignMs);
      // Dated now, before any read begins, so a second wake in the same
      // moment finds nothing due.
      for (const query of due) began.set(query, now);
      // A failed read is its query's own error state; nothing more to report.
      refreshQueries(
        client,
        due.map((query) => query.queryKey),
        () => {},
        () => !stopped,
      );
    }
    // Always ahead: whatever was due has just been dated now.
    const next = Math.min(maxAgeMs, ...queries.map((query) => readAt(query) + maxAgeMs - now));
    timer = setTimeout(check, next);
  };
  // Hidden clears the timer; visible or focused looks once. Both arriving
  // together read at most once, since the first dates what it reads.
  document.addEventListener('visibilitychange', check);
  window.addEventListener('focus', check);
  return {
    target(next) {
      keys = next;
      check();
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', check);
      window.removeEventListener('focus', check);
      unsubscribe();
    },
  };
}

/** One clock for the Shell, pointed at the reads its current route shows. */
export function useWindowClock(keys: readonly QueryKey[]) {
  const client = useQueryClient();
  const clock = useRef<WindowClock | undefined>(undefined);
  const signature = JSON.stringify(keys);
  useEffect(() => {
    const started = startWindowClock(client);
    clock.current = started;
    return () => started.stop();
  }, [client]);
  // After the clock above, so a new clock is pointed at the route too.
  useEffect(() => {
    clock.current?.target(JSON.parse(signature) as QueryKey[]);
  }, [client, signature]);
}
