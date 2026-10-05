import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, SessionListFilter, SessionTitleControls } from '../data/DataSource';
import type { DashboardLaneSession } from '../data/generated/DashboardLaneSession';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionPage } from '../data/generated/SessionPage';
import type { SessionParentLink } from '../data/generated/SessionParentLink';
import type { SessionRow } from '../data/generated/SessionRow';
import type { SessionTitles } from '../data/generated/SessionTitles';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';

/**
 * Verified sub-sessions on Sessions and on the Dashboard's lanes, over
 * synthetic sessions written here (not real history). A child names its
 * exact parent inside its own name cell and stays its own row: its link,
 * measurements, search and page membership are the report's, and a parent the
 * report did not return is named, never added.
 */

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));

const exported = fixture as FixtureExport;
const PARENT = '01a0aaaa-0000-7000-8000-000000000001';
const CHILD = '01a0bbbb-0000-7000-8000-000000000002';
const OTHER = '01a0cccc-0000-7000-8000-000000000003';
const enc = encodeURIComponent;

const link = (patch: Partial<SessionParentLink> = {}): SessionParentLink => ({
  session_id: PARENT,
  host: 'codex',
  title: null,
  evidence: 'native_spawn',
  ...patch,
});

/** A row with its own output count, so each row's measurement is told apart. */
function row(id: string, output: number, patch: Partial<SessionRow> = {}): SessionRow {
  const template = structuredClone(exported.sessions[0].rows[0]);
  if (template.metrics.state === 'indexed') template.metrics.tokens.counters.output_tokens = output;
  return {
    ...template,
    id,
    host: 'codex',
    title: null,
    repo: `/code/${id.slice(4, 8)}-repo`,
    branch: 'main',
    pr_links: [],
    parent: null,
    ...patch,
  };
}

/** One host-title read the test answers by hand. */
interface Pending {
  ids: readonly string[];
  readId: string;
  resolve: (answer: SessionTitles) => void;
}
function titleControls() {
  const reads: Pending[] = [];
  const controls: SessionTitleControls = {
    read: (ids, readId) => new Promise((resolve) => reads.push({ ids: [...ids], readId, resolve })),
    cancel: async () => {},
  };
  return { controls, reads };
}
const answer = (read: Pending, titles: Record<string, string>) =>
  act(async () => {
    read.resolve({ titles: Object.entries(titles).map(([id, title]) => ({ id, title })) });
  });

const unavailable = async (): Promise<never> => {
  throw new Error('Not part of this test.');
};

/**
 * A source whose list filters and pages the way the store's own query does:
 * each filtered page is exactly the rows it was given, in order, and the
 * exact-row read finds any session by its whole identity, listed or not.
 */
function source({
  pages = [[]],
  everyRow = pages.flat(),
  dashboard,
  titles,
}: {
  pages?: SessionRow[][];
  everyRow?: SessionRow[];
  dashboard?: (days: number) => DashboardMetrics;
  titles?: SessionTitleControls;
}) {
  const listed: { filter: SessionListFilter; after: string | null; days: number }[] = [];
  const opened: { id: string; days: number }[] = [];
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
    nativeIndexStatus: async () => exported.native_index,
    sessionsList: async (filter, after, days): Promise<SessionPage> => {
      listed.push({ filter, after, days });
      const index = after ? Number(after) : 0;
      const matches = (candidate: SessionRow) =>
        (!filter.hosts || filter.hosts.includes(candidate.host)) &&
        [candidate.id, candidate.repo, candidate.branch, candidate.title].some((value) =>
          value?.toLowerCase().includes(filter.search.toLowerCase()),
        );
      return {
        window: exported.sessions.find((page) => page.window.days === days)!.window,
        rows: pages[index].filter(matches),
        next: index + 1 < pages.length ? String(index + 1) : null,
      };
    },
    dashboard: async (days) => {
      if (dashboard) return dashboard(days);
      const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
      // Sessions parent tests name only their loaded table rows. Dashboard
      // tests below supply explicit lanes for the activity cases.
      report.lanes = [];
      report.lane_sessions = [];
      report.lanes_total = 0;
      report.lanes_truncated = false;
      return report;
    },
    tokensByHost: async (days) => {
      const report = exported.dashboards.find((r) => r.window.days === days)!;
      return { window: report.window, hosts: report.tokens_by_host };
    },
    today: async () => exported.today,
    environment: async (days) => exported.environments.find((r) => r.window.days === days)!,
    sessionRow: async (id, days) => {
      opened.push({ id, days });
      return everyRow.find((candidate) => candidate.id === id) ?? null;
    },
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    subscribe: async () => () => {},
    ...(titles ? { titles } : {}),
  };
  return { data, listed, opened };
}

