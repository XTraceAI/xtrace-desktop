import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import { FixtureDataSource } from '../data/FixtureDataSource';
import { ThemeProvider } from '../theme/ThemeProvider';
import { Search } from '../kit/Search';
import { AppRoutes } from './AppRoutes';
import { Shell } from './Shell';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { AccountUsage } from '../data/generated/AccountUsage';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});
/** The runtime mounts its screens once every listener has registered. */
const opened = () =>
  waitFor(() => expect(screen.queryByText('Connecting live updates…')).toBeNull());
function mount(path: string, source: DataSource = new FixtureDataSource(exported)) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

it('wires Updates only for a native source with the optional capability', async () => {
  const base = new FixtureDataSource(exported);
  const controls = { viewPublicReleases: vi.fn().mockResolvedValue(undefined) };
  // Keep deterministic fixture reads while exercising Shell's native capability boundary.
  const native = Object.assign(Object.create(base) as DataSource, {
    kind: 'native',
    localUpdates: controls,
  });
  const view = mount('/settings', native);
  await opened();
  fireEvent.click(screen.getByRole('button', { name: 'Updates' }));
  await screen.findByRole('dialog', { name: 'Updates' });
  expect(controls.viewPublicReleases).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'View public releases' }));
  await waitFor(() => expect(controls.viewPublicReleases).toHaveBeenCalledOnce());
  view.unmount();
  const fixtureWithCapability = Object.assign(Object.create(base) as DataSource, {
    localUpdates: controls,
  });
  mount('/settings', fixtureWithCapability);
  await opened();
  expect(screen.queryByRole('button', { name: 'Updates' })).toBeNull();
});

it('does not reread account limits when the Dashboard history range changes', async () => {
  const source = new FixtureDataSource(exported);
  const noLimits: AccountUsage = {
    claude: { state: 'unavailable', issue: 'source_unavailable', checked_at: null, windows: [] },
    codex: { state: 'unavailable', issue: 'source_unavailable', checked_at: null, windows: [] },
  };
  const read = vi.spyOn(source, 'accountUsage').mockResolvedValue(noLimits);
  const refreshClaude = vi.spyOn(source, 'refreshClaudeUsage').mockResolvedValue(noLimits);
  mount('/dashboard', source);
  await waitFor(() => expect(read).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() =>
    expect(screen.getByRole('radio', { name: '14d' }).getAttribute('aria-checked')).toBe('true'),
  );
  expect(read).toHaveBeenCalledTimes(1);
  expect(refreshClaude).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Refresh usage' }));
  await waitFor(() => expect(refreshClaude).toHaveBeenCalledTimes(1));
});

