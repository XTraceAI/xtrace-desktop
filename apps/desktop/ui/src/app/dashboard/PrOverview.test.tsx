import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { PrAutoCheckStatus } from '../../data/generated/PrAutoCheckStatus';
import type { PrList } from '../../data/generated/PrList';
import type { PrRefreshReport } from '../../data/generated/PrRefreshReport';
import type { PrRow } from '../../data/generated/PrRow';
import { ruleSummary } from '../../kit/rules';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import {
  F19_CONFIRMED,
  F19_SHARED,
  MANY_TYPES,
  NO_LINKS,
  PARTIAL_COST,
  UNKNOWN_FACTS,
  withPrEffort,
  type SectionSpec,
} from './pr-effort.synthetic';
import { DELTA_HIDDEN } from './present';
import { EFFORT_CONTEXT } from './DashboardPage';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const f1 = (days = 7) => structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
  localStorage.clear();
});

type Refresh = {
  pullRequests?: () => Promise<PrList>;
  refreshPullRequests?: (ids: number[]) => Promise<PrRefreshReport>;
  cancelPullRequestRefresh?: () => Promise<boolean>;
};
/** A native-shaped source over synthetic reports; nothing here reaches GitHub. */
function nativeSource(report: (days: number) => DashboardMetrics, refresh: Refresh = {}) {
  const refused = async (): Promise<never> => {
    throw new Error('not part of this test');
  };
  return {
    kind: 'native' as const,
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    dashboard: vi.fn(async (days: number) => report(days)),
    tokensByHost: async (days: number) => ({
      window: report(days).window,
      hosts: report(days).tokens_by_host,
    }),
    environment: async (days: number) =>
      exported.environments.find((entry) => entry.window.days === days)!,
    today: async () => exported.today,
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: vi.fn(refresh.pullRequests ?? refused),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    refreshPullRequests: vi.fn(refresh.refreshPullRequests ?? refused),
    cancelPullRequestRefresh: vi.fn(refresh.cancelPullRequestRefresh ?? refused),
    subscribe: async () => () => {},
  } satisfies DataSource;
}
const sectioned = (spec: SectionSpec) => (days: number) => withPrEffort(f1(days), spec);

/** Route changes for the tests, the way the app's own links make them. */
let go: (to: string) => void = () => {};
function Navigator() {
  const navigate = useNavigate();
  go = (to) => void navigate(to);
  return null;
}
const tree = (source: DataSource) => (
  <ThemeProvider>
    <DataProvider source={source}>
      <MemoryRouter initialEntries={['/dashboard']}>
        <Navigator />
        <AppRoutes />
      </MemoryRouter>
    </DataProvider>
  </ThemeProvider>
);
const mount = (source: DataSource) => render(tree(source));
const loaded = () => screen.findByTestId('dashboard-summary');
/** The Overview card's Merged PRs tile. */
const mergedTile = () =>
  screen.getAllByTestId('overview-tile').find((tile) => tile.dataset.label === 'Merged PRs')!;
/** The tile as it reads: its label, then its number (or words) and the line under it. */
const tile = () => ({
  textContent: `Merged PRs${mergedTile().querySelector('[data-testid="overview-value"]')!.textContent}${mergedTile().querySelector('.xt-overview-sub')!.textContent}`,
});
/** The tile's definition control. */
const tileInfo = () => within(mergedTile()).getByRole('button', { name: 'Merged PRs definition' });
const effort = () => screen.getByTestId('effort-by-type');
/** The range's total above the chart, as it reads. */
const total = () => within(effort()).getByTestId('effort-total').textContent;
/** A day's bar, or a day's merged pull requests, by its accessible name. */
const day = (label: string | RegExp) => within(effort()).getByRole('img', { name: label });
/** The day columns with a drawn bar, by date. */
const barred = () =>
  [...effort().querySelectorAll('[data-testid="effort-day"]')]
    .filter((node) => node.querySelector('.xt-effort-bar'))
    .map((node) => node.getAttribute('data-date'));
const measure = (name: 'agent h' | 'human h' | 'cost') => screen.getByRole('radio', { name });
/** Opens the dialog behind Effort's ⓘ: the definition, the notes and the daily values. */
async function method() {
  fireEvent.click(screen.getByRole('button', { name: 'Effort definition' }));
  return screen.findByRole('dialog', { name: 'How effort is counted · daily values' });
}

