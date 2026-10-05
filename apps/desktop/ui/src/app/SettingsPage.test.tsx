import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, RetentionControls } from '../data/DataSource';
import type { ContentPurge } from '../data/generated/ContentPurge';
import type { ContentRetention } from '../data/generated/ContentRetention';
import type { FixtureExport } from '../data/generated/FixtureExport';
import { events, type DataEvent } from '../data/ipc-names';
import { eventPrefixes, subscribeInvalidation } from '../data/subscribe-invalidation';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
import { invoke } from '@tauri-apps/api/core';
import { TauriDataSource } from '../data/TauriDataSource';
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const purged: ContentPurge = {
  tables: [
    { table: 'records', rows: 2 },
    { table: 'sessions', rows: 1 },
    { table: 'tool_uses', rows: 0 },
  ],
  invalidate_content: true,
};

/** A synthetic store: the mode lives here, never in a real database. */
function controls(initial: ContentRetention = 'metadata_only') {
  let saved = initial;
  return {
    read: vi.fn(async () => saved),
    set: vi.fn(async (mode: ContentRetention) => {
      saved = mode;
      return saved;
    }),
    purge: vi.fn(async () => purged),
  } satisfies RetentionControls;
}

function source(retention?: RetentionControls) {
  const listeners = new Map<DataEvent, () => void>();
  const data: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    dashboard: async () => exported.dashboards[0],
    tokensByHost: async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    }),
    today: async () => exported.today,
    environment: async () => exported.environments[0],
    sessionsList: vi.fn(async () => ({
      window: exported.sessions[0].window,
      rows: [],
      next: null,
    })),
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    // Pull-request refresh is not exercised by this test.
    pullRequests: async () => Promise.reject(new Error('not used by this test')),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    refreshPullRequests: async () => Promise.reject(new Error('not used by this test')),
    cancelPullRequestRefresh: async () => Promise.reject(new Error('not used by this test')),
    nativeIndexStatus: async () => exported.native_index,
    subscribe: async (event, listener) => {
      listeners.set(event, listener);
      return () => listeners.delete(event);
    },
    retention,
  };
  return { data, listeners };
}