it('hides the previous Claude percentage when an explicit refresh fails', async () => {
  const source = new FixtureDataSource(exported);
  const old: AccountUsage = {
    claude: {
      state: 'available',
      issue: null,
      checked_at: 1790618400,
      windows: [
        {
          bucket_key: 'claude',
          window_key: 'seven_day',
          scope: 'all_models',
          name: 'Claude',
          window: 'Weekly',
          used_percent: 100,
          duration_minutes: 10080,
          resets_at: null,
        },
      ],
    },
    codex: { state: 'unavailable', issue: 'source_unavailable', checked_at: null, windows: [] },
  };
  vi.spyOn(source, 'accountUsage').mockResolvedValue(old);
  vi.spyOn(source, 'refreshClaudeUsage').mockRejectedValue(new Error('Private provider detail'));
  mount('/dashboard', source);
  expect(
    await screen.findByLabelText('Claude account usage: 0% remaining · Weekly · limit reached'),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Refresh usage' }));
  expect(
    await screen.findByText('Account usage could not be read. Refresh to try again.'),
  ).toBeTruthy();
  expect(screen.queryByText('0% remaining')).toBeNull();
  expect(screen.queryByText('Private provider detail')).toBeNull();
});

it('routes sidebar navigation, crumbs, settings and fixture metadata from the generated export', async () => {
  mount('/prs');
  expect(await screen.findByText('fixture F1')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Pull requests' }).getAttribute('aria-current')).toBe(
    'page',
  );
  expect(
    within(screen.getByRole('navigation', { name: 'Breadcrumb' })).getByText('pull-requests'),
  ).toBeTruthy();
  expect(
    within(screen.getByRole('navigation', { name: 'Main navigation' })).getAllByRole('button'),
  ).toHaveLength(5);
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(screen.getByRole('heading', { name: 'Sessions' })).toBeTruthy();
  fireEvent.keyDown(window, { key: ',', metaKey: true });
  expect(screen.getByRole('heading', { name: 'Settings' })).toBeTruthy();
  expect(screen.getByText(exported.app_info.data_dir)).toBeTruthy();
  expect(await screen.findByText(String(exported.db_counts.records))).toBeTruthy();
  // Read beside its own label: the schema version can share the count's digits.
  expect(screen.getByText('Usage rows').nextElementSibling?.textContent).toBe(
    String(exported.db_counts.usage),
  );
  expect(document.querySelector('.xt-nav-item[aria-current]')).toBeNull();
});

it('preserves PR query context and resolves fires before dynamic rule details', async () => {
  const view = mount('/sessions?pr=https%3A%2F%2Fgithub.com%2Fexample%2Fproject%2Fpull%2F1');
  await opened();
  expect(screen.getByText(/Pull request filtering is not available yet/)).toBeTruthy();
  view.unmount();
  const fires = mount('/rulebook/fires');
  await opened();
  expect(screen.getByRole('heading', { name: 'Recorded rule activity' })).toBeTruthy();
  expect(screen.getByText('fires')).toBeTruthy();
  fires.unmount();
  mount('/rulebook/sample-rule');
  await opened();
  expect(screen.getByRole('heading', { name: 'Rule detail' })).toBeTruthy();
  // The address is repeated as the one asked for, never crumbed as a rule.
  expect(screen.getByText('/rulebook/sample-rule')).toBeTruthy();
  expect(
    within(screen.getByRole('navigation', { name: 'Breadcrumb' })).queryByText('sample-rule'),
  ).toBeNull();
  await act(async () => {});
});

it('shows native metadata failure safely, retries, and preserves native drag exclusions', async () => {
  const source: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: vi
      .fn()
      .mockRejectedValueOnce(new Error('backend-specific detail'))
      .mockResolvedValue(exported.app_info),
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
  vi.spyOn(navigator, 'platform', 'get').mockReturnValue('MacIntel');
  const view = mount('/dashboard', source);
  expect(await screen.findByText(/App data could not be loaded/)).toBeTruthy();
  expect(screen.queryByText('backend-specific detail')).toBeNull();
  const refresh = screen.getByRole('button', { name: 'Refresh' });
  fireEvent.click(refresh);
  expect(await screen.findByText('v0.1.0')).toBeTruthy();
  expect(
    view.container.querySelector('.xt-window-chrome')?.getAttribute('data-tauri-drag-region'),
  ).toBe('deep');
  expect(view.container.querySelector('.xt-topbar')?.getAttribute('data-tauri-drag-region')).toBe(
    'deep',
  );
  expect(
    view.container.querySelector('.xt-topbar-tools')?.getAttribute('data-tauri-drag-region'),
  ).toBe('false');
  expect(screen.getByRole('button', { name: 'Sessions' }).tabIndex).toBe(0);
});

it('CmdK focuses a present page Search and leaves focus alone when none exists', async () => {
  const source = new FixtureDataSource(exported);
  const view = render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter>
          <Routes>
            <Route element={<Shell />}>
              <Route
                index
                element={<Search label="Search test page" value="" onValueChange={() => {}} />}
              />
            </Route>
          </Routes>
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await opened();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(document.activeElement).toBe(screen.getByRole('searchbox'));
  view.unmount();
  mount('/dashboard');
  await opened();
  const button = screen.getByRole('button', { name: 'Sessions' });
  button.focus();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(document.activeElement).toBe(button);
});

// The app keeps the kit's default Leaderboard row: nothing ranks in this local
// build, so the row is disabled and marked "soon" and never navigates, while a
// typed or bookmarked /leaderboard still opens its named placeholder.
it('keeps Leaderboard unavailable in the sidebar while its address still opens', async () => {
  const view = mount('/dashboard');
  await opened();
  expect(await screen.findByRole('heading', { name: 'What your agents did' })).toBeTruthy();
  const leaderboard = screen.getByRole('button', { name: /Leaderboard/ });
  expect(leaderboard.hasAttribute('disabled')).toBe(true);
  expect(within(leaderboard).getByText('soon')).toBeTruthy();
  expect(leaderboard.getAttribute('aria-current')).toBeNull();
  // A pointer press on the disabled row leaves the Dashboard as the page.
  fireEvent.click(leaderboard);
  expect(screen.getByRole('heading', { name: 'What your agents did' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Dashboard' }).getAttribute('aria-current')).toBe(
    'page',
  );
  // The keyboard cannot reach it: a disabled control takes no focus, so it is
  // out of the tab order with nothing for Enter or Space to activate, while
  // the working rows around it still take focus and navigate.
  leaderboard.focus();
  expect(document.activeElement).not.toBe(leaderboard);
  const rulebook = screen.getByRole('button', { name: 'Rulebook' });
  rulebook.focus();
  expect(document.activeElement).toBe(rulebook);
  expect(screen.getByRole('button', { name: 'XTrace Hub' }).tabIndex).toBe(0);
  fireEvent.click(rulebook);
  expect(screen.getByRole('heading', { name: 'Rulebook' })).toBeTruthy();
  view.unmount();
  mount('/leaderboard');
  await opened();
  expect(screen.getByRole('heading', { name: 'Leaderboard' })).toBeTruthy();
  expect(screen.getByText('This view is coming next.')).toBeTruthy();
  expect(
    within(screen.getByRole('navigation', { name: 'Breadcrumb' })).getByText('leaderboard'),
  ).toBeTruthy();
  const row = screen.getByRole('button', { name: /Leaderboard/ });
  expect(row.hasAttribute('disabled')).toBe(true);
  expect(row.getAttribute('aria-current')).toBe('page');
  await act(async () => {});
});