it('F19: one shared session counts once, on its event day, beside two merge markers', async () => {
  mount(nativeSource(sectioned(F19_SHARED)));
  await loaded();
  expect(tile().textContent).toContain('2');
  expect(tile().textContent).not.toContain('unknown');
  // No legend of assignments: one total and one bar per day.
  expect(effort().querySelector('.xt-effort-legend')).toBeNull();
  expect(total()).toBe('0 h 30 m agent hrs');
  expect(within(effort()).getByTestId('effort-headline').textContent).toBe('0 h 30 m agent hrs');
  // Merge markers are deduplicated per pull request, on their own merge days;
  // each is that day's count, with every pull request's facts in its name.
  expect(day(/^2026-09-05: 1 merged pull request · xtrace\/app#1 · feat/).textContent).toBe('1');
  expect(day(/^2026-09-06: 1 merged pull request · xtrace\/app#2 · fix/).textContent).toBe('1');
  expect(screen.getByTestId('effort-marker-count').textContent).toBe('2 PRs');
  expect(barred()).toEqual(['2026-09-03']);
  expect(day('2026-09-03: 0 h 30 m. synthetic-model 0 h 30 m (100%)')).toBeTruthy();
  fireEvent.click(measure('cost'));
  // The card shows no line of session totals above the chart.
  expect(screen.queryByTestId('effort-cohort')).toBeNull();
  expect(effort().textContent).not.toContain('in range');
  expect(total()).toBe('$2.50');
  // Dollars sit on the day the response was recorded, not on a merge day.
  expect(day('2026-09-03: $2.50. synthetic-model $2.50 (100%)')).toBeTruthy();
  // A merge day's name ends with its merged numbers; the effort on it stays what it was.
  expect(day('2026-09-05: no usage. merged #1')).toBeTruthy();
  expect(barred()).toEqual(['2026-09-03']);
  // Each day is a tab stop.
  expect(
    within(effort())
      .getAllByTestId('effort-day')
      .map((node) => node.tabIndex),
  ).toEqual([0, 0, 0, 0, 0, 0, 0]);
});

it('confirmed only: an inferred link is gone before the tile and the markers', async () => {
  mount(nativeSource(sectioned(F19_CONFIRMED)));
  await loaded();
  expect(tile().textContent).toContain('1');
  expect(screen.getByTestId('effort-marker-count').textContent).toBe('1 PR');
  // The static method is one dialog away from the card's header, together with
  // the daily values; nothing of it takes the card's height.
  expect(screen.queryByTestId('effort-notes')).toBeNull();
  const dialog = await method();
  expect(within(dialog).getByTestId('effort-notes').textContent).toContain('confirmed links');
  expect(within(dialog).getByTestId('effort-notes').textContent).toContain(
    'The whole-day columns run from midnight to midnight, as human time and leverage do',
  );
  // The daily table scrolls inside a named region that takes keyboard focus,
  // one row per day with both measures and the merged numbers, and no model.
  const region = within(dialog).getByRole('region', {
    name: 'Effort per day scroll area',
  });
  expect(region.tabIndex).toBe(0);
  const table = within(region).getByRole('table');
  expect(
    within(table)
      .getAllByRole('columnheader')
      .map((cell) => cell.textContent),
  ).toEqual([
    'Day',
    'Agent time',
    'Cost',
    'Merged',
    'Whole day: agent time',
    'Whole day: human time',
  ]);
  expect(
    within(table)
      .getAllByRole('row')
      .slice(1)
      .map((row) => row.textContent),
  ).toEqual([
    // The range's bars (synthetic here), then the whole-day pair leverage
    // divides: F1's own agent hours and its five messages on Sep 7.
    // Each day is named as the chart names it, and both whole-day columns
    // are written as all agent time is.
    'Sep 10 h 0 mno usagenone0 h 0 m0 h 0 m',
    'Sep 20 h 0 mno usagenone0 h 0 m0 h 0 m',
    'Sep 30 h 30 m$2.50none0 h 0 m0 h 0 m',
    'Sep 40 h 0 mno usagenone0 h 0 m0 h 0 m',
    'Sep 50 h 0 mno usage#10 h 0 m0 h 0 m',
    'Sep 60 h 0 mno usagenone0 h 0 m0 h 0 m',
    'Sep 70 h 0 mno usagenone0 h 23 m0 h 20 m',
  ]);
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // No assignment is drawn, so the header has no unresolved-type triangle.
  expect(screen.queryByTestId('effort-unresolved')).toBeNull();
});

it('unknown facts: the known merged count is shown with how many are not checked yet', async () => {
  mount(nativeSource(sectioned(UNKNOWN_FACTS)));
  await loaded();
  expect(tile().textContent).toBe('Merged PRs1so far1 not checked yet');
  // An unresolved session's effort is in the day totals like any other's; the
  // card draws no type, so no unresolved-type triangle is in its header.
  expect(total()).toBe('1 h 0 m agent hrs');
  expect(screen.queryByTestId('effort-unresolved')).toBeNull();
  expect(await statusLine()).toContain(
    'Confirmed-linked pull requests: 1 checked, 1 not checked yet',
  );
});

it('zero: no link at all is a measured 0 and the work still shows', async () => {
  mount(nativeSource(sectioned(NO_LINKS)));
  await loaded();
  expect(within(mergedTile()).getByText('0')).toBeTruthy();
  expect(tile().textContent).not.toMatch(/known|unknown|Unmeasured/);
  expect(barred()).toEqual(['2026-09-02']);
  expect(within(effort()).getAllByText('0 PRs').length).toBe(1);
  expect(effort().querySelectorAll('.xt-effort-marker')).toHaveLength(0);
});

it('partial pricing is explicit: a priced subtotal with a +, the unpriced responses named, never a $0', async () => {
  mount(nativeSource(sectioned(PARTIAL_COST)));
  await loaded();
  fireEvent.click(measure('cost'));
  expect(total()).toBe('$1.25+');
  expect(within(effort()).getByTestId('effort-headline').textContent).toBe('$1.25+');
  const notes = within(await method()).getByTestId('effort-notes').textContent;
  expect(notes).toContain(
    '1 of 2 selected responses could not be priced (unpublished-model: model not in the price catalog, 1 response); they add nothing to the bars',
  );
  expect(notes).toContain(
    '1 Codex response recorded no service tier and is priced at OpenAI’s default (standard) tier',
  );
  expect(within(screen.getByRole('dialog')).getByText('$1.25+')).toBeTruthy();
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // The partial state reads in view without opening anything, and never as a total.
  const partial = day(
    '2026-09-03: $1.25+. synthetic-model $1.25 (100%). No price: 1 unpublished-model response',
  );
  expect(within(partial).getByTestId('effort-partial').textContent).toBe('+');
  expect(day('2026-09-01: no usage')).toBeTruthy();
  expect(screen.queryByTestId('effort-unmeasured')).toBeNull();
  expect(barred()).toEqual(['2026-09-03']);
  // Agent time is measured for the same cohort whatever the pricing says.
  fireEvent.click(measure('agent h'));
  expect(total()).toBe('0 h 30 m agent hrs');
  expect(effort().querySelector('[data-testid="effort-partial"]')).toBeNull();
});

it('nothing priced: the day is unknown, not zero, and the plot says nothing is priced', async () => {
  mount(
    nativeSource(
      sectioned({
        tile: { known_merged: 0 },
        groups: [
          {
            assignment: { kind: 'other' },
            sessions: 1,
            days: { 2: { agentMs: 1_800_000, selected: 3, priced: 0 } },
          },
        ],
      }),
    ),
  );
  await loaded();
  expect(screen.getByRole('heading', { level: 2, name: 'Effort' })).toBeTruthy();
  fireEvent.click(measure('cost'));
  expect(total()).toBe('cost unknown');
  expect(day('2026-09-03: cost unknown. No price: 3 unpublished-model responses')).toBeTruthy();
  expect(screen.getByTestId('effort-unmeasured').textContent).toBe(
    'No priced daily usage to plot.',
  );
  expect(barred()).toEqual([]);
});

it('hovering or focusing a day opens its card with every model and its share', async () => {
  mount(
    nativeSource(
      sectioned({
        tile: { known_merged: 0 },
        groups: [
          {
            assignment: { kind: 'type', work_type: 'feat' },
            sessions: 2,
            days: { 4: { agentMs: 20 * 3_600_000, usd: 300, model: 'gpt-6-astra' } },
          },
          {
            assignment: { kind: 'other' },
            sessions: 1,
            days: { 4: { agentMs: 6 * 3_600_000, usd: 120, model: 'claude-opus-5-5' } },
          },
        ],
      }),
    ),
  );
  await loaded();
  expect(within(effort()).getByTestId('effort-reference').textContent).toBe('24 h');
  expect(within(effort()).getByTestId('effort-headline').textContent).toBe('26 h 0 m agent hrs');
  const busy = day(
    '2026-09-05: 26 h 0 m. gpt-6-astra 20 h 0 m (77%), claude-opus-5-5 6 h 0 m (23%). Above 24 h: agents ran at the same time',
  );
  fireEvent.focus(busy);
  fireEvent.pointerEnter(busy);
  fireEvent.mouseEnter(busy);
  const card = await screen.findByTestId('effort-day-card');
  expect(
    within(card)
      .getAllByTestId('effort-day-model')
      .map((row) => row.textContent),
  ).toEqual(['gpt-6-astra20 h 0 m77%', 'claude-opus-5-56 h 0 m23%']);
  expect(card.textContent).toContain('Sep 5');
  expect(card.textContent).toContain('Above 24 h: agents ran at the same time');
});

it('switches the measure from the keyboard and keeps it across a range change', async () => {
  const source = nativeSource(sectioned(F19_SHARED));
  mount(source);
  await loaded();
  const agent = measure('agent h');
  expect(agent.getAttribute('aria-checked')).toBe('true');
  agent.focus();
  fireEvent.keyDown(agent, { key: 'ArrowRight' });
  await waitFor(() => expect(measure('human h').getAttribute('aria-checked')).toBe('true'));
  expect(document.activeElement).toBe(measure('human h'));
  expect(screen.getByTestId('human-timeline')).toBeTruthy();
  fireEvent.keyDown(measure('human h'), { key: 'ArrowRight' });
  await waitFor(() => expect(measure('cost').getAttribute('aria-checked')).toBe('true'));
  expect(document.activeElement).toBe(measure('cost'));
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(source.dashboard).toHaveBeenCalledWith(14));
  await loaded();
  expect(measure('cost').getAttribute('aria-checked')).toBe('true');
});

// --- manual refresh ----------------------------------------------------------

const ATTENTION = 'Pull-request checks need your attention';
const RUNNING = 'Refreshing pull-request facts…';
/** The Effort card's red !: it opens the refresh dialog, and is drawn only when needed. */
const refreshButton = () => screen.findByRole('button', { name: ATTENTION });
/** The red ! or, while a batch runs, its quiet stand-in. */
const attentionMark = () => screen.queryByTestId('pr-attention');
const dialog = () => screen.getByRole('dialog', { name: 'Refresh pull-request facts' });
/** The confirmed links' status, which the refresh dialog states first. */
async function statusLine() {
  fireEvent.click(await refreshButton());
  const text = within(await screen.findByRole('dialog')).getByTestId('pr-refresh').textContent!;
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  return text;
}
/**
 * The same native-shaped source whose automatic check paused because gh is
 * missing, so the red ! is there whatever the report's counts.
 */
function ghMissing<S extends ReturnType<typeof nativeSource>>(source: S) {
  const status = autoStatus({ paused: 'gh_missing' });
  return {
    ...source,
    prAutoCheck: { status: vi.fn(async () => status), request: vi.fn(async () => status) },
  };
}
const option = (text: RegExp) => within(dialog()).getByRole('checkbox', { name: text });
const start = () => within(dialog()).getByRole('button', { name: /^Refresh \d+ pull request/ });

it('asks nothing of storage or GitHub until opened, and refreshes only on the button', async () => {
  const source = new FixtureDataSource(exported);
  const list = vi.spyOn(source, 'pullRequests');
  const refresh = vi.spyOn(source, 'refreshPullRequests');
  mount(source);
  await loaded();
  expect(list).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();
  fireEvent.click(await refreshButton());
  await within(dialog()).findByRole('list', { name: 'Indexed pull requests' });
  expect(list).toHaveBeenCalledOnce();
  const text = dialog().textContent!;
  expect(text).toContain('It only reads: nothing is written to GitHub');
  expect(text).toContain('XTrace also checks on its own, the same way');
  expect(text).toContain('Merged and closed ones are not checked again');
  expect(text).toContain('up to 20 pull requests');
  expect(text).toContain('120 seconds');
  for (const number of [11, 12, 13]) expect(option(new RegExp(`#${number}`))).toBeTruthy();
  expect(start()).toHaveProperty('disabled', true);
  expect(refresh).not.toHaveBeenCalled();
});

it('refreshes a selection, updates the Dashboard after a commit, and replays a repeat as unchanged', async () => {
  const source = new FixtureDataSource(exported);
  const dashboard = vi.spyOn(source, 'dashboard');
  mount(source);
  await loaded();
  fireEvent.click(await refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#11/ }));
  expect(within(dialog()).getByText(/1 of 20 selected/)).toBeTruthy();
  const before = dashboard.mock.calls.length;
  fireEvent.click(start());
  const report = await within(dialog()).findByTestId('pr-refresh-report');
  expect(report.textContent).toBe(
    'Requested 1: 1 checked, 0 could not be checked, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
  await waitFor(() => expect(dashboard.mock.calls.length).toBeGreaterThan(before));
  expect(within(dialog()).getByText(/Checked · saved/)).toBeTruthy();
  // Repeat: the fixture stamps every attempt with its pinned instant, so the
  // same result is unchanged, nothing commits and the Dashboard is not re-read.
  // The selection is kept after a batch, so the same pull request goes again.
  const settled = dashboard.mock.calls.length;
  expect((option(/#11/) as HTMLInputElement).checked).toBe(true);
  fireEvent.click(start());
  await waitFor(() =>
    expect(within(dialog()).getByTestId('pr-refresh-report').textContent).toBe(
      'Requested 1: 1 checked, 0 could not be checked, 0 skipped. No stored facts changed.',
    ),
  );
  expect(within(dialog()).getByText(/Checked · unchanged/)).toBeTruthy();
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(dashboard.mock.calls.length).toBe(settled);
});

it('a partial failure keeps earlier facts, and the whole refresh shows what the export read after it', async () => {
  const source = new FixtureDataSource(exported);
  mount(source);
  await loaded();
  expect(tile().textContent).toContain('2 PRs not checked yet');
  fireEvent.click(await refreshButton());
  for (const number of [11, 12, 13])
    fireEvent.click(
      await within(dialog()).findByRole('checkbox', { name: new RegExp(`#${number}`) }),
    );
  fireEvent.click(start());
  expect((await within(dialog()).findByTestId('pr-refresh-report')).textContent).toBe(
    'Requested 3: 2 checked, 1 could not be checked, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
  expect(
    within(dialog()).getByText(/Could not be checked: rate limited; earlier facts are kept/),
  ).toBeTruthy();
  await waitFor(() =>
    expect(within(dialog()).getByText(/could not be checked \(rate limited\)/)).toBeTruthy(),
  );
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  // #12's check failed, so it is not "not checked": it could not be checked.
  // #11 was checked, so the known zero shows, marked incomplete.
  await waitFor(() => expect(tile().textContent).toContain('0so far1 could not be checked'));
  // #12 hit GitHub's rate limit: that is about the whole run and temporary,
  // so the manual refresh does not mark it as tried, and the red ! still asks.
  await waitFor(() => expect(attentionMark()!.dataset.state).toBe('attention'));
  const tip = await mergedTip();
  expect(tip).toContain(
    '1 PR could not be checked. The last try failed. Click the red ! on the Effort card to check it again.',
  );
  expect(await statusLine()).toContain(
    'Confirmed-linked pull requests: 1 checked, 1 could not be checked',
  );
});

const rows = (count: number): PrRow[] =>
  Array.from({ length: count }, (_, index) => ({
    pull_request: {
      id: index + 1,
      repository: 'xtrace/app',
      number: index + 100,
      url: `https://github.com/xtrace/app/pull/${index + 100}`,
    },
    linked_sessions: 1,
    title: null,
    state: null,
    merged_at: null,
    additions: null,
    deletions: null,
    head_ref_name: null,
    refreshed_at_ms: null,
    last_attempted_at_ms: null,
    status: { status: 'never_attempted' },
  }));
const cancelledReport = (ids: number[]): PrRefreshReport => ({
  requested: ids.length,
  attempted: 0,
  succeeded: 0,
  failed: 0,
  skipped: ids.length,
  unrecorded: 0,
  cancelled: true,
  committed: false,
  rows: ids.map((id) => ({
    id,
    pull_request: rows(25)[id - 1]!.pull_request,
    outcome: { outcome: 'skipped', reason: 'cancelled' },
  })),
});

it('lists a not-found number for a check by hand and words its result apart from a confirmed one', async () => {
  const facts = Date.UTC(2026, 8, 6);
  const [missing, confirmed] = rows(2);
  const listed: PrRow[] = [
    { ...missing!, last_attempted_at_ms: facts, status: { status: 'not_found_on_github' } },
    {
      ...confirmed!,
      title: 'feat: confirmed once',
      state: 'open',
      refreshed_at_ms: facts,
      last_attempted_at_ms: facts,
      status: { status: 'refreshed' },
    },
  ];
  const notFound = {
    outcome: 'failed',
    error: 'not_found',
    persistence: { persistence: 'recorded', write: 'applied' },
  } as const;
  const source = ghMissing(
    nativeSource(sectioned(NO_LINKS), {
      pullRequests: async () => ({ rows: listed }),
      refreshPullRequests: async (ids) => ({
        requested: ids.length,
        attempted: ids.length,
        succeeded: 0,
        failed: ids.length,
        skipped: 0,
        unrecorded: 0,
        cancelled: false,
        committed: true,
        rows: ids.map((id) => ({
          id,
          pull_request: listed[id - 1]!.pull_request,
          outcome: notFound,
        })),
      }),
    }),
  );
  mount(source);
  await loaded();
  fireEvent.click(await refreshButton());
  const boxes = await within(dialog()).findAllByRole('checkbox');
  expect(boxes).toHaveLength(2);
  expect(
    within(dialog()).getByText(/not found on GitHub \(checked .*\); not counted as a pull request/),
  ).toBeTruthy();
  // Only never-confirmed numbers are said to be left out.
  expect(dialog().textContent).toContain('never confirmed before, is marked not found on GitHub');
  expect(dialog().textContent).toContain('A pull request GitHub confirmed before keeps its facts');
  for (const box of boxes) fireEvent.click(box);
  fireEvent.click(start());
  expect((await within(dialog()).findByTestId('pr-refresh-report')).textContent).toBe(
    'Requested 2: 0 checked, 0 could not be checked, 2 not found on GitHub, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
  expect(
    within(dialog()).getByText(/Not found on GitHub; not counted as a pull request · saved/),
  ).toBeTruthy();
  expect(
    within(dialog()).getByText(/Not found on GitHub; earlier facts kept · saved/),
  ).toBeTruthy();
});

it('selects at most twenty pull requests and sends exactly those', async () => {
  const source = ghMissing(
    nativeSource(sectioned(NO_LINKS), {
      pullRequests: async () => ({ rows: rows(25) }),
      refreshPullRequests: async (ids) => cancelledReport(ids),
    }),
  );
  mount(source);
  await loaded();
  fireEvent.click(await refreshButton());
  const boxes = await within(dialog()).findAllByRole('checkbox');
  expect(boxes).toHaveLength(25);
  for (const box of boxes.slice(0, 20)) fireEvent.click(box);
  expect(within(dialog()).getByText(/20 of 20 selected/)).toBeTruthy();
  expect(boxes.slice(20).every((box) => (box as HTMLInputElement).disabled)).toBe(true);
  expect(within(dialog()).getByText(/20 is the most one batch refreshes/)).toBeTruthy();
  fireEvent.click(start());
  await within(dialog()).findByTestId('pr-refresh-report');
  expect(source.refreshPullRequests).toHaveBeenCalledExactlyOnceWith(
    Array.from({ length: 20 }, (_, index) => index + 1),
  );
});

it('cancels a running batch and reports what it skipped', async () => {
  let finish: (report: PrRefreshReport) => void = () => {};
  const source = ghMissing(
    nativeSource(sectioned(NO_LINKS), {
      pullRequests: async () => ({ rows: rows(2) }),
      refreshPullRequests: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
      cancelPullRequestRefresh: async () => true,
    }),
  );
  mount(source);
  await loaded();
  const reads = source.dashboard.mock.calls.length;
  fireEvent.click(await refreshButton());
  for (const box of await within(dialog()).findAllByRole('checkbox')) fireEvent.click(box);
  fireEvent.click(start());
  const cancel = await within(dialog()).findByRole('button', { name: 'Cancel' });
  // While the batch runs, the red ! is a quiet "refreshing" mark instead.
  expect(attentionMark()!.getAttribute('aria-label')).toBe(RUNNING);
  expect(attentionMark()!.dataset.state).toBe('running');
  expect(
    within(dialog())
      .getAllByRole('checkbox')
      .every((box) => box.matches(':disabled')),
  ).toBe(true);
  fireEvent.click(cancel);
  expect(source.cancelPullRequestRefresh).toHaveBeenCalledOnce();
  expect(within(dialog()).getByRole('button', { name: 'Cancelling…' })).toHaveProperty(
    'disabled',
    true,
  );
  await act(async () => finish(cancelledReport([1, 2])));
  expect((await within(dialog()).findByTestId('pr-refresh-report')).textContent).toBe(
    'Requested 2: 0 checked, 0 could not be checked, 2 skipped. The batch was cancelled. No stored facts changed.',
  );
  expect(within(dialog()).getAllByText(/Skipped: cancelled before it ran/)).toHaveLength(2);
  expect(source.dashboard).toHaveBeenCalledTimes(reads);
});

it('a refused batch is stated and changes nothing already shown', async () => {
  const source = nativeSource(sectioned(UNKNOWN_FACTS), {
    pullRequests: async () => ({ rows: rows(1) }),
    refreshPullRequests: async () => {
      throw new Error('The GitHub CLI was not found. Install gh, then try again.');
    },
  });
  mount(source);
  await loaded();
  const reads = source.dashboard.mock.calls.length;
  fireEvent.click(await refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#100/ }));
  fireEvent.click(start());
  expect((await within(dialog()).findByTestId('pr-refresh-error')).textContent).toBe(
    'The refresh did not run: The GitHub CLI was not found. Install gh, then try again.',
  );
  expect(source.dashboard).toHaveBeenCalledTimes(reads);
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(tile().textContent).toContain('1 not checked yet');
});

it('a stale row keeps its cached facts and says the last refresh failed', async () => {
  const [row] = rows(1);
  const stale: PrRow = {
    ...row!,
    title: 'feat: kept',
    state: 'merged',
    merged_at: '2026-09-05T12:00:00Z',
    refreshed_at_ms: Date.parse('2026-09-06T00:00:00Z'),
    last_attempted_at_ms: Date.parse('2026-09-07T00:00:00Z'),
    status: { status: 'failed_after_refresh', error: 'rate_limited' },
  };
  mount(
    ghMissing(
      nativeSource(sectioned(MANY_TYPES), { pullRequests: async () => ({ rows: [stale] }) }),
    ),
  );
  await loaded();
  fireEvent.click(await refreshButton());
  const item = (
    await within(dialog()).findByRole('checkbox', { name: /#100 · feat: kept/ })
  ).closest('li')!;
  expect(item.textContent).toContain('merged Sep 5, 12:00 PM');
  expect(item.textContent).toContain(
    'stale after a failed check (rate limited); facts from Sep 6, 12:00 AM',
  );
});

it('opens from the keyboard and returns focus to its button on Escape', async () => {
  mount(
    ghMissing(nativeSource(sectioned(NO_LINKS), { pullRequests: async () => ({ rows: rows(1) }) })),
  );
  await loaded();
  const button = await refreshButton();
  button.focus();
  fireEvent.click(button);
  const box = await within(dialog()).findByRole('checkbox', { name: /#100/ });
  box.focus();
  fireEvent.click(box);
  expect((box as HTMLInputElement).checked).toBe(true);
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(button));
});

it('refreshing only the exact link #11 shows the state Rust read for exactly that', async () => {
  const source = new FixtureDataSource(exported);
  mount(source);
  await loaded();
  fireEvent.click(await refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#11/ }));
  fireEvent.click(start());
  await within(dialog()).findByTestId('pr-refresh-report');
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  // #11 is refreshed; the SHA link #12 has no facts yet, so the count stays
  // unknown; the inferred #13 is not counted by the confirmed-only Dashboard.
  // #11 was checked, so the known zero shows, marked incomplete.
  await waitFor(() => expect(tile().textContent).toContain('0so far1 not checked yet'));
  expect(await statusLine()).toContain(
    'Confirmed-linked pull requests: 1 checked, 1 not checked yet',
  );
});

it('keeps a running batch, its Cancel and its result across a range change and a route return', async () => {
  let finish: (report: PrRefreshReport) => void = () => {};
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const source = ghMissing(
    nativeSource(sectioned(NO_LINKS), {
      pullRequests: async () => ({ rows: rows(2) }),
      refreshPullRequests: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
      cancelPullRequestRefresh: async () => true,
    }),
  );
  source.dashboard.mockImplementation(async (days: number) => {
    if (days === 14) await gate;
    return sectioned(NO_LINKS)(days);
  });
  mount(source);
  await loaded();
  fireEvent.click(await refreshButton());
  for (const box of await within(dialog()).findAllByRole('checkbox')) fireEvent.click(box);
  fireEvent.click(start());
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(screen.getByRole('button', { name: RUNNING })).toBeTruthy();
  // An uncached range replaces the whole report, this control included.
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await screen.findByText(/Reading Dashboard metrics for the last 14d/);
  expect(attentionMark()).toBeNull();
  await act(async () => release());
  await loaded();
  // The batch is still running, still cancellable, and cannot be doubled.
  fireEvent.click(screen.getByRole('button', { name: RUNNING }));
  expect(within(dialog()).getByRole('button', { name: 'Refreshing…' })).toHaveProperty(
    'disabled',
    true,
  );
  fireEvent.click(within(dialog()).getByRole('button', { name: 'Cancel' }));
  expect(source.cancelPullRequestRefresh).toHaveBeenCalledOnce();
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // Leave the Dashboard while the batch finishes, then come back to its result.
  act(() => go('/sessions'));
  await waitFor(() => expect(attentionMark()).toBeNull());
  await act(async () => finish(cancelledReport([1, 2])));
  act(() => go('/dashboard'));
  await loaded();
  expect(screen.queryByRole('button', { name: RUNNING })).toBeNull();
  fireEvent.click(await refreshButton());
  expect((await within(dialog()).findByTestId('pr-refresh-report')).textContent).toBe(
    'Requested 2: 0 checked, 0 could not be checked, 2 skipped. The batch was cancelled. No stored facts changed.',
  );
  expect(source.refreshPullRequests).toHaveBeenCalledOnce();
});

it('a replaced source starts with no batch and ignores the old one', async () => {
  let finish: (report: PrRefreshReport) => void = () => {};
  const first = ghMissing(
    nativeSource(sectioned(NO_LINKS), {
      pullRequests: async () => ({ rows: rows(1) }),
      refreshPullRequests: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    }),
  );
  const second = ghMissing(
    nativeSource(sectioned(NO_LINKS), {
      pullRequests: async () => ({ rows: rows(1) }),
    }),
  );
  const view = mount(first);
  await loaded();
  fireEvent.click(await refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#100/ }));
  fireEvent.click(start());
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  view.rerender(tree(second));
  await loaded();
  expect(screen.queryByRole('button', { name: RUNNING })).toBeNull();
  const reads = second.dashboard.mock.calls.length;
  await act(async () => finish({ ...cancelledReport([1]), committed: true }));
  expect(screen.queryByRole('button', { name: RUNNING })).toBeNull();
  fireEvent.click(await refreshButton());
  await within(dialog()).findByRole('checkbox', { name: /#100/ });
  expect(within(dialog()).queryByTestId('pr-refresh-report')).toBeNull();
  expect(second.dashboard).toHaveBeenCalledTimes(reads);
  expect(second.refreshPullRequests).not.toHaveBeenCalled();
});

// --- automatic check ---------------------------------------------------------

function autoStatus(status: Partial<PrAutoCheckStatus> = {}): PrAutoCheckStatus {
  return { enabled: true, checking: false, paused: null, last_finished_at_ms: null, ...status };
}
/** F1's report from a native-shaped source that also checks GitHub on its own. */
function autoChecked(status: PrAutoCheckStatus) {
  const prAutoCheck = {
    status: vi.fn(async () => status),
    request: vi.fn(async () => status),
  };
  return { ...nativeSource(() => f1()), prAutoCheck };
}
async function mergedTip() {
  fireEvent.focus(tileInfo());
  return (await screen.findByRole('tooltip')).textContent!;
}

it('asks the app to check GitHub when the Dashboard is shown, and says so while it checks', async () => {
  const source = autoChecked(autoStatus({ checking: true }));
  mount(source);
  await loaded();
  expect(source.prAutoCheck.request).toHaveBeenCalledOnce();
  await waitFor(() => expect(tile().textContent).toBe('Merged PRs2 PRs not checked yetchecking…'));
  expect(await mergedTip()).toContain('Checking GitHub now… 2 PRs are not checked yet.');
  // The automatic check is the app's own; no manual batch starts.
  expect(source.refreshPullRequests).not.toHaveBeenCalled();
  expect(source.pullRequests).not.toHaveBeenCalled();
});

it('the Merged PRs tip says in plain words what it counts and how fresh it is', async () => {
  mount(autoChecked(autoStatus()));
  await loaded();
  await waitFor(() => expect(tile().textContent).toBe('Merged PRs2 PRs not checked yet'));
  const text = await mergedTip();
  expect(text).toContain(
    'Pull requests your agent sessions opened or pushed to, and how many of them merged on GitHub in this range. XTrace checks GitHub using your gh sign-in; it only reads.',
  );
  expect(text).toContain('2 PRs are not checked yet.');
  // None of the rule's technical wording is in this tile's tip.
  expect(text).not.toMatch(/unions|confirmed_only|whole-linked-session|cached|gh pr view/);
  // The Effort card keeps the rule's own summary in its definition.
  fireEvent.blur(tileInfo());
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  act(() => screen.getByRole('button', { name: 'Effort definition' }).focus());
  expect((await screen.findByRole('tooltip')).textContent).toContain(ruleSummary('M-19'));
});

it('a paused check says why in plain words, without an error', async () => {
  mount(autoChecked(autoStatus({ paused: 'gh_signed_out' })));
  await loaded();
  await waitFor(() =>
    expect(tile().textContent).toBe('Merged PRs2 PRs not checked yetgh not signed in'),
  );
  expect(await mergedTip()).toContain(
    'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. 2 PRs are not checked yet. Run gh auth login, then click the red ! on the Effort card to check again.',
  );
  expect(screen.queryByRole('alert')).toBeNull();
});

it('a complete count whose change is hidden still says why in its tip', async () => {
  mount(nativeSource(sectioned(F19_SHARED)));
  await loaded();
  expect(await mergedTip()).toContain(DELTA_HIDDEN);
});

// --- the red ! ---------------------------------------------------------------

/** Focuses the red ! and reads its tip. */
async function attentionTip() {
  const button = await refreshButton();
  fireEvent.focus(button);
  return (await screen.findByRole('tooltip')).textContent!;
}

it('shows no red ! and no status line when nothing needs the user', async () => {
  // Two pull requests not checked yet while automatic checks run: XTrace checks them itself.
  const source = autoChecked(autoStatus());
  mount(source);
  await loaded();
  await waitFor(() => expect(source.prAutoCheck.status).toHaveBeenCalled());
  await waitFor(() => expect(tile().textContent).toBe('Merged PRs2 PRs not checked yet'));
  expect(attentionMark()).toBeNull();
  expect(screen.queryByRole('button', { name: 'Refresh PR facts…' })).toBeNull();
  // The old line under the chart is gone; its words live in the dialog now.
  expect(document.querySelector('.xt-dashboard')!.textContent).not.toContain(
    'Confirmed-linked pull requests',
  );
  // Nor does the tile's tip point to a red ! that is not there.
  expect(await mergedTip()).not.toContain('red !');
  // The ⓘ beside the title opens the per-day table; there is no separate button.
  for (const name of ['Method', 'Details'])
    expect(screen.queryByRole('button', { name })).toBeNull();
  const opened = await method();
  // The ⓘ's short definition opens the dialog, before the notes and the table.
  expect(within(opened).getByTestId('effort-definition').textContent).toBe(
    `${ruleSummary('M-19')} ${EFFORT_CONTEXT}`,
  );
});

it('shows the red ! with a plain tip when a check failed, and opens the refresh dialog', async () => {
  const failed: SectionSpec = {
    ...NO_LINKS,
    tile: {
      known_merged: 0,
      freshness: {
        never_attempted: 0,
        refreshed: 1,
        failed_never_refreshed: 1,
        failed_after_refresh: 1,
        manual_failed_never_refreshed: 0,
        manual_failed_after_refresh: 0,
        oldest_refreshed_at: null,
        newest_attempted_at: null,
      },
    },
  };
  const status = autoStatus();
  const source = {
    ...nativeSource(sectioned(failed), { pullRequests: async () => ({ rows: rows(1) }) }),
    prAutoCheck: { status: vi.fn(async () => status), request: vi.fn(async () => status) },
  };
  mount(source);
  await loaded();
  const button = await refreshButton();
  expect(button.dataset.state).toBe('attention');
  // It sits in the Effort card's header, with the ⓘ.
  const header = button.closest('.xt-dash-card-actions, header')!;
  expect(header.contains(screen.getByRole('button', { name: 'Effort definition' }))).toBe(true);
  // One never had facts, one kept older ones: each is named in the tile's words.
  expect(await attentionTip()).toBe(
    '1 pull request could not be checked. The last try failed. 1 pull request is stale after a failed check. Older facts are shown. Click to check them again.',
  );
  fireEvent.click(button);
  const opened = await screen.findByRole('dialog', { name: 'Refresh pull-request facts' });
  // The status line that used to sit under the chart opens the dialog.
  expect(within(opened).getByTestId('pr-refresh').textContent).toBe(
    'Confirmed-linked pull requests: 1 checked, 1 stale after a failed check, 1 could not be checked. 1 pull request could not be checked. The last try failed. 1 pull request is stale after a failed check. Older facts are shown. Choose them below and refresh them again.',
  );
  expect(source.pullRequests).toHaveBeenCalledOnce();
});

it('shows the red ! when gh paused the automatic check, and says what to do', async () => {
  mount(autoChecked(autoStatus({ paused: 'gh_signed_out' })));
  await loaded();
  expect(await attentionTip()).toBe(
    'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. Run gh auth login, then click to check again.',
  );
});

it('shows the red ! when automatic checks are off and some were never checked', async () => {
  mount(autoChecked(autoStatus({ enabled: false })));
  await loaded();
  expect(await attentionTip()).toBe(
    'Automatic checks are off and 2 pull requests are not checked yet. Click to check them.',
  );
  expect(await mergedTip()).toContain('Click the red ! on the Effort card to check them.');
});

it('returns focus to the ⓘ when a refresh cleared what the red ! was for', async () => {
  const failed: SectionSpec = {
    ...NO_LINKS,
    tile: {
      known_merged: 0,
      freshness: {
        never_attempted: 0,
        refreshed: 0,
        failed_never_refreshed: 0,
        failed_after_refresh: 1,
        manual_failed_never_refreshed: 0,
        manual_failed_after_refresh: 0,
        oldest_refreshed_at: null,
        newest_attempted_at: null,
      },
    },
  };
  let spec = failed;
  const status = autoStatus();
  const source = {
    ...nativeSource((days) => sectioned(spec)(days), {
      pullRequests: async () => ({ rows: rows(1) }),
      refreshPullRequests: async (ids) => {
        // The check succeeds: the next Dashboard read has nothing left to flag.
        spec = NO_LINKS;
        return {
          ...cancelledReport(ids),
          attempted: ids.length,
          succeeded: ids.length,
          skipped: 0,
          cancelled: false,
          committed: true,
          rows: [],
        };
      },
    }),
    prAutoCheck: { status: vi.fn(async () => status), request: vi.fn(async () => status) },
  };
  mount(source);
  await loaded();
  const button = await refreshButton();
  button.focus();
  fireEvent.click(button);
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#100/ }));
  fireEvent.click(start());
  await within(dialog()).findByTestId('pr-refresh-report');
  // The re-read report has no failure; the red ! stays only while its dialog is open.
  await waitFor(() => expect(attentionMark()!.dataset.state).toBe('open'));
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(attentionMark()).toBeNull();
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Effort definition' })),
  );
});

it('stops asking once the user’s own check fails for the pull request itself, and returns focus to the ⓘ', async () => {
  const freshness = (manual: number) => ({
    never_attempted: 0,
    refreshed: 0,
    failed_never_refreshed: 1,
    failed_after_refresh: 0,
    manual_failed_never_refreshed: manual,
    manual_failed_after_refresh: 0,
    oldest_refreshed_at: null,
    newest_attempted_at: null,
  });
  // Rust reads storage again after the batch: the same failure, now also
  // counted as tried by hand (`gh pr view` failed for this pull request).
  let spec: SectionSpec = { ...NO_LINKS, tile: { known_merged: 0, freshness: freshness(0) } };
  const status = autoStatus();
  const source = {
    ...nativeSource((days) => sectioned(spec)(days), {
      pullRequests: async () => ({ rows: rows(1) }),
      refreshPullRequests: async (ids): Promise<PrRefreshReport> => {
        spec = { ...NO_LINKS, tile: { known_merged: 0, freshness: freshness(1) } };
        return {
          requested: ids.length,
          attempted: ids.length,
          succeeded: 0,
          failed: ids.length,
          skipped: 0,
          unrecorded: 0,
          cancelled: false,
          committed: true,
          rows: ids.map((id) => ({
            id,
            pull_request: rows(1)[0]!.pull_request,
            outcome: {
              outcome: 'failed',
              error: 'execution_failed',
              persistence: { persistence: 'recorded', write: 'applied' },
            },
          })),
        };
      },
    }),
    prAutoCheck: { status: vi.fn(async () => status), request: vi.fn(async () => status) },
  };
  mount(source);
  await loaded();
  const button = await refreshButton();
  button.focus();
  fireEvent.click(button);
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#100/ }));
  fireEvent.click(start());
  await within(dialog()).findByTestId('pr-refresh-report');
  // The re-read report counts it as tried by hand: nothing is left for the red !.
  await waitFor(() => expect(attentionMark()!.dataset.state).toBe('open'));
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(attentionMark()).toBeNull();
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Effort definition' })),
  );
  // The tile still says it failed, and that the user's own check failed too,
  // with no pointer to a red ! that is not there.
  const tip = await mergedTip();
  expect(tip).toContain(
    '1 PR could not be checked. The last try failed. Your own check of it failed too.',
  );
  expect(tip).not.toContain('red !');
});