/** Renders, then waits for the runtime to open its screens once every listener has registered. */
async function mount(data: DataSource) {
  const view = render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={['/settings']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByRole('heading', { name: 'Settings', level: 1 });
  return view;
}

const mode = () => screen.getByTestId('retention-mode');
const archive = () => screen.getByRole('switch', { name: 'Archive full transcript content' });

it('reads the saved mode, defaults to metrics and indexing, and states what the mode changes', async () => {
  const store = controls();
  await mount(source(store).data);
  expect(screen.getByRole('heading', { name: 'Settings', level: 1 })).toBeTruthy();
  expect(screen.queryByText('This view is coming next.')).toBeNull();
  await waitFor(() => expect(mode().textContent).toBe('Metrics and indexing'));
  expect(store.read).toHaveBeenCalledOnce();
  expect(archive().getAttribute('aria-checked')).toBe('false');
  expect(
    screen.getByText('Applies to future imports and enrichment. Existing saved content remains.'),
  ).toBeTruthy();
  // The one text kept whatever the mode is disclosed beside it.
  expect(screen.getByTestId('retention-previews').textContent).toContain(
    'a short preview (up to 280 characters) of each message you typed',
  );
  expect(screen.getByTestId('retention-previews').textContent).toContain(
    'previews cannot be turned off',
  );
  // The existing sections stay on the page.
  expect(screen.getByRole('heading', { name: 'Appearance' })).toBeTruthy();
  expect(await screen.findByText(exported.app_info.data_dir)).toBeTruthy();
  expect(screen.getByRole('heading', { name: 'Native index' })).toBeTruthy();
  // Unimplemented settings are named, not faked with controls.
  expect(screen.getByText(/This build has no telemetry collector/)).toBeTruthy();
  expect(screen.getAllByRole('switch')).toHaveLength(1);
  expect(store.set).not.toHaveBeenCalled();
  expect(store.purge).not.toHaveBeenCalled();
});

it('reads an existing opt-in as saved', async () => {
  await mount(source(controls('full_content')).data);
  await waitFor(() =>
    expect(mode().textContent).toBe('Metrics, indexing and full-content archive'),
  );
  expect(archive().getAttribute('aria-checked')).toBe('true');
});

it('opts in and out through the store and shows only the mode it reports back', async () => {
  const store = controls();
  let finish: ((mode: ContentRetention) => void) | undefined;
  store.set.mockImplementationOnce(
    (next) => new Promise<ContentRetention>((resolve) => (finish = () => resolve(next))),
  );
  await mount(source(store).data);
  await waitFor(() => expect(mode().textContent).toBe('Metrics and indexing'));
  fireEvent.click(archive());
  await waitFor(() => expect(store.set).toHaveBeenCalledExactlyOnceWith('full_content'));
  // Pending: the request is not shown as saved, and the control cannot be re-sent.
  expect(archive().getAttribute('aria-checked')).toBe('false');
  expect(archive().hasAttribute('disabled')).toBe(true);
  await act(async () => finish?.('full_content'));
  await waitFor(() => expect(archive().getAttribute('aria-checked')).toBe('true'));
  expect(mode().textContent).toBe('Metrics, indexing and full-content archive');
  fireEvent.click(archive());
  await waitFor(() => expect(mode().textContent).toBe('Metrics and indexing'));
  expect(store.set).toHaveBeenLastCalledWith('metadata_only');
  // Opting out never deletes saved content.
  expect(store.purge).not.toHaveBeenCalled();
});

it('keeps the saved mode when a change fails', async () => {
  const store = controls();
  store.set.mockRejectedValueOnce(new Error('application database is closed'));
  await mount(source(store).data);
  await waitFor(() => expect(mode().textContent).toBe('Metrics and indexing'));
  fireEvent.click(archive());
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The storage mode could not be changed. The mode shown is the saved one.',
  );
  await waitFor(() => expect(store.read).toHaveBeenCalledTimes(2));
  expect(archive().getAttribute('aria-checked')).toBe('false');
  expect(screen.queryByText(/application database is closed/)).toBeNull();
});

it('cancels the deletion by button or Escape without purging, returning focus', async () => {
  const store = controls();
  await mount(source(store).data);
  const trigger = screen.getByRole('button', { name: 'Delete stored content' });
  fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Delete stored content?' });
  expect(dialog.textContent).toContain(
    'Original host transcripts and other files are not touched.',
  );
  expect(dialog.textContent).toContain(
    'Counts, usage, tool names, identifiers and source provenance stay.',
  );
  expect(dialog.textContent).toContain('the short message previews');
  expect(dialog.textContent).toContain(
    'Messages indexed after the delete get new previews; previews cannot be turned off.',
  );
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(trigger));

  // Keyboard: focus moves into the dialog; Escape dismisses it.
  fireEvent.click(trigger);
  const again = await screen.findByRole('dialog');
  await waitFor(() => expect(again.contains(document.activeElement)).toBe(true));
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(store.purge).not.toHaveBeenCalled();
});

it('purges only on confirmation and refreshes content views from the committed outcome', async () => {
  const store = controls('full_content');
  const { data } = source(store);
  await mount(data);
  await waitFor(() =>
    expect(mode().textContent).toBe('Metrics, indexing and full-content archive'),
  );
  fireEvent.click(screen.getByRole('button', { name: 'Delete stored content' }));
  const dialog = await screen.findByRole('dialog');
  expect(dialog.textContent).toContain('later imports can save content again');
  // The settled index status reconciles its own queries once; spy after that.
  await screen.findByTestId('native-index');
  const invalidate = vi.spyOn(QueryClient.prototype, 'invalidateQueries');
  fireEvent.click(within(dialog).getByRole('button', { name: 'Delete stored content' }));
  await waitFor(() => expect(store.purge).toHaveBeenCalledOnce());
  expect(await screen.findByText('Deleted stored content from 3 rows.')).toBeTruthy();
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // The mode is unchanged by deletion.
  expect(store.set).not.toHaveBeenCalled();
  expect(mode().textContent).toBe('Metrics, indexing and full-content archive');
  expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
    ['sessions'],
    ['metrics'],
  ]);
});

