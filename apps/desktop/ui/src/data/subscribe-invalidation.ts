import type { Query, QueryClient, QueryKey } from '@tanstack/react-query';
import type { DataSource, Unsubscribe } from './DataSource';
import type { NativeIndexStatus } from './generated/NativeIndexStatus';
import { discardStoredWords } from './content-purges';
import { events, type DataEvent } from './ipc-names';
import { queryKeys } from './query-client';

const ingest = ['database', 'metrics', 'sessions', 'hosts', 'compactions'];
export const eventPrefixes: Record<DataEvent, readonly string[]> = {
  [events.importReceived]: ingest,
  [events.backfillProgress]: ingest,
  [events.turnCompleted]: ingest,
  [events.hostConnected]: ['app', ...ingest],
  [events.fireReceived]: ['fires', 'rules'],
  [events.prsRefreshed]: ['prs', 'gh'],
  [events.nativeIndexStatus]: ['native', ...ingest],
  // Published only after a committed purge: views that can show stored content.
  [events.contentPurged]: ['sessions', 'metrics'],
  // Only the narrower keys below: no top-level key is wholly its.
  [events.sessionCreationsChanged]: [],
};

/**
 * The stored pull-request inventory: committed indexing can add a session's
 * link, or the first link to a pull request, so every event that commits
 * sessions re-reads it. Only the stored list; nothing is refreshed from GitHub.
 */
const inventory: readonly QueryKey[] = [queryKeys.pullRequests];

/**
 * Narrower key prefixes an event also refreshes, under a top-level key it does
 * not otherwise own. A committed pull-request refresh changes the stored
 * titles the Sessions list, the exact session row and a pull request's
 * linked-session drilldown carry on their links, and the cached facts the PRs
 * report is computed from; their measurements, the session stretches and the
 * Environment report do not change, so only those readers are read again.
 * Committed ingest can add links, so it re-reads the stored inventory.
 */
export const eventKeyPrefixes: Partial<Record<DataEvent, readonly QueryKey[]>> = {
  [events.importReceived]: inventory,
  [events.backfillProgress]: inventory,
  [events.turnCompleted]: inventory,
  [events.hostConnected]: inventory,
  // Only a settled status: initial-scan progress moves the status alone.
  [events.nativeIndexStatus]: inventory,
  [events.prsRefreshed]: [
    queryKeys.sessions(7).slice(0, 2),
    queryKeys.session(7, '').slice(0, 2),
    queryKeys.prAnalytics(7, true).slice(0, 2),
    queryKeys
      .prSessions({
        repository: '',
        number: 0,
        confirmedOnly: true,
        windowDays: 7,
        windowEndMs: 0,
      })
      .slice(0, 2),
  ],
  /**
   * A background pass committed a new sub-session relation, or withheld one as
   * conflicted: the parent the Sessions list, the exact session row and every
   * Dashboard range's lane context carry. These relations also change Human
   * eligibility, so refresh Human totals and linked-session Human counts.
   */
  [events.sessionCreationsChanged]: [
    queryKeys.sessions(7).slice(0, 2),
    queryKeys.session(7, '').slice(0, 2),
    queryKeys.dashboards,
    queryKeys.today,
    queryKeys.prAnalytics(7, true).slice(0, 2),
    queryKeys
      .prSessions({ repository: '', number: 0, confirmedOnly: true, windowDays: 7, windowEndMs: 0 })
      .slice(0, 2),
  ],
};

/** A status still moving on its own: the initial scan, or interpreter discovery. */
export const isTransientIndexStatus = (status: NativeIndexStatus | undefined) =>
  status !== undefined &&
  (status.phase.phase === 'scanning' || status.python.state === 'resolving');

/**
 * The index status each QueryClient's data queries were last reconciled with:
 * set by a status event (its invalidation is then pending) or by the status
 * query. It belongs to the client, so the routes that read the status share it
 * and a remount does not repeat the reconciliation.
 */
export const reconciledIndexStatus = new WeakMap<QueryClient, 'transient' | 'settled'>();

/**
 * How many times each QueryClient's listeners have all registered. `connected()`
 * raises it before its catch-up reads begin, so a read that began at the current
 * count began after every listener registered, and one that began at an earlier
 * count did not. `connected` itself still means only that every event is heard.
 */
