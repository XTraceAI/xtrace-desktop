import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useData } from '../data/DataProvider';
import type { LiveSessionSnapshot } from '../data/generated/LiveSessionSnapshot';
import { HOST_NAMES } from '../kit/hosts';

export type LiveSessionStatus = LiveSessionSnapshot['states'][number]['status'];
export const LIVE_SESSION_LIMIT = 16;
export const LIVE_SESSION_POLL_MS = 2000;

export const LIVE_SESSION_HOSTS = {
  claude: { label: HOST_NAMES.claude, source: `${HOST_NAMES.claude} runtime` },
  codex: { label: HOST_NAMES.codex, source: `${HOST_NAMES.codex} desktop runtime` },
};
export type LiveSessionHost = keyof typeof LIVE_SESSION_HOSTS;
export function isLiveSessionHost(host: string): host is LiveSessionHost {
  return Object.hasOwn(LIVE_SESSION_HOSTS, host);
}

/** Priority order is supplied by the page, with recent chats first on Sessions. */
export function liveSessionIds(ids: readonly string[]): string[] {
  return [...new Set(ids)].slice(0, LIVE_SESSION_LIMIT);
}

const statuses = new Set<LiveSessionStatus>([
  'running',
  'waiting_approval',
  'waiting_input',
  'idle',
  'unknown',
]);
/** One native-issued lease and at most one read chain, including replaced selections. */
export function useLiveSessionStatus(
  scope: string,
  ids: readonly string[],
  { keepResolved = false }: { keepResolved?: boolean } = {},
) {
  const { source } = useData();
  const controls = source.liveSessions;
  // Choose by page priority first; order alone must not reopen subscriptions.
  const request = JSON.stringify(liveSessionIds(ids).sort());
  const selected = useMemo<string[]>(() => JSON.parse(request), [request]);
  const key = JSON.stringify([scope, request]);
  const [state, setState] = useState<{
    key: string;
    scope: string;
    controls: typeof controls;
    found: ReadonlyMap<string, LiveSessionStatus>;
  }>({ key: '', scope, controls, found: new Map() });
  const inflight = useRef<Promise<void> | null>(null);

  useEffect(() => {
    // Dashboard rows keep their last observed state when they scroll away.
    // New scopes/sources start empty; polling still names only visible IDs.
    setState((previous) =>
      keepResolved && previous.scope === scope && previous.controls === controls
        ? previous
        : { key, scope, controls, found: new Map() },
    );
    if (!controls || selected.length === 0) return;
    let viewId: string | null = null;
    let active = true;
    const release = (token: string) => {
      void controls.release(token).catch(() => {});
    };
    const settle = (answer: ReadonlyMap<string, LiveSessionStatus>) => {
      setState((previous) => {
        const found =
          keepResolved && previous.scope === scope && previous.controls === controls
            ? new Map(previous.found)
            : new Map<string, LiveSessionStatus>();
        // A missing/failed state is unknown, never the old Running state.
        for (const id of selected) found.delete(id);
        for (const [id, status] of answer) found.set(id, status);
        return { key, scope, controls, found };
      });
    };
    const poll = () => {
      if (!active || inflight.current) return;
      const read = (async () => {
        let token = viewId;
        try {
          if (token === null) {
            // Registration has no subscriptions. A replaced registration must
            // release its issued token without ever asking about chat IDs.
            const registration = await controls.read([], null);
            if (typeof registration.view_id !== 'string' || !registration.view_id) {
              throw new Error('Live session registration returned no lease');
            }
            token = registration.view_id;
            if (!active) return;
            viewId = token;
          }
          const snapshot = await controls.read(selected, token);
          if (!active) return;
          if (snapshot.view_id !== token) throw new Error('Live session lease changed');
          const found = new Map<string, LiveSessionStatus>();
          for (const row of snapshot.states) {
            if (selected.includes(row.id)) {
              found.set(row.id, statuses.has(row.status) ? row.status : 'unknown');
            }
          }
          settle(found);
        } catch {
          if (active) {
            settle(new Map());
            viewId = null;
            if (token !== null) release(token);
            // The next timer poll explicitly registers a fresh empty lease.
            // A rejected token is never reused or created by this UI.
          }
        } finally {
          // This chain's old token only; cleanup never names a successor.
          if (!active && token !== null) release(token);
        }
      })();
      inflight.current = read;
      void read.then(() => {
        if (inflight.current === read) inflight.current = null;
      });
    };
    if (inflight.current) void inflight.current.then(poll);
    else poll();
    const timer = setInterval(poll, LIVE_SESSION_POLL_MS);
    return () => {
      active = false;
      clearInterval(timer);
      if (viewId !== null) release(viewId);
    };
  }, [controls, key, selected, scope, keepResolved]);

  return useCallback(
    (id: string): LiveSessionStatus | undefined =>
      controls
        ? (((
            keepResolved ? state.scope === scope && state.controls === controls : state.key === key
          )
            ? state.found.get(id)
            : undefined) ?? 'unknown')
        : undefined,
    [controls, key, state, scope, keepResolved],
  );
}