function mount(data: DataSource, path: string) {
  return render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

/** An IntersectionObserver the test drives: lane rows are in view when it says. */
class FakeObserver {
  static all: FakeObserver[] = [];
  readonly targets = new Set<Element>();
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeObserver.all.push(this);
  }
  observe(target: Element) {
    this.targets.add(target);
  }
  unobserve(target: Element) {
    this.targets.delete(target);
  }
  disconnect() {
    this.targets.clear();
  }
  takeRecords() {
    return [];
  }
  show(ids: readonly string[]) {
    const entries = [...this.targets].map(
      (target) =>
        ({
          target,
          isIntersecting: ids.includes((target as HTMLElement).dataset.visibleId ?? ''),
        }) as IntersectionObserverEntry,
    );
    act(() => this.callback(entries, this as unknown as IntersectionObserver));
  }
}
beforeEach(() => {
  FakeObserver.all = [];
  vi.stubGlobal('IntersectionObserver', FakeObserver);
  return () => vi.unstubAllGlobals();
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

const parentLabel = (name: string) => `Sub-session of ${name}, open parent session ${PARENT}`;
const markers = () => screen.queryAllByRole('link', { name: /^Sub-session of / });
const sessionsTable = () => screen.getByRole('table', { name: 'Indexed sessions' });
/** Each listed row's own identity link and its own output cell, in order. */
const listed = () =>
  within(sessionsTable())
    .getAllByRole('link', { name: /^Open session / })
    .map((open) => {
      const cells = within(open.closest<HTMLElement>('[role="row"]')!).getAllByRole('cell');
      return [open.getAttribute('aria-label'), cells.at(-1)!.textContent];
    });

it('names a parent on the same Sessions page by its row, host title once read, and keeps both rows', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link({ title: 'Saved parent' }) });
  const parent = row(PARENT, 999, { title: 'Saved parent' });
  const { data } = source({ pages: [[child, parent]], titles: controls });
  mount(data, '/sessions?q=01a0&host=codex&range=14d');

  // Until the title read answers, the parent is named as its own row reads:
  // its saved title.
  const marker = await screen.findByRole('link', { name: parentLabel('Saved parent') });
  expect(marker.textContent).toBe('↳Sub-session of Saved parent');
  // The exact parent route, carrying this list's own address.
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}?q=01a0&host=codex&range=14d`);
  expect(marker.getAttribute('title')).toBe(`Sub-session of Saved parent · ${PARENT}`);
  // The child is its own row with its own link and measurement; the parent is
  // its own row, once, where the report put it. Only the child has a marker.
  expect(listed()).toEqual([
    [`Open session ${CHILD}`, '111'],
    ['Open session Saved parent, ' + PARENT, '999'],
  ]);
  expect(markers()).toHaveLength(1);

  // One read names the page's rows, parent included because it is listed.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([CHILD, PARENT]);
  await answer(reads[0], { [PARENT]: 'Parent host title', [CHILD]: 'Child host title' });
  expect(screen.getByRole('link', { name: parentLabel('Parent host title') })).toBe(marker);
  expect(listed()).toEqual([
    [`Open session Child host title, ${CHILD}`, '111'],
    [`Open session Parent host title, ${PARENT}`, '999'],
  ]);
  expect(reads).toHaveLength(1);
});

it('names a filtered-out parent by what the report sent, reads nothing for it and never adds it', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link({ title: 'Saved parent' }) });
  const parent = row(PARENT, 999, { title: 'Saved parent', repo: '/code/elsewhere' });
  const { data, listed: calls } = source({ pages: [[child, parent]], titles: controls });
  mount(data, `/sessions?q=${CHILD}`);

  const marker = await screen.findByRole('link', { name: parentLabel('Saved parent') });
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}?q=${enc(CHILD)}`);
  expect(listed()).toEqual([[`Open session ${CHILD}`, '111']]);
  // Every read is the search the address holds; the parent is not searched for.
  expect(new Set(calls.map((call) => call.filter.search))).toEqual(new Set([CHILD]));
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([CHILD]);
  // An answer naming a row it was not asked about is not used.
  await answer(reads[0], { [PARENT]: 'Unasked parent title' });
  expect(screen.getByRole('link', { name: parentLabel('Saved parent') })).toBe(marker);
  expect(screen.queryByText('Unasked parent title')).toBeNull();
});

