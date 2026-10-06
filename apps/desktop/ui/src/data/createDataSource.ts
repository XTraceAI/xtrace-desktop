import { invoke, isTauri } from '@tauri-apps/api/core';
import type { DataSource } from './DataSource';
import { TauriDataSource } from './TauriDataSource';

export async function createDataSource(): Promise<DataSource> {
  // Native fixture mode belongs to Rust; a Vite flag must never replace real IPC.
  if (isTauri()) {
    const source = new TauriDataSource();
    try {
      const info = await source.appInfo();
      if (info?.fixture !== null) return source;
      let updatesEnabled: boolean | undefined;
      try {
        updatesEnabled = await invoke<boolean>('updates_enabled');
      } catch {
        // Unknown updater mode exposes neither update capability.
      }
      return new TauriDataSource(true, updatesEnabled);
    } catch {
      return source;
    }
  }
  if (import.meta.env.DEV && import.meta.env.VITE_XTRACE_FIXTURE) {
    const { loadFixtureDataSource } = await import('./FixtureDataSource');
    return loadFixtureDataSource(import.meta.env.VITE_XTRACE_FIXTURE);
  }
  const unavailable = async (): Promise<never> => {
    throw new Error('Native data is unavailable in browser preview.');
  };
  return {
    kind: 'preview',
    accountUsage: unavailable,
    refreshClaudeUsage: unavailable,
    dashboard: unavailable,
    tokensByHost: unavailable,
    environment: unavailable,
    today: unavailable,
    sessionsList: unavailable,
    sessionRow: unavailable,
    sessionStretches: unavailable,
    sessionTranscript: unavailable,
    // Nothing was started, so there is nothing to abandon.
    cancelSessionTranscript: async () => {},
    appInfo: unavailable,
    dbCounts: unavailable,
    nativeIndexStatus: unavailable,
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async () => () => {},
  };
}
