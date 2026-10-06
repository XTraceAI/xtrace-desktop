import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, HumanBreakControls } from '../data/DataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import { queryKeys } from '../data/query-client';
import { TauriDataSource } from '../data/TauriDataSource';
import { ThemeProvider } from '../theme/ThemeProvider';
import { invoke } from '@tauri-apps/api/core';
import { AppRoutes } from './AppRoutes';
import { parseBreakMinutes } from './SettingsPage';
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/** A synthetic store: the length lives here, never in a real database. */
function controls(initial = 60) {
  let saved = initial;
  return {
    read: vi.fn(async () => saved),
    set: vi.fn(async (minutes: number) => {
      saved = minutes;
      return saved;
    }),
  } satisfies HumanBreakControls;
}

function source(humanBreak?: HumanBreakControls): DataSource {
  const unused = async () => Promise.reject(new Error('not used by this test'));
  return {
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
    tokensByHost: unused,
    today: async () => exported.today,
    environment: async () => exported.environments[0],
    sessionsList: unused,
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: unused,
    cancelSessionTranscript: async () => {},
    pullRequests: unused,
    refreshPullRequests: unused,
    cancelPullRequestRefresh: unused,
    pullRequestAnalytics: unused,
    pullRequestSessions: unused,
    nativeIndexStatus: async () => exported.native_index,
    subscribe: async () => () => {},
    humanBreak,
  };
}

async function mount(data: DataSource) {
  render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={['/settings']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByRole('heading', { name: 'Your hours' });
  // The settled index status reconciles its own queries once; spy after that.
  await screen.findByTestId('native-index');
}

const saved = () => screen.getByTestId('human-break-saved');
const field = () =>
  screen.getByRole('textbox', { name: 'Break after (minutes)' }) as HTMLInputElement;
const saveButton = () => screen.getByRole('button', { name: 'Save' });
const reset = () => screen.getByRole('button', { name: 'Reset to 60 min' });
/** From now on, the query keys invalidated, in order. */
function spyInvalidate() {
  const spy = vi.spyOn(QueryClient.prototype, 'invalidateQueries');
  return () => spy.mock.calls.map(([filters]) => filters?.queryKey);
}

it('accepts whole minutes from 5 to 240 only', () => {
  for (const [text, minutes] of [
    ['5', 5],
    [' 60 ', 60],
    ['240', 240],
  ] as const)
    expect(parseBreakMinutes(text)).toBe(minutes);
  for (const text of ['', '4', '241', '-5', '60.5', 'abc', '1000'])
    expect(parseBreakMinutes(text)).toBeNull();
});

it('reads the saved default and explains what a break is', async () => {
  const store = controls();
  await mount(source(store));
  await waitFor(() => expect(saved().textContent).toBe('60 min'));
  expect(field().value).toBe('60');
  const hint = document.getElementById(field().getAttribute('aria-describedby')!)!;
  expect(hint.textContent).toContain(
    'at most this many minutes apart, the time between them counts',
  );
  expect(reset().hasAttribute('disabled')).toBe(true);
  expect(store.set).not.toHaveBeenCalled();
});

it('saves a new length and re-reads every Dashboard range', async () => {
  const store = controls();
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const invalidated = spyInvalidate();
  fireEvent.change(field(), { target: { value: '45' } });
  fireEvent.submit(field().form!);
  await waitFor(() => expect(store.set).toHaveBeenCalledExactlyOnceWith(45));
  await waitFor(() => expect(saved().textContent).toBe('45 min'));
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
});

it('refuses an invalid length without writing, and says a failed save changed nothing', async () => {
  const store = controls();
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  fireEvent.change(field(), { target: { value: '4' } });
  expect(screen.getByRole('alert').textContent).toBe('Enter a whole number from 5 to 240.');
  fireEvent.submit(field().form!);
  expect(store.set).not.toHaveBeenCalled();
  store.set.mockRejectedValueOnce(new Error('application database is closed'));
  fireEvent.change(field(), { target: { value: '90' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The break length could not be saved. The length shown is the saved one.',
  );
  expect(saved().textContent).toBe('60 min');
});

it('states the preview fixed length where there is no app database', async () => {
  await mount(source());
  expect(screen.getByText(/This preview uses 60 minutes/)).toBeTruthy();
  expect(screen.queryByRole('textbox', { name: 'Break after (minutes)' })).toBeNull();
});

it('invokes the native break-length commands with plain integers', async () => {
  vi.mocked(invoke).mockImplementation(async () => 45);
  const { humanBreak } = new TauriDataSource();
  expect(await humanBreak.read()).toBe(45);
  expect(await humanBreak.set(45)).toBe(45);
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['human_break'],
    ['set_human_break', { minutes: 45 }],
  ]);
  expect(queryKeys.humanBreak).toEqual(['settings', 'human-break']);
});