it('keeps a child on its own page while its parent is on a later one, then names the loaded parent', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link() });
  const parent = row(PARENT, 999);
  const { data, listed: calls } = source({
    pages: [[child, row(OTHER, 5)], [parent]],
    titles: controls,
  });
  mount(data, '/sessions');

  // No saved title anywhere: the parent's short identity.
  const marker = await screen.findByRole('link', { name: parentLabel('Session 01a0aaaa') });
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}`);
  expect(listed().map(([name]) => name)).toEqual([
    `Open session ${CHILD}`,
    `Open session ${OTHER}`,
  ]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([CHILD, OTHER]);
  await answer(reads[0], {});

  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  // The parent arrives on its own page, below; nothing moved, and the child
  // is still listed once with its own measurement.
  await waitFor(() => expect(listed()).toHaveLength(3));
  expect(listed()).toEqual([
    [`Open session ${CHILD}`, '111'],
    [`Open session ${OTHER}`, '5'],
    [`Open session ${PARENT}`, '999'],
  ]);
  // Only the list's own pages were read, the next one by its cursor.
  expect(new Set(calls.map((call) => call.after))).toEqual(new Set([null, '1']));
  expect(new Set(calls.map((call) => call.filter.search))).toEqual(new Set(['']));
  // Its own row's title read then names it in the child's marker too.
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual([PARENT]);
  await answer(reads[1], { [PARENT]: 'Parent host title' });
  expect(screen.getByRole('link', { name: parentLabel('Parent host title') })).toBe(marker);
});

it('leaves a row with no verified parent ordinary', async () => {
  const unset = row(OTHER, 5);
  delete unset.parent;
  const { data } = source({
    pages: [
      [
        row(CHILD, 111, { parent: null }),
        unset,
        // Never shown as its own parent, whatever a report says.
        row(PARENT, 999, { parent: link() }),
      ],
    ],
  });
  mount(data, '/sessions');
  await screen.findByRole('link', { name: `Open session ${PARENT}` });
  expect(markers()).toHaveLength(0);
  expect(screen.queryByText(/Sub-session/)).toBeNull();
  expect(document.querySelector('.xt-session-line')).toBeNull();
});

it('names a Codex parent that launched a Claude session exactly as any other verified parent', async () => {
  const CLAUDE_CHILD = '0c000000-0000-4000-8000-0000000000c1';
  const child = row(CLAUDE_CHILD, 111, {
    host: 'claude',
    parent: link({ title: 'Saved parent', evidence: 'agent_launch' }),
  });
  const { data } = source({ pages: [[child, row(PARENT, 999, { title: 'Saved parent' })]] });
  mount(data, '/sessions');

  const marker = await screen.findByRole('link', { name: parentLabel('Saved parent') });
  expect(marker.textContent).toBe('↳Sub-session of Saved parent');
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}`);
  // The kind of proof adds no role, task or launch wording to the row.
  expect(marker.closest('[role="row"]')!.textContent).not.toMatch(/launch|review|role|task/i);
  expect(listed()).toEqual([
    [`Open session ${CLAUDE_CHILD}`, '111'],
    ['Open session Saved parent, ' + PARENT, '999'],
  ]);
  expect(markers()).toHaveLength(1);
});

