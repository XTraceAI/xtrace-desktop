import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, TypingSpeedControls } from '../data/DataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import { queryKeys } from '../data/query-client';
import { TauriDataSource } from '../data/TauriDataSource';
import { ThemeProvider } from '../theme/ThemeProvider';
import { invoke } from '@tauri-apps/api/core';
import { AppRoutes } from './AppRoutes';
import { parseWpm, TYPING_TEST_URL } from './SettingsPage';
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/** A synthetic store: the speed lives here, never in a real database. */
function controls(initial = 40) {
  let saved = initial;
  return {
    read: vi.fn(async () => saved),
    set: vi.fn(async (wpm: number) => {
      saved = wpm;
      return saved;
    }),
    openTest: vi.fn(async () => {}),
  } satisfies TypingSpeedControls;
}

function source(typingSpeed?: TypingSpeedControls): DataSource {
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
    typingSpeed,
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
  await screen.findByRole('heading', { name: 'Typing speed' });
  // The settled index status reconciles its own queries once; spy after that.
  await screen.findByTestId('native-index');
}

const saved = () => screen.getByTestId('typing-speed-saved');
const field = () => screen.getByRole('textbox', { name: 'Words per minute' }) as HTMLInputElement;
const saveButton = () => screen.getByRole('button', { name: 'Save' });
const reset = () => screen.getByRole('button', { name: 'Reset to 40 WPM' });
const link = () => screen.getByRole('link', { name: 'Test your typing speed on Monkeytype' });
/** From now on, the query keys invalidated, in order. */
function spyInvalidate() {
  const spy = vi.spyOn(QueryClient.prototype, 'invalidateQueries');
  return () => spy.mock.calls.map(([filters]) => filters?.queryKey);
}

it('accepts whole numbers from 1 to 300 only', () => {
  for (const [text, wpm] of [
    ['1', 1],
    ['40', 40],
    [' 80 ', 80],
    ['300', 300],
    ['007', 7],
  ] as const)
    expect(parseWpm(text)).toBe(wpm);
  for (const text of [
    '',
    ' ',
    '0',
    '-1',
    '301',
    '1000',
    '40.5',
    '4e1',
    '0x28',
    'abc',
    '+40',
    'NaN',
  ])
    expect(parseWpm(text)).toBeNull();
});

it('reads the saved default and explains the conversion beside the official test', async () => {
  const store = controls();
  await mount(source(store));
  await waitFor(() => expect(saved().textContent).toBe('40 WPM (200 characters per minute)'));
  expect(field().value).toBe('40');
  expect(store.read).toHaveBeenCalledOnce();
  const hint = document.getElementById(field().getAttribute('aria-describedby')!)!;
  expect(hint.textContent).toContain('at 5 characters per word, the same word Monkeytype counts');
  expect(hint.textContent).toContain('40 WPM, the default, is 200 characters per minute');
  expect(hint.textContent).toContain('both the selected and the previous period');
  expect(hint.textContent).toContain(
    'not a measurement of your attention or of whether text was typed or pasted',
  );
  // Unchanged: nothing to save, and already at the default.
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
  expect(link().getAttribute('href')).toBe('https://monkeytype.com/');
  expect(link().getAttribute('target')).toBe('_blank');
  expect(link().getAttribute('rel')).toBe('noopener noreferrer');
  expect(store.set).not.toHaveBeenCalled();
});

it('saves by keyboard, shows only the committed speed and re-reads every Dashboard range', async () => {
  const store = controls();
  let finish: (() => void) | undefined;
  store.set.mockImplementationOnce(
    (wpm) => new Promise<number>((resolve) => (finish = () => resolve(wpm))),
  );
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('40'));
  const invalidated = spyInvalidate();
  field().focus();
  fireEvent.change(field(), { target: { value: '80' } });
  expect(saveButton().hasAttribute('disabled')).toBe(false);
  fireEvent.submit(field().form!);
  await waitFor(() => expect(store.set).toHaveBeenCalledExactlyOnceWith(80));
  // Pending: the saved value is still the old one and nothing can be re-sent.
  expect(saved().textContent).toBe('40 WPM (200 characters per minute)');
  expect(screen.getByRole('button', { name: 'Saving…' }).hasAttribute('disabled')).toBe(true);
  expect(invalidated()).toEqual([]);
  await act(async () => finish?.());
  await waitFor(() => expect(saved().textContent).toBe('80 WPM (400 characters per minute)'));
  expect(field().value).toBe('80');
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  expect(reset().hasAttribute('disabled')).toBe(false);
});

