import { act, render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import type { DataSource } from '../data/DataSource';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import fixture from '../../fixtures/F1.json';
import { events } from '../data/ipc-names';
import { SessionsPage } from './SessionsPage';
afterEach(cleanup);

it('renders stored metadata and resets pagination when filters change', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList');
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  expect(screen.getByText('25')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'not-present' } });
  await screen.findByText('No sessions match these filters.');
  await waitFor(() => expect(list).toHaveBeenLastCalledWith('not-present', null, null));
  fireEvent.change(screen.getByLabelText('Filter by host'), { target: { value: 'codex' } });
  await waitFor(() => expect(list).toHaveBeenLastCalledWith('not-present', 'codex', null));
});

it('does not pretend a PR filter is applied before that query is implemented', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList');
  render(
    <MemoryRouter initialEntries={['/sessions?pr=example']}>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText(/Pull request filtering is not available/)).toBeTruthy();
  expect(list).not.toHaveBeenCalled();
});

it('loads the next page, refreshes after import and resets pages for a host filter', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let count = 25;
  const row = fixture.sessions.rows[0];
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (_search, host, after) => {
    if (host) return { rows: [], next: null };
    return after
      ? { rows: [{ ...row, id: 'second-session', record_count: 9 }], next: null }
      : { rows: [{ ...row, record_count: count }], next: 'page-two' };
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('25');
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  await screen.findByText('Session second-s');
  expect(list).toHaveBeenCalledWith('', null, 'page-two');
  count = 26;
  act(() => source.emit(events.importReceived));
  await screen.findByText('26');
  expect(screen.getByText('Session second-s')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('Filter by host'), { target: { value: 'cursor' } });
  await screen.findByText('No sessions match these filters.');
  expect(screen.queryByText('Session second-s')).toBeNull();
  expect(list).toHaveBeenLastCalledWith('', 'cursor', null);
});

it('recovers from a failed query using the retry control', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi
    .spyOn(source, 'sessionsList')
    .mockRejectedValue(new Error('database unavailable'));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('alert');
  list.mockRestore();
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await screen.findByText('Session 00000000');
});

it('keeps indexed rows visible with a warning when native indexing is disabled', async () => {
  const fixtureSource = new FixtureDataSource(fixture as FixtureExport);
  const source: DataSource = {
    kind: 'native',
    appInfo: () => fixtureSource.appInfo(),
    dbCounts: () => fixtureSource.dbCounts(),
    nativeIndexStatus: () => fixtureSource.nativeIndexStatus(),
    sessionsList: (search, host, after) => fixtureSource.sessionsList(search, host, after),
    subscribe: (event, listener) => fixtureSource.subscribe(event, listener),
  };
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText(/Indexing is unavailable/);
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  expect(screen.getByRole('link', { name: 'View indexing status' }).getAttribute('href')).toBe(
    '/settings',
  );
});