it('opens the exact parent from the keyboard and comes back to the same list', async () => {
  const child = row(CHILD, 111, { parent: link() });
  const parent = row(PARENT, 999, { repo: '/code/parent-repo', branch: 'trunk' });
  const { data, opened } = source({ pages: [[child]], everyRow: [child, parent] });
  mount(data, '/sessions?q=01a0bbbb&host=codex&range=30d');

  const open = await screen.findByRole('link', { name: `Open session ${CHILD}` });
  const marker = screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') });
  // A real link in the tab order, right after the row's own.
  expect(open.compareDocumentPosition(marker) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(marker.tabIndex).toBe(0);
  marker.focus();
  expect(document.activeElement).toBe(marker);
  fireEvent.click(marker);

  await screen.findByRole('heading', { name: 'parent-repo · trunk' });
  await waitFor(() => expect(opened).toEqual([{ id: PARENT, days: 30 }]));
  const back = screen.getByRole('link', { name: '← All sessions' });
  expect(back.getAttribute('href')).toBe('/sessions?q=01a0bbbb&host=codex&range=30d');
  fireEvent.click(back);
  await screen.findByRole('link', { name: `Open session ${CHILD}` });
  expect((screen.getByLabelText('Search sessions') as HTMLInputElement).value).toBe('01a0bbbb');
  expect(screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') })).toBeTruthy();
});

/** A lane report of these sessions, newest first, with this context. */
/** A lane session's cost, every response priced. */
const laneCost = (total: number) => ({
  total_usd: total,
  priced_subtotal_usd: total,
  selected_observations: 1,
  priced_observations: 1,
  unpriced_observations: 0,
  assumed_tier_observations: 0,
  unpriced: [],
});

function lanesReport(
  days: number,
  lanes: { id: string; context?: Partial<DashboardLaneSession> | null }[],
  cap?: { total: number },
): DashboardMetrics {
  const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
  const minute = 60_000;
  report.lanes = lanes.map(({ id }, index) => ({
    session_id: id,
    host: 'codex',
    start_ms: report.lane_end_ms - (index + 1) * 30 * minute,
    end_ms: report.lane_end_ms - (index + 1) * 30 * minute + 10 * minute,
  }));
  report.lanes_total = cap?.total ?? lanes.length;
  report.lanes_truncated = cap !== undefined;
  report.lane_sessions = lanes
    .filter(({ context }) => context !== null)
    .map(({ id, context }) => ({
      session_id: id,
      host: 'codex',
      repo: `/code/${id.slice(4, 8)}-repo`,
      branch: 'main',
      title: null,
      started_at_ms: null,
      pr_links: 0,
      inferred_pr_links: 0,
      cost: laneCost(id === CHILD ? 1.11 : 9.99),
      parent: null,
      ...context,
      automated_review: context?.automated_review ?? false,
    }));
  return report;
}

const laneTable = () => screen.getByRole('table', { name: 'Session lanes' });
/** Each lane's own name link and cost, in order. */
const laneRows = () =>
  within(laneTable())
    .getAllByRole('link', { name: /^Open session / })
    .map((open) => {
      const cells = within(open.closest<HTMLElement>('[role="row"]')!).getAllByRole('cell');
      return [open.textContent, cells.at(-1)!.textContent];
    });

const SECOND = '01a0dddd-0000-7000-8000-000000000004';
const groupToggle = (name: string) => within(laneTable()).getByRole('button', { name });
/** Every drawn body row: its text, and whether it has a lane track of its own. */
const drawn = () =>
  within(laneTable())
    .getAllByRole('row')
    .slice(1)
    .map((row) => row.querySelector('.xt-lane-name')!.firstChild!.textContent);

it('collapses a returned parent’s children under its own row and lists each as it was when opened', async () => {
  const lanes = (parent: SessionParentLink | null) => [
    { id: CHILD, context: { parent } },
    { id: OTHER },
    { id: PARENT, context: { title: 'Saved parent lane' } },
    { id: SECOND, context: { parent, cost: laneCost(2.22) } },
  ];
  // The same capped report without the relation, for comparison.
  const plain = source({ dashboard: (days) => lanesReport(days, lanes(null), { total: 40 }) });
  const first = mount(plain.data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  const before = laneRows();
  first.unmount();
  expect(before).toEqual([
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0cccc', '$9.99'],
    ['Saved parent lane', '$9.99'],
    ['Session 01a0dddd', '$2.22'],
  ]);

  const { controls, reads } = titleControls();
  const { data } = source({
    dashboard: (days) => lanesReport(days, lanes(link()), { total: 40 }),
    titles: controls,
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // Collapsed: the parent keeps its own row, measurement and link, placed
  // where its newest returned child was; neither child has a link in the page.
  expect(laneRows()).toEqual([
    ['Saved parent lane', '$9.99'],
    ['Session 01a0cccc', '$9.99'],
  ]);
  expect(markers()).toHaveLength(0);
  expect(within(laneTable()).queryByRole('link', { name: new RegExp(CHILD) })).toBeNull();
  const toggle = groupToggle('2 returned sub-sessions of Saved parent lane');
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  // Its children are rows of the same table, not one element it could name.
  expect(toggle.hasAttribute('aria-controls')).toBe(false);
  const parentRow = toggle.closest<HTMLElement>('[role="row"]')!;
  expect(parentRow.querySelector('.sr-only')!.textContent).toContain(
    '2 returned sub-sessions collapsed under this row.',
  );
  // No caption under the rows states the counts or where a group sits.
  expect(screen.queryByTestId('lanes-disclosure')).toBeNull();
  // Titles are read only for the rows drawn.
  FakeObserver.all.at(-1)!.show([CHILD, OTHER, PARENT, SECOND]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, PARENT]);
  await answer(reads[0], { [PARENT]: 'Parent host title' });
  expect(toggle.getAttribute('aria-label')).toBe('2 returned sub-sessions of Parent host title');

  // Opened: each child's own row, cost and link, newest first, under it.
  toggle.focus();
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(document.activeElement).toBe(toggle);
  expect(laneRows()).toEqual([
    ['Parent host title', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0dddd', '$2.22'],
    ['Session 01a0cccc', '$9.99'],
  ]);
  expect(markers()).toHaveLength(2);
  for (const marker of markers())
    expect(marker.getAttribute('aria-label')).toBe(parentLabel('Parent host title'));
  expect(screen.getByRole('link', { name: `Open session ${SECOND}` }).getAttribute('href')).toBe(
    `/sessions/${enc(SECOND)}?q=${enc(SECOND)}&host=codex&range=7d`,
  );
  const childName = screen
    .getByRole('link', { name: `Open session ${CHILD}` })
    .closest<HTMLElement>('.xt-lane-name')!;
  expect(childName.style.getPropertyValue('--depth')).toBe('1');
  // The lane table's sized rows follow what is drawn.
  const lanesBox = document.querySelector<HTMLElement>('.xt-lanes')!;
  expect(lanesBox.style.getPropertyValue('--count')).toBe('4');

  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(markers()).toHaveLength(0);
  expect(lanesBox.style.getPropertyValue('--count')).toBe('2');
});

it('names an absent parent on one collapsed row with blank cells, reads nothing for it and adds no lane', async () => {
  const { controls, reads } = titleControls();
  const parent = link({ title: 'Saved parent' });
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [
        { id: OTHER },
        { id: CHILD, context: { parent } },
        { id: SECOND, context: { parent, cost: laneCost(2.22) } },
      ]),
    titles: controls,
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // The group sits at its newest returned child, after the newer ordinary row.
  expect(drawn()).toEqual(['Session 01a0cccc', '›2']);
  const toggle = groupToggle('2 returned sub-sessions of Saved parent');
  const row = toggle.closest<HTMLElement>('[role="row"]')!;
  expect(row.querySelector('.xt-lane-name')!.textContent).toMatch(
    /^›2Sub-sessions of Saved parent/,
  );
  // It opens the parent's own page, and claims no measurement, start or span.
  const open = within(row).getByRole('link', {
    name: `Open parent session Saved parent, ${PARENT}`,
  });
  expect(open.getAttribute('href')).toBe(
    `/sessions/${enc(PARENT)}?q=${enc(PARENT)}&host=codex&range=7d`,
  );
  const cells = within(row).getAllByRole('cell');
  expect(cells.slice(2).map((cell) => cell.textContent)).toEqual(['', '', '', '', '']);
  expect(row.querySelector('.xt-lane-track, .xt-lane-span, time, .xt-metric-cell')).toBeNull();
  expect(row.querySelector('.sr-only')!.textContent).toContain(
    `which has no span returned in this report: it is only named here, with no measurement of its own. 2 returned sub-sessions collapsed under this row. Only sub-sessions with returned spans are counted.`,
  );
  expect(laneRows()).toEqual([['Session 01a0cccc', '$9.99']]);

  fireEvent.click(toggle);
  expect(laneRows()).toEqual([
    ['Session 01a0cccc', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0dddd', '$2.22'],
  ]);
  expect(markers().map((marker) => marker.getAttribute('aria-label'))).toEqual([
    parentLabel('Saved parent'),
    parentLabel('Saved parent'),
  ]);
  // The absent parent is never read, drawn or observed.
  FakeObserver.all.at(-1)!.show([CHILD, SECOND, OTHER, PARENT]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, CHILD, SECOND]);
});

it('leaves a lane with no verified parent, or no context, ordinary', async () => {
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [
        { id: CHILD, context: { parent: null } },
        { id: OTHER, context: null },
        { id: PARENT, context: { parent: link() } },
      ]),
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(laneRows()).toHaveLength(3);
  expect(markers()).toHaveLength(0);
  expect(document.querySelector('.xt-lane-name[data-subsession]')).toBeNull();
  // Nothing is grouped, so nothing is behind a disclosure.
  expect(within(laneTable()).queryByRole('button', { name: /returned sub-session/ })).toBeNull();
});

it('opens an absent Dashboard parent from the keyboard with a Back that finds it', async () => {
  const parent = row(PARENT, 999, { repo: '/code/parent-repo', branch: 'trunk' });
  const { data, opened } = source({
    everyRow: [parent],
    dashboard: (days) => lanesReport(days, [{ id: CHILD, context: { parent: link() } }]),
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  const open = await waitFor(() => {
    const found = screen.getByRole('link', {
      name: `Open parent session Session 01a0aaaa, ${PARENT}`,
    });
    expect(found.getAttribute('href')).toContain('range=14d');
    return found;
  });
  // The one child is behind its disclosure; opened, its marker says the same.
  fireEvent.click(groupToggle('1 returned sub-session of Session 01a0aaaa'));
  expect(screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') })).toBeTruthy();
  open.focus();
  expect(document.activeElement).toBe(open);
  fireEvent.click(open);
  await screen.findByRole('heading', { name: 'parent-repo · trunk' });
  await waitFor(() => expect(opened).toEqual([{ id: PARENT, days: 14 }]));
  expect(screen.getByRole('link', { name: '← All sessions' }).getAttribute('href')).toBe(
    `/sessions?q=${enc(PARENT)}&host=codex&range=14d`,
  );
});
