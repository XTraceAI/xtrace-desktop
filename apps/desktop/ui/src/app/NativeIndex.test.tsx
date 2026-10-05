import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const dashboard = async () => exported.dashboards[0];
const tokensByHost = async () => ({
  window: exported.dashboards[0].window,
  hosts: exported.dashboards[0].tokens_by_host,
});
const environment = async () => exported.environments[0];
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
      skipped_conversations: [],
      skipped_conversations_omitted: 0,
      records_new: 6,
      records_enriched: 1,
      diagnostics: 0,
    },
    {
      host: 'codex',
      state: 'missing_runtime',
      detail: 'python runtime unavailable: python3 on PATH: python3 must be 3.10 or newer',
      sessions_imported: 0,
      sessions_partial: 0,
      sessions_skipped: 0,
      skipped_conversations: [],
      skipped_conversations_omitted: 0,
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
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    dashboard,
    tokensByHost,
    today: async () => exported.today,
    environment,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: vi.fn(async () => status),
    // These screens open no transcript; the seam is answered, never called.
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
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
    within(card).getByText(
      'complete · 2 imported, 0 partial, 0 skipped, 6 new records, 1 enriched',
    ),
  ).toBeTruthy();
  expect(
    within(card).getByText(
      'missing_runtime · 0 imported, 0 partial, 0 skipped, 0 new records, 0 enriched · python runtime unavailable: python3 on PATH: python3 must be 3.10 or newer',
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
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: vi.fn(async () => exported.db_counts),
    dashboard,
    tokensByHost,
    today: async () => exported.today,
    environment,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: vi.fn(async () => status),
    // No event ever arrives: the listener registered after the only `ready`.
    // These screens open no transcript; the seam is answered, never called.
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async () => () => {},
  };
  mount(source);
  // The runtime mounts its screens once every listener has registered.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(50);
  });
  expect(screen.getByText('Scanning (2 Claude transcripts read)')).toBeTruthy();
  const countsBeforeSettling = vi.mocked(source.dbCounts).mock.calls.length;
  status = ready;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1100);
  });
  expect(screen.getByText('Ready (3 reconciliations)')).toBeTruthy();
  // Settling also refetches the database counts the lost event would have.
  expect(vi.mocked(source.dbCounts).mock.calls.length).toBeGreaterThan(countsBeforeSettling);
  const settled = vi.mocked(source.nativeIndexStatus).mock.calls.length;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3000);
  });
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(settled);
  vi.useRealTimers();
});

it('reconciles the data queries when the first status seen is already settled', async () => {
  // The counts query can answer mid-scan just before a status that is
  // already ready, after the one-time event was lost: the first settled
  // status refetches them once.
  const source: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: vi.fn(async () => exported.db_counts),
    dashboard,
    tokensByHost,
    today: async () => exported.today,
    environment,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: vi.fn(async () => ready),
    // These screens open no transcript; the seam is answered, never called.
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async () => () => {},
  };
  mount(source);
  await screen.findByText('Ready (3 reconciliations)');
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(source.dbCounts).toHaveBeenCalledTimes(2);
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(1);
});

it('says a host state covers the supported source scan, and that Cursor IDE database conversations are outside it', async () => {
  const cursor = {
    ...ready.hosts[0],
    host: 'cursor',
    sessions_imported: 5,
    records_new: 40,
    records_enriched: 0,
  };
  let status: NativeIndexStatus = { ...ready, hosts: [...ready.hosts, cursor] };
  let listener: (() => void) | undefined;
  const source: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    dashboard,
    tokensByHost,
    today: async () => exported.today,
    environment,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: async () => status,
    // These screens open no transcript; the seam is answered, never called.
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async (event, callback) => {
      if (event === events.nativeIndexStatus) listener = callback;
      return () => {};
    },
  };
  mount(source);
  const card = await screen.findByTestId('native-index');
  // The reported state and counts are unchanged, and no total is invented.
  expect(
    within(card).getByText(
      'complete · 5 imported, 0 partial, 0 skipped, 40 new records, 0 enriched',
    ),
  ).toBeTruthy();
  const scope = screen.getByTestId('native-index-scope');
  expect(scope.textContent).toBe(
    "Complete and incomplete describe each host's last scan of the sources this index reads, not all of that host's history. Conversations kept only in the Cursor IDE's database are not read, so they are not in Cursor's counts.",
  );
  // A plain note after the host rows: nothing to focus, nothing announced as it changes.
  expect(card.compareDocumentPosition(scope) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(scope.getAttribute('role')).toBeNull();
  expect(scope.querySelector('a, button, [tabindex]')).toBeNull();

  // Without a Cursor scan, only the general scope is stated.
  vi.useFakeTimers();
  status = ready;
  await act(async () => {
    listener?.();
    await vi.advanceTimersByTimeAsync(600);
  });
  vi.useRealTimers();
  await waitFor(() => expect(screen.queryByText(/^cursor$/)).toBeNull());
  expect(screen.getByTestId('native-index-scope').textContent).toBe(
    "Complete and incomplete describe each host's last scan of the sources this index reads, not all of that host's history.",
  );
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
  // No host was scanned, so there is no scan scope to explain.
  expect(screen.queryByTestId('native-index-scope')).toBeNull();
});

