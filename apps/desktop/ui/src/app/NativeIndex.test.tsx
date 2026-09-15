import { act, cleanup, render, screen, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { events } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const ready: NativeIndexStatus = {
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/opt/homebrew/bin/python3' },
  readers: {
    state: 'verified',
    commit: 'd7c94227cc9bd8ff46f933539ac65b478dd8f8ef',
    plugin_version: '0.55.0',
  },
  hosts: [
    {
      host: 'claude',
      state: 'complete',
      detail: null,
      sessions_imported: 2,
      sessions_partial: 0,
      sessions_skipped: 0,
      records_new: 6,
      records_enriched: 0,
      diagnostics: 0,
    },
    {
      host: 'codex',
      state: 'missing_runtime',
      detail: 'python runtime unavailable: python3 on PATH: python3 must be 3.10 or newer',
      sessions_imported: 0,
      sessions_partial: 0,
      sessions_skipped: 0,
      records_new: 0,
      records_enriched: 0,
      diagnostics: 0,
    },
  ],
  reconciles: 3,
  files_scanned: 2,
};

function mount(source: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/settings']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

it('shows the typed native index status as reported and refreshes it on its event', async () => {
  let listener: (() => void) | undefined;
  let status = ready;
  const source: DataSource = {
    kind: 'native',
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: vi.fn(async () => status),
    subscribe: vi.fn(async (event, callback) => {
      if (event === events.nativeIndexStatus) listener = callback;
      return () => {};
    }),
  };
  mount(source);
  const card = await screen.findByTestId('native-index');
  expect(within(card).getByText('Ready (3 reconciliations)')).toBeTruthy();
  expect(within(card).getByText('Live: changes are reconciled as they happen')).toBeTruthy();
  expect(within(card).getByText('/opt/homebrew/bin/python3')).toBeTruthy();
  expect(within(card).getByText('Bundled memhub 0.55.0 at d7c94227cc9b')).toBeTruthy();
  expect(
    within(card).getByText('complete · 2 imported, 0 partial, 0 skipped, 6 new records'),
  ).toBeTruthy();
  expect(
    within(card).getByText(
      'missing_runtime · 0 imported, 0 partial, 0 skipped, 0 new records · python runtime unavailable: python3 on PATH: python3 must be 3.10 or newer',
    ),
  ).toBeTruthy();
  // The next status arrives through the event: the query is invalidated and refetched.
  vi.useFakeTimers();
  status = {
    ...ready,
    phase: { phase: 'stopped' },
    freshness: { freshness: 'degraded', reason: 'watcher lost' },
  };
  await act(async () => {
    listener?.();
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  expect(await screen.findByText('Stopped')).toBeTruthy();
  expect(screen.getByText('Degraded: watcher lost')).toBeTruthy();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
});

it('polls a transient status until it settles, so a ready event that precedes the listener is not lost', async () => {
  vi.useFakeTimers();
  const scanning: NativeIndexStatus = {
    ...ready,
    phase: { phase: 'scanning' },
    python: { state: 'resolving' },
    hosts: [],
    reconciles: 0,
  };
  let status = scanning;
  const source: DataSource = {
    kind: 'native',
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: vi.fn(async () => status),
    // No event ever arrives: the listener registered after the only `ready`.
    subscribe: async () => () => {},
  };
  mount(source);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(50);
  });
  expect(screen.getByText('Scanning (2 Claude transcripts read)')).toBeTruthy();
  status = ready;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1100);
  });
  expect(screen.getByText('Ready (3 reconciliations)')).toBeTruthy();
  const settled = vi.mocked(source.nativeIndexStatus).mock.calls.length;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3000);
  });
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(settled);
  vi.useRealTimers();
});

it('shows the disabled fixture index without inventing hosts', async () => {
  const { FixtureDataSource } = await import('../data/FixtureDataSource');
  mount(new FixtureDataSource(exported));
  const card = await screen.findByTestId('native-index');
  expect(within(card).getByText('Disabled: fixture mode uses a disposable database')).toBeTruthy();
  expect(within(card).getByText('Not watching yet')).toBeTruthy();
  expect(within(card).getByText('Unavailable: not resolved: the index is disabled')).toBeTruthy();
  expect(within(card).getByText('Unavailable: not verified: the index is disabled')).toBeTruthy();
  expect(within(card).queryByText(/imported/)).toBeNull();
});
