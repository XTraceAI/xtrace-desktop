import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { App } from '../App';
import { createDataSource } from '../data/createDataSource';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { DataSource } from '../data/DataSource';
import { ThemeProvider } from '../theme/ThemeProvider';
import type { FixtureExport } from '../data/generated/FixtureExport';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('../data/createDataSource', () => ({ createDataSource: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  history.replaceState(null, '', '/');
});
it('does not replace the current source when an obsolete bootstrap resolves', async () => {
  let finish!: (source: DataSource) => void;
  vi.mocked(createDataSource)
    .mockImplementationOnce(
      () =>
        new Promise((done) => {
          finish = done;
        }),
    )
    .mockResolvedValueOnce(new FixtureDataSource(exported));
  render(
    <StrictMode>
      <ThemeProvider>
        <App />
      </ThemeProvider>
    </StrictMode>,
  );
  expect(await screen.findByText('v0.1.0')).toBeTruthy();
  await act(async () => {
    finish(
      new FixtureDataSource({
        ...exported,
        app_info: { ...exported.app_info, version: 'obsolete' },
      }),
    );
  });
  expect(screen.queryByText('vobsolete')).toBeNull();
});
it('renders an explicit unavailable fixture state when selection fails', async () => {
  vi.mocked(createDataSource).mockRejectedValue(new Error('Unsupported fixture'));
  render(
    <ThemeProvider>
      <App />
    </ThemeProvider>,
  );
  await screen.findByRole('alert');
  expect(screen.getByRole('alert').textContent).toContain('Only F1 is supported');
});

it('uses hash routes for native protocol navigation and reloads', async () => {
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
    dashboard: async () => exported.dashboards[0],
    tokensByHost: async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    }),
    today: async () => exported.today,
    environment: async () => exported.environments[0],
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    nativeIndexStatus: async () => exported.native_index,
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async () => () => {},
  };
  vi.mocked(createDataSource).mockResolvedValue(source);
  history.replaceState(
    null,
    '',
    '/#/sessions?pr=https%3A%2F%2Fgithub.com%2Fexample%2Fproject%2Fpull%2F1',
  );
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(
      <ThemeProvider>
        <App />
      </ThemeProvider>,
    );
  });
  await screen.findByRole('heading', { name: 'Sessions' });
  expect(screen.getByText(/Pull request filtering is not available yet/)).toBeTruthy();
  await act(async () => {
    fireEvent.keyDown(window, { key: ',', metaKey: true });
  });
  expect(location.hash).toBe('#/settings');
  expect(location.pathname).toBe('/');
  view.unmount();
  render(
    <ThemeProvider>
      <App />
    </ThemeProvider>,
  );
  expect(await screen.findByRole('heading', { name: 'Settings' })).toBeTruthy();
});
