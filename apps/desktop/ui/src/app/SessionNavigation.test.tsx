import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { DashboardLane } from '../data/generated/DashboardLane';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionRow } from '../data/generated/SessionRow';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const template = exported.sessions[0].rows[0];
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

/**
 * Synthetic sessions, written here rather than copied from a real history.
 * The second one is an ID that carries query-string punctuation, which a
 * host's own identifier may.
 */
const ATLAS = '11111111-0000-4000-8000-000000000001';
const AWKWARD = 'codex-branch a&b';
const rows: SessionRow[] = [
  {
    ...template,
    id: ATLAS,
    host: 'claude',
    repo: '/Users/example/code/atlas',
    branch: 'feat/navigation',
    model: 'model-a',
  },
  { ...template, id: AWKWARD, host: 'codex', repo: null, branch: null, model: null },
];
const lanes: DashboardLane[] = [
  { session_id: ATLAS, host: 'claude', start_ms: 0, end_ms: 0 },
  { session_id: AWKWARD, host: 'codex', start_ms: 0, end_ms: 0 },
  // A session with spans on the recent axis but nothing inside any selected
  // window: the link is still offered, and the search simply finds nothing.
  { session_id: 'gone-from-the-window', host: 'cursor', start_ms: 0, end_ms: 0 },
];

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};

/** A source whose list filters the way the store's own query does. */
function navigationSource() {
  const calls: { search: string; hosts: string[] | null; withPrs?: boolean; days: number }[] = [];
  const opened: string[] = [];
  const cancelled: string[] = [];
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
    nativeIndexStatus: async () => exported.native_index,
    dashboard: async (days) => {
      const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
      report.lanes = lanes.map((lane) => ({
        ...lane,
        start_ms: report.lane_start_ms + 600_000,
        end_ms: report.lane_start_ms + 1_200_000,
      }));
      report.lanes_total = lanes.length;
      return report;
    },
    tokensByHost: async (days) => {
      const report = exported.dashboards.find((r) => r.window.days === days)!;
      return { window: report.window, hosts: report.tokens_by_host };
    },
    today: async () => exported.today,
    environment: async (days) => exported.environments.find((e) => e.window.days === days)!,
    sessionsList: async ({ search, hosts, withPrs }, _after, days) => {
      calls.push({ search, hosts, withPrs, days });
      const page = exported.sessions.find((entry) => entry.window.days === days)!;
      return {
        window: page.window,
        next: null,
        rows: rows.filter(
          (row) =>
            (!hosts || hosts.includes(row.host)) &&
            (!withPrs || row.pr_links.length > 0) &&
            [row.id, row.repo, row.branch, row.title].some((value) =>
              value?.toLowerCase().includes(search.toLowerCase()),
            ),
        ),
      };
    },
    sessionStretches: async () => ({
      state: 'measured',
      stretches: [],
      repeat_thresholds: { active_ms: 240_000, repeats: 5 },
    }),
    sessionRow: async (sessionId, days) => {
      calls.push({ search: sessionId, hosts: null, days });
      return rows.find((row) => row.id === sessionId) ?? null;
    },
    sessionTranscript: async (sessionId) => {
      opened.push(sessionId);
      return sessionId === ATLAS
        ? {
            state: 'loaded',
            generation: { generation: 'indexed', appended: false },
            sources: 1,
            dropped_records: 0,
            gaps: [],
            records: [
              {
                id: 'aaaaaaaa-0000-4000-8000-00000000000a',
                role: 'user',
                at: null,
                blocks: [{ kind: 'text', index: 0, text: 'a synthetic question' }],
              },
            ],
          }
        : { state: 'unavailable', reason: { reason: 'missing' } };
    },
    cancelSessionTranscript: async (readId) => void cancelled.push(readId),
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async () => () => {},
  };
  return { source, calls, opened, cancelled };
}

