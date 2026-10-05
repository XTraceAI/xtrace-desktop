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

export const purgeEpoch = (client: QueryClient) => client.getQueryData<number>(purgeKey) ?? 0;

/**
 * A committed "Delete stored content": span details read before it may hold
 * words that are now deleted, and metric reads no longer wait for the purge,
 * so a read that began before it can answer after. Every span detail is keyed
 * by the epoch it began in; raising the epoch stops those reads and drops
 * their answers, cached or still to come, and the next open reads again.
 */
export function discardStoredWords(client: QueryClient) {
  client.setQueryData<number>(purgeKey, purgeEpoch(client) + 1);
  void client.cancelQueries({ queryKey: spanDetails });
  client.removeQueries({ queryKey: spanDetails });
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