it('expands skipped details, states missing facts and truncation, and clears old reasons on host updates', async () => {
  const { FixtureDataSource } = await import('../data/FixtureDataSource');
  const id = '00000000-0000-4000-8000-000000000001';
  const skippedHost: NativeIndexStatus['hosts'][number] = {
    ...ready.hosts[0],
    state: 'incomplete',
    sessions_skipped: 3,
    skipped_conversations: [
      { conversation_id: id, reason: 'invalid_transcript' },
      { conversation_id: null, reason: 'unknown' },
    ],
    skipped_conversations_omitted: 1,
  };
  let status: NativeIndexStatus = { ...ready, hosts: [skippedHost] };
  let listener: (() => void) | undefined;
  const source = new FixtureDataSource(exported);
  source.nativeIndexStatus = async () => status;
  source.subscribe = async (event, callback) => {
    if (event === events.nativeIndexStatus) listener = callback;
    return () => {};
  };
  mount(source);
  const summary = await screen.findByText('Skipped conversation details (claude)');
  const details = summary.closest('details')!;
  expect(details.open).toBe(false);
  fireEvent.click(summary);
  expect(details.open).toBe(true);
  expect(
    within(details).getByText(`${id} — Conversation data has an invalid format.`),
  ).toBeTruthy();
  expect(within(details).getByText('ID unavailable — Reason unavailable.')).toBeTruthy();
  expect(
    within(details).getByText('Showing 2 of 3 skipped conversations. 1 more omitted.'),
  ).toBeTruthy();
  expect(details.textContent).toContain('including earlier outcomes it retained');
  fireEvent.click(summary);
  expect(details.open).toBe(false);

  const update = async (nextHost: NativeIndexStatus['hosts'][number]) => {
    status = { ...ready, hosts: [nextHost], reconciles: status.reconciles + 1 };
    vi.useFakeTimers();
    await act(async () => {
      listener?.();
      await vi.advanceTimersByTimeAsync(600);
    });
    vi.useRealTimers();
  };
  // An incomplete/cancelled scan can still report skips; the facts remain visible.
  await update({ ...skippedHost, state: 'cancelled' });
  expect(screen.getByText('Skipped conversation details (claude)')).toBeTruthy();
  // The next report replaces both the ID and reason, including while expanded.
  fireEvent.click(screen.getByText('Skipped conversation details (claude)'));
  await update({
    ...skippedHost,
    sessions_skipped: 1,
    skipped_conversations: [{ conversation_id: null, reason: 'unreadable' }],
    skipped_conversations_omitted: 0,
  });
  expect(screen.queryByText(new RegExp(id))).toBeNull();
  expect(screen.queryByText(/Reason unavailable/)).toBeNull();
  expect(screen.getByText('ID unavailable — Conversation file could not be read.')).toBeTruthy();
  expect(screen.queryByText(/more omitted/)).toBeNull();
  // A recovered scan, then an empty host, removes the disclosure and its old text.
  await update(ready.hosts[0]);
  expect(screen.queryByText(/Skipped conversation details/)).toBeNull();
  expect(screen.queryByText(/Conversation file could not be read/)).toBeNull();
  await update({ ...ready.hosts[0], state: 'missing_source', sessions_imported: 0 });
  expect(screen.queryByText(/Skipped conversation details/)).toBeNull();
});