export const heardEpoch = new WeakMap<QueryClient, number>();

/** Only a status whose phase and interpreter state are readable is trusted to narrow an event. */
const indexStatusPayload = (payload: unknown) => {
  const status = payload as { phase?: { phase?: unknown }; python?: { state?: unknown } } | null;
  return typeof status?.phase?.phase === 'string' && typeof status.python?.state === 'string'
    ? (payload as NativeIndexStatus)
    : undefined;
};

/**
 * A change a query's running read may predate, owed once that read settles.
 * Every requester's liveness is kept, so a disposed source cannot drop a
 * change its live replacement also asked for.
 */
type Owed = { live: Set<() => boolean>; onError: () => void };
const owed = new WeakMap<Query, Owed>();

/**
 * Settles what is owed to `query` after the read it is running, whether that
 * read succeeds or fails: the query is marked stale again, active or not, and
 * while any requester is live its active observers read once more. Changes
 * arriving before then share this one follow-up, which starts after all of
 * them; a read that starts meanwhile carries the obligation instead.
 */
function followUp(client: QueryClient, query: Query, onError: () => void, live: () => boolean) {
  const pending = owed.get(query);
  if (pending) {
    pending.live.add(live);
    pending.onError = onError;
    return;
  }
  const debt: Owed = { live: new Set([live]), onError };
  owed.set(query, debt);
  const settle = () => {
    if (client.getQueryCache().get(query.queryHash) !== query) {
      owed.delete(query);
      return;
    }
    if (query.state.fetchStatus === 'fetching') {
      query.fetch(undefined, { cancelRefetch: false }).then(settle, settle);
      return;
    }
    owed.delete(query);
    const { queryKey } = query;
    void client
      .invalidateQueries({ queryKey, exact: true, refetchType: 'none' })
      .then(() =>
        [...debt.live].some((isLive) => isLive())
          ? client.refetchQueries(
              { queryKey, exact: true, type: 'active' },
              { cancelRefetch: false },
            )
          : undefined,
      )
      .catch(debt.onError);
  };
  query.fetch(undefined, { cancelRefetch: false }).then(settle, settle);
}

/**
 * Marks each prefix stale and reads its active queries again, never cancelling
 * a read already running: a cancelled read's native work is not stopped, so a
 * restart only adds work behind it. A query that is reading when the change
 * arrives keeps that read and gets one follow-up read after it, so every query
 * has at most one read running and one waiting, and the last read starts
 * after the last change. A prefix is a top-level key or a longer key prefix,
 * such as every range of one report.
 */
export function refreshQueries(
  client: QueryClient,
  prefixes: Iterable<string | QueryKey>,
  onError: () => void,
  live: () => boolean = () => true,
) {
  const cache = client.getQueryCache();
  for (const prefix of prefixes) {
    const queryKey = typeof prefix === 'string' ? [prefix] : prefix;
    const running = cache.findAll({ queryKey, fetchStatus: 'fetching' });
    void client
      .invalidateQueries({ queryKey, refetchType: 'none' })
      .then(() =>
        live()
          ? client.refetchQueries({ queryKey, type: 'active' }, { cancelRefetch: false })
          : undefined,
      )
      .catch(onError);
    for (const query of running) followUp(client, query, onError, live);
  }
}

/** Where one subscription attempt stands; only `connected` means every event is heard. */
export type SubscriptionState = 'connecting' | 'connected' | 'failed';

/** How long every listener has to register before the attempt is abandoned. */
export const registrationTimeoutMs = 10_000;

/**
 * Everything an event can change, plus the retention mode a purge elsewhere may
 * have followed. Each attempt reads these once it hears every event, since
 * events sent while no listener was registered (before the first attempt
 * completed, or between attempts) are lost. The pull-request list is re-read from
 * storage; nothing is refreshed from GitHub, re-indexed or imported.
 */
export const catchUpPrefixes: readonly (string | QueryKey)[] = [
  ...new Set(Object.values(eventPrefixes).flat()),
  queryKeys.contentRetention,
];

