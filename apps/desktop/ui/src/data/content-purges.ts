import type { QueryClient } from '@tanstack/react-query';
import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useSyncExternalStore } from 'react';

/**
 * How many committed content purges this QueryClient has heard of. Kept in
 * the client's own cache, never read from a source and never refetched, so
 * no invalidation can reset it.
 */
const purgeKey = ['content-purges'] as const;

/** The prefix of every lane span's detail. */
const spanDetails = ['metrics', 'span-detail'] as const;
/** The prefix of every day card's sessions, which carry saved titles. */
const daySessions = ['metrics', 'day-sessions'] as const;

export const purgeEpoch = (client: QueryClient) => client.getQueryData<number>(purgeKey) ?? 0;

/**
 * A committed "Delete stored content": span details read before it may hold
 * words that are now deleted, and a day card's sessions saved titles, and
 * metric reads no longer wait for the purge, so a read that began before it
 * can answer after. Both are keyed by the epoch they began in; raising the
 * epoch stops those reads and drops their answers, cached or still to come,
 * and the next open reads again.
 */
export function discardStoredWords(client: QueryClient) {
  client.setQueryData<number>(purgeKey, purgeEpoch(client) + 1);
  for (const queryKey of [spanDetails, daySessions]) {
    void client.cancelQueries({ queryKey });
    client.removeQueries({ queryKey });
  }
}

/** The current purge epoch, re-rendering when a purge raises it. */
export function usePurgeEpoch() {
  const client = useQueryClient();
  const subscribe = useCallback(
    (notify: () => void) => client.getQueryCache().subscribe(notify),
    [client],
  );
  return useSyncExternalStore(subscribe, () => purgeEpoch(client));
}