it('reports an empty purge without claiming a deletion', async () => {
  const store = controls();
  store.purge.mockResolvedValueOnce({ tables: [], invalidate_content: false });
  await mount(source(store).data);
  fireEvent.click(screen.getByRole('button', { name: 'Delete stored content' }));
  const dialog = await screen.findByRole('dialog');
  fireEvent.click(within(dialog).getByRole('button', { name: 'Delete stored content' }));
  expect(await screen.findByText('There was no stored content to delete.')).toBeTruthy();
});

it('keeps the dialog open and invalidates nothing when the purge fails', async () => {
  const store = controls();
  store.purge.mockRejectedValueOnce(new Error('application storage operation failed'));
  await mount(source(store).data);
  await waitFor(() => expect(mode().textContent).toBe('Metrics and indexing'));
  // The settled index status reconciles its own queries once; spy after that.
  await screen.findByTestId('native-index');
  const invalidate = vi.spyOn(QueryClient.prototype, 'invalidateQueries');
  fireEvent.click(screen.getByRole('button', { name: 'Delete stored content' }));
  const dialog = await screen.findByRole('dialog');
  fireEvent.click(within(dialog).getByRole('button', { name: 'Delete stored content' }));
  expect((await within(dialog).findByRole('alert')).textContent).toBe(
    'Stored content could not be deleted.',
  );
  expect(screen.getByRole('dialog')).toBe(dialog);
  expect(screen.queryByText(/Deleted stored content/)).toBeNull();
  expect(invalidate).not.toHaveBeenCalled();
  // A retry from the same dialog can still succeed.
  fireEvent.click(within(dialog).getByRole('button', { name: 'Delete stored content' }));
  expect(await screen.findByText('Deleted stored content from 3 rows.')).toBeTruthy();
  expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
    ['sessions'],
    ['metrics'],
  ]);
});

it('says the controls are unavailable when the source has no app database', async () => {
  await mount(source().data);
  expect(screen.getByText(/Storage controls need the desktop app's database/)).toBeTruthy();
  expect(screen.queryByRole('switch')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Delete stored content' })).toBeNull();
  await act(async () => {});
});

it('refreshes content views when the committed-purge event arrives', async () => {
  expect(eventPrefixes[events.contentPurged]).toEqual(['sessions', 'metrics']);
  vi.useFakeTimers();
  const { data, listeners } = source();
  const client = new QueryClient();
  const invalidate = vi.spyOn(client, 'invalidateQueries').mockResolvedValue();
  const stop = subscribeInvalidation(data, client, (state) => {
    if (state === 'failed') throw new Error('subscription failed');
  });
  await act(async () => {});
  // Connecting reads everything once; this test is about the event that follows.
  invalidate.mockClear();
  listeners.get(events.contentPurged)?.();
  await vi.advanceTimersByTimeAsync(600);
  expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
    ['sessions'],
    ['metrics'],
  ]);
  stop();
});

it('invokes the native retention commands with the generated shapes', async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'purge_stored_content' ? purged : 'full_content',
  );
  const { retention } = new TauriDataSource();
  expect(await retention.read()).toBe('full_content');
  expect(await retention.set('full_content')).toBe('full_content');
  expect(await retention.purge()).toEqual(purged);
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['content_retention'],
    ['set_content_retention', { mode: 'full_content' }],
    ['purge_stored_content'],
  ]);
});
