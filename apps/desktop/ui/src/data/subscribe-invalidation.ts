import type { QueryClient } from '@tanstack/react-query';
import type { DataSource, Unsubscribe } from './DataSource';
import { events, type DataEvent } from './ipc-names';

const ingest = ['database', 'metrics', 'sessions', 'hosts'];
export const eventPrefixes: Record<DataEvent, readonly string[]> = {
  [events.importReceived]: ingest,
  [events.backfillProgress]: ingest,
  [events.turnCompleted]: ingest,
  [events.hostConnected]: ['app', ...ingest],
  [events.fireReceived]: ['fires', 'rules'],
  [events.prsRefreshed]: ['prs', 'gh'],
  [events.nativeIndexStatus]: ['native', ...ingest],
};

export function subscribeInvalidation(
  source: DataSource,
  client: QueryClient,
  onError: () => void,
): Unsubscribe {
  let active = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const pending = new Set<string>();
  const stops = new Set<Unsubscribe>();
  const report = () => {
    if (active) onError();
  };
  const stop = (unlisten: Unsubscribe) => {
    try {
      unlisten();
    } catch {
      report();
    }
  };
  for (const event of Object.values(events)) {
    try {
      void source
        .subscribe(event, () => {
          if (!active) return;
          for (const prefix of eventPrefixes[event]) pending.add(prefix);
          // Coalesce from the first event, so continuous imports cannot starve refresh.
          timer ??= setTimeout(() => {
            timer = undefined;
            if (!active) return;
            const prefixes = [...pending];
            pending.clear();
            for (const prefix of prefixes)
              void client.invalidateQueries({ queryKey: [prefix] }).catch(report);
          }, 500);
        })
        .then((unlisten) => {
          if (active) stops.add(unlisten);
          else stop(unlisten);
        }, report);
    } catch {
      report();
    }
  }
  return () => {
    active = false;
    clearTimeout(timer);
    pending.clear();
    stops.forEach(stop);
    stops.clear();
  };
}