it('refuses invalid entries in the page without writing or refreshing', async () => {
  const store = controls(60);
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const invalidated = spyInvalidate();
  for (const value of ['0', '-5', '40.5', '', 'abc', '301']) {
    fireEvent.change(field(), { target: { value } });
    expect(field().getAttribute('aria-invalid')).toBe('true');
    expect(screen.getByRole('alert').textContent).toBe('Enter a whole number from 1 to 300.');
    expect(field().getAttribute('aria-describedby')).toContain(screen.getByRole('alert').id);
    expect(saveButton().hasAttribute('disabled')).toBe(true);
    fireEvent.submit(field().form!);
  }
  expect(store.set).not.toHaveBeenCalled();
  expect(invalidated()).toEqual([]);
  expect(saved().textContent).toBe('60 WPM (300 characters per minute)');
  // Reset abandons the edit and restores the default.
  fireEvent.click(reset());
  await waitFor(() => expect(store.set).toHaveBeenCalledExactlyOnceWith(40));
  await waitFor(() => expect(saved().textContent).toBe('40 WPM (200 characters per minute)'));
  expect(field().value).toBe('40');
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
});

it('keeps the saved speed and the Dashboard cache when a save fails', async () => {
  const store = controls();
  store.set.mockRejectedValueOnce(new Error('application database is closed'));
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('40'));
  const invalidated = spyInvalidate();
  fireEvent.change(field(), { target: { value: '90' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The typing speed could not be saved. The speed shown is the saved one.',
  );
  // The setting was read back once and still holds the old speed.
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(saved().textContent).toBe('40 WPM (200 characters per minute)');
  // The edit stays for a retry; nothing was invalidated.
  expect(field().value).toBe('90');
  expect(invalidated()).toEqual([]);
  expect(screen.queryByText(/application database is closed/)).toBeNull();
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved().textContent).toBe('90 WPM (450 characters per minute)'));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
});

it('treats a committed save whose reply was lost as saved', async () => {
  const store = controls();
  store.set.mockImplementationOnce(async (wpm) => {
    await store.set(wpm);
    throw new Error('application database write did not report its outcome');
  });
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('40'));
  const invalidated = spyInvalidate();
  fireEvent.change(field(), { target: { value: '70' } });
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved().textContent).toBe('70 WPM (350 characters per minute)'));
  // Read back as the requested speed: a save, with the edit and error cleared.
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(field().value).toBe('70');
  expect(field().getAttribute('aria-invalid')).toBe('false');
  expect(reset().hasAttribute('disabled')).toBe(false);
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  // The draft was cleared: a new edit starts from the saved speed.
  fireEvent.change(field(), { target: { value: '75' } });
  expect(saveButton().hasAttribute('disabled')).toBe(false);
});

/** A write that commits and then loses its reply. */
function commitThenLoseReply(store: ReturnType<typeof controls>) {
  store.set.mockImplementationOnce(async (wpm) => {
    await store.set(wpm);
    throw new Error('application database write did not report its outcome');
  });
}

const UNKNOWN_70 =
  'The typing speed could not be read back, so whether 70 WPM was saved is unknown. The speed shown was last confirmed before this save. Check the saved speed before changing it; the Dashboard will read the saved speed again.';
const check = () => screen.getByRole('button', { name: 'Check saved speed' });

/**
 * Saves 70 over a saved 40: the write commits (or not) and loses its reply,
 * and the one read-back fails too. Returns the spy of invalidated keys.
 */
