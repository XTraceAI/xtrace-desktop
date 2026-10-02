import { isTauri } from '@tauri-apps/api/core';
import type { DataSource } from './DataSource';
import { TauriDataSource } from './TauriDataSource';

export async function createDataSource(): Promise<DataSource> {
  // Native fixture mode belongs to Rust; a Vite flag must never replace real IPC.
  if (isTauri()) return new TauriDataSource();
  if (import.meta.env.DEV && import.meta.env.VITE_XTRACE_FIXTURE) {
    const { loadFixtureDataSource } = await import('./FixtureDataSource');
    return loadFixtureDataSource(import.meta.env.VITE_XTRACE_FIXTURE);
  }
  const unavailable = async (): Promise<never> => {
    throw new Error('Native data is unavailable in browser preview.');
  };
  return {
    kind: 'preview',
    dashboard: unavailable,
    tokensByHost: unavailable,
    sessionsList: unavailable,
    appInfo: unavailable,
    dbCounts: unavailable,
    nativeIndexStatus: unavailable,
    subscribe: async () => () => {},
  };
}
