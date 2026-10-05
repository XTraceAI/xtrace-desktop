import { QueryClientProvider } from '@tanstack/react-query';
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { StartupSplash } from '../kit/StartupSplash';
import type { DataSource } from './DataSource';
import { createPrRefreshOperation, type PrRefreshOperation } from './pr-refresh-operation';
import { createQueryClient } from './query-client';
import {
  refreshQueries,
  subscribeInvalidation,
  type SubscriptionState,
} from './subscribe-invalidation';

/** The runtime's live updates: which attempt is current, where it stands, and a manual retry. */
export type LiveUpdates = {
  state: SubscriptionState;
  /** 0 for the runtime's own first attempt; each manual reconnect counts one more. */
  attempt: number;
  /** Starts a new attempt, only when the current one has failed. */
  reconnect: () => void;
};
const DataContext = createContext<{
  source: DataSource;
  prRefresh: PrRefreshOperation;
} | null>(null);
// Apart from the data context, so a change of connection state re-renders what
// shows it and not every screen that reads data.
const LiveUpdatesContext = createContext<LiveUpdates | null>(null);
type ProviderProps = { source: DataSource; children: ReactNode };
export function DataProvider({ source, children }: ProviderProps) {
  const [identity, setIdentity] = useState({ source, generation: 0 });
  if (identity.source !== source) {
    // Reset the entire runtime: query observers retain their original client.
    setIdentity({ source, generation: identity.generation + 1 });
    return null;
  }
  return (
    <SourceRuntime key={identity.generation} source={source}>
      {children}
    </SourceRuntime>
  );
}
function SourceRuntime({ source, children }: ProviderProps) {
  const [client] = useState(createQueryClient);
  // One manual pull-request refresh per runtime, owned here so that no screen
  // unmounting (a range change, another route) loses the batch or its Cancel.
  const [prRefresh] = useState(() => createPrRefreshOperation(source, client));
  const [subscription, setSubscription] = useState<{
    attempt: number;
    state: SubscriptionState;
  }>({ attempt: 0, state: 'connecting' });
  const { attempt, state } = subscription;
  useEffect(() => {
    prRefresh.attach();
    return () => prRefresh.detach();
  }, [prRefresh]);
  // Only the runtime ending (unmounting, or its source being replaced) stops
  // its reads; a reconnect leaves every read, the pull-request batch and any
  // open transcript alone.
  useEffect(() => () => void client.cancelQueries(), [client]);
  // One app-owned count refresh, including browser focus (which the query
  // library does not listen for). A visibility/focus pair shares one wake.
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const wake = () => {
      if (document.visibilityState === 'hidden') return;
      timer ??= setTimeout(() => {
        timer = undefined;
        if (live && document.visibilityState !== 'hidden')
          refreshQueries(
            client,
            ['compactions'],
            () => {},
            () => live,
          );
      }, 0);
    };
    window.addEventListener('focus', wake);
    document.addEventListener('visibilitychange', wake);
    return () => {
      live = false;
      clearTimeout(timer);
      window.removeEventListener('focus', wake);
      document.removeEventListener('visibilitychange', wake);
    };
  }, [client]);
  useEffect(
    () =>
      subscribeInvalidation(
        source,
        client,
        // An attempt's answer is kept only while that attempt is current.
        (state) =>
          setSubscription((current) =>
            current.attempt === attempt && current.state !== state ? { attempt, state } : current,
          ),
      ),
    [source, client, attempt],
  );
  // Same function for the runtime's life. The updater admits one new attempt
  // per failure, so a second click before the first re-renders adds nothing.
  const [reconnect] = useState(
    () => () =>
      setSubscription((current) =>
        current.state === 'failed'
          ? { attempt: current.attempt + 1, state: 'connecting' }
          : current,
      ),
  );
  // The runtime's screens first mount once its first attempt has settled, so
  // their first reads begin after every listener was registered (a change made
  // meanwhile is then read, not missed). Settled either way, the fence stays
  // open: a failed first attempt opens the app with its notice and Reconnect,
  // and a reconnect never closes it again.
  const open = attempt > 0 || state !== 'connecting';
  const data = useMemo(() => ({ source, prRefresh }), [source, prRefresh]);
  const live = useMemo(() => ({ state, attempt, reconnect }), [state, attempt, reconnect]);
  return (
    <DataContext.Provider value={data}>
      <LiveUpdatesContext.Provider value={live}>
        <QueryClientProvider client={client}>
          {open ? children : <StartupSplash>Connecting live updates…</StartupSplash>}
        </QueryClientProvider>
      </LiveUpdatesContext.Provider>
    </DataContext.Provider>
  );
}
/** The runtime's manual refresh and its current state. */
export function usePrRefresh() {
  const { prRefresh } = useData();
  const state = useSyncExternalStore(prRefresh.subscribe, prRefresh.getState);
  return [state, prRefresh] as const;
}
/** Whether data events are heard, and the manual reconnect when they are not. */
export function useLiveUpdates() {
  const live = useContext(LiveUpdatesContext);
  if (!live) throw new Error('useLiveUpdates requires DataProvider.');
  return live;
}
export function useData() {
  const data = useContext(DataContext);
  if (!data) throw new Error('useData requires DataProvider.');
  return data;
}