async function saveUnknown70(store: ReturnType<typeof controls>, commits = true) {
  if (commits) commitThenLoseReply(store);
  else store.set.mockRejectedValueOnce(new Error('application database is closed'));
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('40'));
  const invalidated = spyInvalidate();
  store.read.mockRejectedValueOnce(new Error('application storage operation failed'));
  fireEvent.change(field(), { target: { value: '70' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(UNKNOWN_70);
  return invalidated;
}

it('reports an unknown outcome after a lost reply and a failed read-back, and writes nothing more', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store);
  // One read-back only, then one conservative Dashboard refresh.
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  expect(screen.queryByText(/could not be saved/)).toBeNull();
  // The cached speed is only the last confirmed one.
  expect(screen.getByText('Last confirmed')).toBeTruthy();
  expect(saved().textContent).toBe('40 WPM (200 characters per minute)');
  expect(field().value).toBe('70');
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
  expect(check().hasAttribute('disabled')).toBe(false);
  // Neither the edit, the cached speed nor the keyboard writes anything.
  fireEvent.submit(field().form!);
  fireEvent.change(field(), { target: { value: '40' } });
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  fireEvent.submit(field().form!);
  fireEvent.click(reset());
  await act(async () => {});
  // Only the lost-reply write (and the commit it wraps).
  expect(store.set.mock.calls).toEqual([[70], [70]]);
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(screen.getByText('Last confirmed')).toBeTruthy();
});

it('settles an unknown save as saved when a check reads the requested speed, then resets by writing', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store);
  fireEvent.click(check());
  await waitFor(() => expect(saved().textContent).toBe('70 WPM (350 characters per minute)'));
  expect(store.read).toHaveBeenCalledTimes(3);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByText('Saved')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Check saved speed' })).toBeNull();
  // The draft was cleared; the ranges were already re-read when the outcome became unknown.
  expect(field().value).toBe('70');
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  // Reset now writes 40 over the confirmed 70 and re-reads every range.
  fireEvent.click(reset());
  await waitFor(() => expect(saved().textContent).toBe('40 WPM (200 characters per minute)'));
  expect(store.set.mock.calls).toEqual([[70], [70], [40]]);
  expect(invalidated()).toEqual([
    ['metrics', 'dashboard'],
    ['metrics', 'dashboard'],
  ]);
  expect(reset().hasAttribute('disabled')).toBe(true);
});

it('settles an unknown save as failed when a check reads the old speed, keeping the edit to retry', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store, false);
  fireEvent.click(check());
  expect(
    (
      await screen.findByText(
        'The typing speed could not be saved. The speed shown is the saved one.',
      )
    ).textContent,
  ).toBeTruthy();
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  expect(screen.getByText('Saved')).toBeTruthy();
  expect(saved().textContent).toBe('40 WPM (200 characters per minute)');
  expect(field().value).toBe('70');
  // Already the default: Reset has nothing to write; Save retries the edit.
  expect(reset().hasAttribute('disabled')).toBe(true);
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved().textContent).toBe('70 WPM (350 characters per minute)'));
  expect(store.set.mock.calls).toEqual([[70], [70]]);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invalidated()).toEqual([
    ['metrics', 'dashboard'],
    ['metrics', 'dashboard'],
  ]);
});

it('stays unknown when a check cannot read the setting, and a later check settles it', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store);
  store.read.mockRejectedValueOnce(new Error('application storage operation failed'));
  fireEvent.click(check());
  await waitFor(() => expect(store.read).toHaveBeenCalledTimes(3));
  await waitFor(() => expect(check().hasAttribute('disabled')).toBe(false));
  expect(screen.getByRole('alert').textContent).toBe(UNKNOWN_70);
  expect(screen.getByText('Last confirmed')).toBeTruthy();
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
  // A check never refreshes the Dashboard again.
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  fireEvent.click(check());
  await waitFor(() => expect(screen.getByText('Saved')).toBeTruthy());
  expect(saved().textContent).toBe('70 WPM (350 characters per minute)');
  expect(store.set.mock.calls).toEqual([[70], [70]]);
});

