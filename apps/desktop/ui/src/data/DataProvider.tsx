import { QueryClientProvider } from '@tanstack/react-query';
import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import type { DataSource } from './DataSource';
import { createQueryClient } from './query-client';
import { subscribeInvalidation } from './subscribe-invalidation';

const DataContext = createContext<{ source: DataSource; eventError: boolean } | null>(null);
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
  const [eventError, setEventError] = useState(false);
  useEffect(() => {
    const stop = subscribeInvalidation(source, client, () => setEventError(true));
    return () => {
      stop();
      void client.cancelQueries();
    };
  }, [source, client]);
  return (
    <DataContext.Provider value={{ source, eventError }}>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </DataContext.Provider>
  );
}
export function useData() {
  const data = useContext(DataContext);
  if (!data) throw new Error('useData requires DataProvider.');
  return data;
}
