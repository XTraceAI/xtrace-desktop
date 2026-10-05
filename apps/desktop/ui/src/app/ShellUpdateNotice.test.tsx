import { cleanup, render, screen, within } from '@testing-library/react';
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
});

it.each(['native', 'preview', 'fixture'])(
  'places update controls only in the %s sidebar',
  async (kind) => {
    const source = new FixtureDataSource(fixture as FixtureExport);
    Object.defineProperty(source, 'kind', { value: kind });
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