it('lets no setting query read update or settle an unknown save', async () => {
  const store = controls();
  const mounted = vi.spyOn(QueryClient.prototype, 'mount');
  // A background read starts before the save and answers after it: whatever it
  // says, it was not read after the write, so it must not confirm anything.
  let answer: ((wpm: number) => void) | undefined;
  commitThenLoseReply(store);
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('40'));
  const client = mounted.mock.contexts[0] as QueryClient;
  store.read.mockImplementationOnce(() => new Promise<number>((resolve) => (answer = resolve)));
  await act(async () => void client.invalidateQueries({ queryKey: queryKeys.typingSpeed }));
  expect(store.read).toHaveBeenCalledTimes(2);
  store.read.mockRejectedValueOnce(new Error('application storage operation failed'));
  fireEvent.change(field(), { target: { value: '70' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(UNKNOWN_70);
  await act(async () => answer?.(70));
  // Neither the stale answer nor a later refetch reads or changes anything.
  await act(async () => {
    await client.invalidateQueries({ queryKey: queryKeys.typingSpeed });
    await client.refetchQueries({ queryKey: queryKeys.typingSpeed });
  });
  expect(store.read).toHaveBeenCalledTimes(3);
  expect(screen.getByRole('alert').textContent).toBe(UNKNOWN_70);
  expect(screen.getByText('Last confirmed')).toBeTruthy();
  expect(client.getQueryData(queryKeys.typingSpeed)).toBe(40);
  // Only the user's check settles it, from its own read.
  fireEvent.click(check());
  await waitFor(() => expect(saved().textContent).toBe('70 WPM (350 characters per minute)'));
  expect(store.read).toHaveBeenCalledTimes(4);
  expect(screen.queryByRole('alert')).toBeNull();
});

it('sends one save at a time', async () => {
  const store = controls();
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('40'));
  fireEvent.change(field(), { target: { value: '80' } });
  act(() => {
    field().form!.requestSubmit();
    field().form!.requestSubmit();
  });
  await waitFor(() => expect(saved().textContent).toBe('80 WPM (400 characters per minute)'));
  expect(store.set).toHaveBeenCalledExactlyOnceWith(80);
});

it('shows an unreadable setting as unavailable with no control to change it', async () => {
  const store = controls();
  store.read.mockRejectedValue(new Error('application storage operation failed'));
  await mount(source(store));
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The typing speed could not be read.',
  );
  expect(saved().textContent).toBe('Unavailable');
  expect(field().hasAttribute('disabled')).toBe(true);
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
});

it('opens the official test through the native seam without navigating the app', async () => {
  const store = controls();
  await mount(source(store));
  link().focus();
  expect(document.activeElement).toBe(link());
  const navigated = fireEvent.click(link());
  expect(navigated).toBe(false);
  expect(store.openTest).toHaveBeenCalledOnce();
  expect(store.openTest).toHaveBeenCalledWith();
  expect(window.location.pathname).not.toContain('monkeytype');
  store.openTest.mockRejectedValueOnce(new Error('the typing test could not be opened'));
  fireEvent.click(link());
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The typing test could not be opened in your browser.',
  );
});

it('keeps the link but no control where there is no app database', async () => {
  await mount(source());
  expect(screen.getByText(/It cannot be changed in this preview/).textContent).toContain(
    '40 WPM (200 characters per minute)',
  );
  expect(screen.queryByRole('textbox', { name: 'Words per minute' })).toBeNull();
  expect(link().getAttribute('href')).toBe(TYPING_TEST_URL);
  // A plain new-tab link here: the browser handles it.
  expect(fireEvent.click(link())).toBe(true);
});

it('a committed change invalidates every Dashboard range and nothing else', async () => {
  const client = new QueryClient();
  const keys = [
    queryKeys.dashboard(7),
    queryKeys.dashboard(14),
    queryKeys.dashboard(30),
    queryKeys.today,
    queryKeys.environment(7),
    queryKeys.tokensByHost(7),
    queryKeys.sessions(7),
    queryKeys.contentRetention,
  ];
  for (const key of keys) client.setQueryData(key, {});
  await client.invalidateQueries({ queryKey: queryKeys.dashboards, refetchType: 'none' });
  expect(keys.map((key) => client.getQueryState(key)?.isInvalidated)).toEqual([
    true,
    true,
    true,
    false,
    false,
    false,
    false,
    false,
  ]);
});

it('invokes the native typing-speed commands with plain integers', async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'open_typing_test' ? undefined : 80,
  );
  const { typingSpeed } = new TauriDataSource();
  expect(await typingSpeed.read()).toBe(80);
  expect(await typingSpeed.set(80)).toBe(80);
  await typingSpeed.openTest();
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['typing_speed'],
    ['set_typing_speed', { wpm: 80 }],
    ['open_typing_test'],
  ]);
});