/**
 * One attempt to hear every data event. It is `connected` only once all of the
 * listeners have registered; a synchronous or asynchronous failure, or the
 * registration outlasting {@link registrationTimeoutMs}, makes it `failed`, and
 * a failed attempt stops everything it registered. It never retries: a new
 * attempt is the caller's. After stopping, its callbacks are inert and a
 * listener registered late is unsubscribed at once, so an attempt that is
 * replaced cannot report, refresh or keep an effective listener. A complete
 * registration is followed by one read of what the events could have changed
 * while nothing was listening; an attempt stopped before that reads nothing.
 *
 * Stopping only ends the attempt: the queries it refreshed are left running.
 */
export function subscribeInvalidation(
  source: DataSource,
  client: QueryClient,
  onState: (state: SubscriptionState) => void,
): Unsubscribe {
  let active = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  // Keyed by the serialized prefix, so a top-level key and a longer one
  // coalesce alike.
  const pending = new Map<string, string | QueryKey>();
  const stops = new Set<Unsubscribe>();
  const required = Object.values(events);
  let registered = 0;
  const stop = (unlisten: Unsubscribe) => {
    try {
      unlisten();
    } catch {
      // Its callback is already inert. Only a failing attempt releases while it
      // is current, and that attempt already shows as failed; a replaced one
      // must not change the state of the attempt that replaced it.
    }
  };
  const halt = () => {
    active = false;
    clearTimeout(timer);
    clearTimeout(deadline);
    timer = undefined;
    pending.clear();
    const handles = [...stops];
    stops.clear();
    return handles;
  };
  const fail = () => {
    if (!active) return;
    const handles = halt();
    onState('failed');
    handles.forEach(stop);
  };
  const listener = (event: DataEvent) => (payload?: unknown) => {
    if (!active) return;
    // Not coalesced: an answer holding deleted words must not wait 500ms.
    if (event === events.contentPurged) discardStoredWords(client);
    let prefixes = eventPrefixes[event];
    let keyPrefixes = eventKeyPrefixes[event] ?? [];
    const status = event === events.nativeIndexStatus ? indexStatusPayload(payload) : undefined;
    // Initial-scan progress moves the status alone: the scan's data is
    // read once, when `ready` or `stopped` settles it.
    if (status?.phase.phase === 'scanning') {
      prefixes = ['native'];
      keyPrefixes = [];
    } else if (status)
      reconciledIndexStatus.set(client, isTransientIndexStatus(status) ? 'transient' : 'settled');
    for (const prefix of prefixes) pending.set(JSON.stringify([prefix]), prefix);
    for (const prefix of keyPrefixes) pending.set(JSON.stringify(prefix), prefix);
    // Coalesce from the first event, so continuous imports cannot starve refresh.
    timer ??= setTimeout(() => {
      timer = undefined;
      if (!active) return;
      const prefixes = [...pending.values()];
      pending.clear();
      refreshQueries(client, prefixes, fail, () => active);
    }, 500);
  };
  const connected = () => {
    clearTimeout(deadline);
    // The catch-up covers every prefix, so events heard while registering are
    // not refreshed again after it: a screen opened on `connected` reads once.
    clearTimeout(timer);
    timer = undefined;
    pending.clear();
    // Read before announcing: whatever opens on `connected` reads after every
    // listener, and what was already open re-reads what it may have missed.
    heardEpoch.set(client, (heardEpoch.get(client) ?? 0) + 1);
    refreshQueries(client, catchUpPrefixes, fail, () => active);
    onState('connected');
  };
  onState('connecting');
  const deadline = setTimeout(fail, registrationTimeoutMs);
  for (const event of required) {
    if (!active) break;
    let registration: Promise<Unsubscribe>;
    try {
      // An adapter that answers without a promise is held to the same rules.
      registration = Promise.resolve(source.subscribe(event, listener(event)));
    } catch {
      fail();
      break;
    }
    registration.then(
      (unlisten) => {
        if (!active) return stop(unlisten);
        stops.add(unlisten);
        if (++registered === required.length) connected();
      },
      () => fail(),
    );
  }
  return () => {
    // Replaced or unmounted: nothing is reported, and every handle is released.
    halt().forEach(stop);
  };
}
