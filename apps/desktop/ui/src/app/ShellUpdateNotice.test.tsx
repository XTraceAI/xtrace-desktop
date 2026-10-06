import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import { ThemeProvider } from '../theme/ThemeProvider';
import { Shell } from './Shell';

// Synthetic ready state: no native updater is run by this location check.
vi.mock('./UpdateNotice', () => ({
  UpdateNotice: () => <button type="button">Restart to update</button>,
}));
afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.unstubAllEnvs();
});

it.each(['native', 'preview', 'fixture'])(
  'places update controls only in the %s sidebar',
  async (kind) => {
    vi.stubEnv('DEV', false);
    const source = new FixtureDataSource(fixture as FixtureExport);
    Object.defineProperty(source, 'kind', { value: kind });
    Object.defineProperty(source, 'publicUpdates', { value: true });
    render(
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter initialEntries={['/settings']}>
            <Routes>
              <Route element={<Shell />}>
                <Route path="/settings" element={<p>Page content</p>} />
              </Route>
            </Routes>
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>,
    );
    await screen.findByText('Page content');
    const main = screen.getByRole('main');
    expect(within(main).queryByRole('button', { name: 'Restart to update' })).toBeNull();
    if (kind === 'native') {
      const sidebar = screen.getByRole('complementary', { name: 'Workspace' });
      await within(sidebar).findByText(`v${fixture.app_info.version}`);
      const button = within(sidebar).getByRole('button', { name: 'Restart to update' });
      const row = button.closest('.xt-sidebar-version');
      expect(row?.textContent).toContain(`v${fixture.app_info.version}`);
      expect(screen.getAllByRole('button', { name: 'Restart to update' })).toHaveLength(1);
    } else {
      expect(screen.queryByRole('button', { name: 'Restart to update' })).toBeNull();
    }
  },
);

it.each([true, false])('uses selected local controls with DEV=%s', async (dev) => {
  vi.stubEnv('DEV', dev);
  const source = new FixtureDataSource(fixture as FixtureExport);
  const viewPublicReleases = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(source, 'kind', { value: 'native' });
  Object.defineProperty(source, 'localUpdates', { value: { viewPublicReleases } });
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/settings']}>
          <Routes>
            <Route element={<Shell />}>
              <Route path="/settings" element={<p>Page content</p>} />
            </Route>
          </Routes>
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  const updates = await screen.findByRole('button', { name: 'Updates' });
  expect(screen.queryByRole('button', { name: 'Restart to update' })).toBeNull();
  expect(viewPublicReleases).not.toHaveBeenCalled();
  fireEvent.click(updates);
  fireEvent.click(await screen.findByRole('button', { name: 'View public releases' }));
  expect(viewPublicReleases).toHaveBeenCalledExactlyOnceWith();
});
