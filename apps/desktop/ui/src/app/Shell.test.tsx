import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
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
// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});
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
  expect(screen.getByText(String(exported.db_counts.records))).toBeTruthy();
  expect(screen.getByText(String(exported.db_counts.usage))).toBeTruthy();
  expect(document.querySelector('.xt-nav-item[aria-current]')).toBeNull();
});

it('preserves PR query context and resolves fires before dynamic rule details', async () => {
  const view = mount('/sessions?pr=https%3A%2F%2Fgithub.com%2Fexample%2Fproject%2Fpull%2F1');
  expect(
    screen.getByText('Pull request filter: https://github.com/example/project/pull/1'),
  ).toBeTruthy();
  view.unmount();
  const fires = mount('/rulebook/fires');
  expect(screen.getByRole('heading', { name: 'Rule fires' })).toBeTruthy();
  expect(screen.getByText('fires')).toBeTruthy();
  fires.unmount();
  mount('/rulebook/sample-rule');
  expect(screen.getByRole('heading', { name: 'Rule detail' })).toBeTruthy();
  expect(screen.getByText('Rule: sample-rule')).toBeTruthy();
  await act(async () => {});
});

it('shows native metadata failure safely, retries, and preserves native drag exclusions', async () => {
  const source: DataSource = {
    kind: 'native',
    appInfo: vi
      .fn()
      .mockRejectedValueOnce(new Error('backend-specific detail'))
      .mockResolvedValue(exported.app_info),
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
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

it('CmdK focuses a present page Search and leaves focus alone when none exists', () => {
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
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(document.activeElement).toBe(screen.getByRole('searchbox'));
  view.unmount();
  mount('/dashboard');
  const button = screen.getByRole('button', { name: 'Sessions' });
  button.focus();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(document.activeElement).toBe(button);
});