/** Drives the router's own history, the way the browser's buttons do. */
function History() {
  const navigate = useNavigate();
  return (
    <>
      <button type="button" onClick={() => void navigate(-1)}>
        history back
      </button>
      <button type="button" onClick={() => void navigate(1)}>
        history forward
      </button>
    </>
  );
}

function mount(source: DataSource, path = '/dashboard') {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <History />
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

const laneLink = (id: string) => screen.getByRole('link', { name: `Open session ${id}` });
const backLink = () => screen.getByRole('link', { name: '← All sessions' });
const searchBox = () => screen.getByLabelText('Search sessions') as HTMLInputElement;
const hostFilter = () => screen.getByRole('button', { name: 'Filter by host' });
/** The hosts the compact menu shows as selected, by their glyphs' names. */
const selectedHosts = () =>
  within(hostFilter())
    .getAllByRole('img', { hidden: true })
    .map((glyph) => glyph.getAttribute('aria-label'));
/** Check or uncheck one host in the menu, as a keyboard or pointer user would. */
async function toggleHost(label: string) {
  fireEvent.click(hostFilter());
  fireEvent.click(await screen.findByRole('checkbox', { name: label }));
}

it('opens a Dashboard session by name, and offers the list that holds it', async () => {
  const { source, calls, opened } = navigationSource();
  mount(source);
  await screen.findByTestId('dashboard-summary');
  // The Dashboard has no repository or branch for a session, so a lane names
  // the identity it does have; the link says which session, in full.
  const link = laneLink(ATLAS);
  expect(link.getAttribute('href')).toBe(
    `/sessions/${encodeURIComponent(ATLAS)}?q=${encodeURIComponent(ATLAS)}&host=claude&range=7d`,
  );
  // A real link, reachable and operable from the keyboard: focusing it and
  // pressing Enter is the click jsdom does not synthesize for an anchor.
  link.focus();
  expect(document.activeElement).toBe(link);
  fireEvent.click(link);

  // The session itself, named by what the index knows about it, with the
  // transcript read for this open and nothing chosen for the reader.
  await screen.findByRole('heading', { name: 'atlas · feat/navigation' });
  await waitFor(() => expect(opened).toEqual([ATLAS]));
  expect(await screen.findByText('a synthetic question')).toBeTruthy();
  // Its measurements are the list's, over the window the Dashboard showed.
  // The host filter is not applied: the session is named exactly, and a
  // filter could only hide the one row this page is about.
  await waitFor(() => expect(calls.at(-1)).toEqual({ search: ATLAS, hosts: null, days: 7 }));

  // Back is a list that holds this session, filtered as the link said.
  expect(backLink().getAttribute('href')).toBe(
    `/sessions?q=${encodeURIComponent(ATLAS)}&host=claude&range=7d`,
  );
  fireEvent.click(backLink());
  await screen.findByRole('heading', { name: 'Sessions' });
  expect(searchBox().value).toBe(ATLAS);
  expect(selectedHosts()).toEqual(['Claude']);
  const named = await screen.findByText('Session 11111111');
  const row = named.closest('[role="row"]')!;
  // The row opens the same session, and says so in full.
  expect(
    within(row as HTMLElement)
      .getByRole('link', { name: `Open session ${ATLAS}` })
      .getAttribute('href'),
  ).toBe(
    `/sessions/${encodeURIComponent(ATLAS)}?q=${encodeURIComponent(ATLAS)}&host=claude&range=7d`,
  );
});

it('keeps the selected range when opening a session from the Dashboard', async () => {
  const { source, calls } = navigationSource();
  mount(source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(laneLink(ATLAS).getAttribute('href')).toContain('range=30d'));
  fireEvent.click(laneLink(ATLAS));
  await screen.findByRole('heading', { name: 'atlas · feat/navigation' });
  // The session's measurements are taken over the window that was selected,
  // and the page says which window that was.
  await waitFor(() => expect(calls.at(-1)).toEqual({ search: ATLAS, hosts: null, days: 30 }));
  // The row and the stretches both say which window they were measured over.
  expect(await screen.findByText(/^Records .* measured over the last 30 days$/)).toBeTruthy();
  expect(screen.getByText('Measured over the last 30 days')).toBeTruthy();
  // A range followed through a link is this window's range from then on, so
  // leaving for a page whose address carries none keeps the same window.
  fireEvent.click(screen.getByRole('button', { name: 'Dashboard' }));
  await screen.findByTestId('dashboard-summary');
  await waitFor(() =>
    expect(screen.getByRole('radio', { name: '30d' }).getAttribute('aria-checked')).toBe('true'),
  );
});

it('keeps a session ID that carries path and query punctuation intact', async () => {
  const { source, opened } = navigationSource();
  mount(source);
  await screen.findByTestId('dashboard-summary');
  const link = laneLink(AWKWARD);
  const href = link.getAttribute('href')!;
  // The identity is one path segment, so its slash cannot become a route of
  // its own, and its ampersand and space cannot split or end the query.
  expect(href).toBe(`/sessions/codex-branch%20a%26b?q=codex-branch+a%26b&host=codex&range=7d`);
  expect(href).not.toContain(' ');
  fireEvent.click(link);
  // The whole identity reaches the read, not a prefix and not a decoded half.
  await waitFor(() => expect(opened).toEqual([AWKWARD]));
  expect(await screen.findByText(AWKWARD)).toBeTruthy();
  // No repository or branch is known, and nothing is invented to fill the line.
  expect(screen.getByRole('heading', { name: 'Unknown repository' })).toBeTruthy();
  expect(screen.getByText('Unknown model')).toBeTruthy();
});

it('says plainly when an opened session is not in the index, and shows no transcript', async () => {
  const { source } = navigationSource();
  mount(source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(laneLink('gone-from-the-window'));
  expect(await screen.findByText(/holds no session with this identifier/)).toBeTruthy();
  // The text is unavailable, and is never worded as a session that said
  // nothing: the two are different facts about somebody's work.
  expect(await screen.findByText('This transcript is not available to show.')).toBeTruthy();
  expect(screen.queryByText('This view was given no turns to show.')).toBeNull();
  fireEvent.click(backLink());
  await screen.findByRole('heading', { name: 'Sessions' });
});

it('cancels the read it started when the reader leaves the session', async () => {
  const { source, cancelled } = navigationSource();
  mount(source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(laneLink(ATLAS));
  await screen.findByText('a synthetic question');
  expect(cancelled).toEqual([]);
  fireEvent.click(backLink());
  await screen.findByRole('heading', { name: 'Sessions' });
  // Exactly the read this open started, named by this open.
  await waitFor(() => expect(cancelled.length).toBe(1));
  expect(cancelled[0]).toMatch(/^read-\d+-/);
});

it('restores a followed list with back and forward', async () => {
  const { source } = navigationSource();
  mount(source, `/sessions?q=${encodeURIComponent(ATLAS)}&host=claude&range=7d`);
  await screen.findByRole('heading', { name: 'Sessions' });
  expect(searchBox().value).toBe(ATLAS);
  fireEvent.click(await screen.findByRole('link', { name: `Open session ${ATLAS}` }));
  await screen.findByRole('heading', { name: 'atlas · feat/navigation' });

  fireEvent.click(screen.getByRole('button', { name: 'history back' }));
  await screen.findByRole('heading', { name: 'Sessions' });
  // The filters come back from the address, not from anything kept in memory.
  await waitFor(() => expect(searchBox().value).toBe(ATLAS));
  expect(selectedHosts()).toEqual(['Claude']);
  fireEvent.click(screen.getByRole('button', { name: 'history forward' }));
  await screen.findByRole('heading', { name: 'atlas · feat/navigation' });
});

it('lets a manual search replace a followed one, and back undo the page', async () => {
  const { source, calls } = navigationSource();
  mount(source, `/sessions?q=${encodeURIComponent(ATLAS)}&host=claude&range=7d`);
  await screen.findByRole('heading', { name: 'Sessions' });
  await waitFor(() => expect(calls.at(-1)?.search).toBe(ATLAS));
  // Typing over a followed search wins; the address follows the box.
  fireEvent.change(searchBox(), { target: { value: 'atlas' } });
  await waitFor(() =>
    expect(calls.at(-1)).toEqual({ search: 'atlas', hosts: ['claude'], withPrs: false, days: 7 }),
  );
  expect(searchBox().value).toBe('atlas');
  // Filtering is not a page: the entry is replaced, not stacked.
  expect(screen.getByRole('link', { name: `Open session ${ATLAS}` })).toBeTruthy();
});

it('ignores an unusable host or range in the address without breaking the filters', async () => {
  const { source, calls } = navigationSource();
  mount(source, '/sessions?q=atlas&host=pretend-host&range=99d');
  await screen.findByRole('heading', { name: 'Sessions' });
  // An unsupported host is no filter, and an unsupported range falls back to
  // the default window, rather than sending either to a query that fails.
  await waitFor(() =>
    expect(calls.at(-1)).toEqual({ search: 'atlas', hosts: null, withPrs: false, days: 7 }),
  );
  expect(selectedHosts()).toEqual(['Claude', 'Codex', 'Cursor']);
  expect(screen.getByRole('radio', { name: '7d' }).getAttribute('aria-checked')).toBe('true');
  expect(await screen.findByText('Session 11111111')).toBeTruthy();
  // The filters still work by hand from there: unchecking Claude leaves a
  // set of two, applied by the query rather than to loaded rows.
  await toggleHost('Claude Code');
  await waitFor(() =>
    expect(calls.at(-1)).toEqual({
      search: 'atlas',
      hosts: ['codex', 'cursor'],
      withPrs: false,
      days: 7,
    }),
  );
  expect(await screen.findByText('No sessions match these filters.')).toBeTruthy();
});

it('carries a host set and the pull-request filter into a session and back again', async () => {
  const { source, calls } = navigationSource();
  mount(source, '/sessions?host=claude,codex&with_prs=1&range=7d');
  await screen.findByRole('heading', { name: 'Sessions' });
  await waitFor(() =>
    expect(calls.at(-1)).toEqual({
      search: '',
      hosts: ['claude', 'codex'],
      withPrs: true,
      days: 7,
    }),
  );
  expect(selectedHosts()).toEqual(['Claude', 'Codex']);
  const toggle = screen.getByRole('switch', { name: 'With PRs only' });
  expect(toggle.getAttribute('aria-checked')).toBe('true');
  const open = await screen.findByRole('link', { name: `Open session ${ATLAS}` });
  expect(open.getAttribute('href')).toBe(
    `/sessions/${encodeURIComponent(ATLAS)}?host=claude%2Ccodex&with_prs=1&range=7d`,
  );
  fireEvent.click(open);
  await screen.findByRole('heading', { name: 'atlas · feat/navigation' });
  expect(backLink().getAttribute('href')).toBe('/sessions?host=claude%2Ccodex&with_prs=1&range=7d');
  fireEvent.click(backLink());
  await screen.findByRole('heading', { name: 'Sessions' });
  expect(selectedHosts()).toEqual(['Claude', 'Codex']);
  expect(screen.getByRole('switch', { name: 'With PRs only' }).getAttribute('aria-checked')).toBe(
    'true',
  );
  // Turning the filter off is keyboard-operable and leaves the address, so
  // Back from the next session restores the unfiltered list.
  const again = screen.getByRole('switch', { name: 'With PRs only' });
  again.focus();
  expect(document.activeElement).toBe(again);
  fireEvent.click(again);
  await waitFor(() => expect(calls.at(-1)).toMatchObject({ withPrs: false }));
  expect(
    (await screen.findByRole('link', { name: `Open session ${ATLAS}` })).getAttribute('href'),
  ).toBe(`/sessions/${encodeURIComponent(ATLAS)}?host=claude%2Ccodex&range=7d`);
});
