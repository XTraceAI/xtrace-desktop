import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { PrList } from '../../data/generated/PrList';
import type { PrRefreshReport } from '../../data/generated/PrRefreshReport';
import type { PrRow } from '../../data/generated/PrRow';
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
const tile = () =>
  screen.getAllByRole('button').find((button) => button.textContent?.startsWith('Merged PRs'))!;
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
const measure = (name: 'agent h' | 'cost') => screen.getByRole('radio', { name });
/** Opens the method dialog from the card's header: the notes and the daily values. */
async function method() {
  fireEvent.click(screen.getByRole('button', { name: 'Method' }));
  return screen.findByRole('dialog', { name: 'How effort is counted · daily values' });
}

it('F19: one shared session counts once, on its event day, beside two merge markers', async () => {
  mount(nativeSource(sectioned(F19_SHARED)));
  await loaded();
  expect(tile().textContent).toContain('2');
  expect(tile().textContent).not.toContain('unknown');
  // No legend of assignments: one total and one bar per day.
  expect(effort().querySelector('.xt-effort-legend')).toBeNull();
  expect(total()).toBe('0.5 agent h');
  expect(within(effort()).getByTestId('effort-headline').textContent).toBe(
    '0.5 agent hlast 7 days',
  );
  // Merge markers are deduplicated per pull request, on their own merge days;
  // each is that day's count, with every pull request's facts in its name.
  expect(day(/^2026-09-05: 1 merged pull request · xtrace\/app#1 · feat/).textContent).toBe('1');
  expect(day(/^2026-09-06: 1 merged pull request · xtrace\/app#2 · fix/).textContent).toBe('1');
  expect(screen.getByTestId('effort-marker-count').textContent).toBe('2 PRs');
  expect(barred()).toEqual(['2026-09-03']);
  expect(day('2026-09-03: 0.5 h. synthetic-model 0.5 h (100%)')).toBeTruthy();
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
  ).toEqual(['Day', 'Agent h', 'Cost', 'Merged']);
  expect(
    within(table)
      .getAllByRole('row')
      .slice(1)
      .map((row) => row.textContent),
  ).toEqual([
    '2026-09-010 hno usagenone',
    '2026-09-020 hno usagenone',
    '2026-09-030.5 h$2.50none',
    '2026-09-040 hno usagenone',
    '2026-09-050 hno usage#1',
    '2026-09-060 hno usagenone',
    '2026-09-070 hno usagenone',
  ]);
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // No assignment is drawn, so the header has no unresolved-type triangle.
  expect(screen.queryByTestId('effort-unresolved')).toBeNull();
});

it('unknown facts: the known subtotal is named and the count stays unknown, never zero', async () => {
  mount(nativeSource(sectioned(UNKNOWN_FACTS)));
  await loaded();
  const text = tile().textContent!;
  expect(text).toContain(
    'Unmeasured: 1 known merged; 1 linked pull request has no cached merge facts',
  );
  expect(text).toContain('1 + 1 unknown');
  // An unresolved session's effort is in the day totals like any other's; the
  // card draws no type, so no unresolved-type triangle is in its header.
  expect(total()).toBe('1 agent h');
  expect(screen.queryByTestId('effort-unresolved')).toBeNull();
  expect(screen.getByTestId('pr-refresh').textContent).toContain(
    'Confirmed-linked pull requests: 1 refreshed, 1 never refreshed',
  );
});

it('zero: no link at all is a measured 0 and the work still shows', async () => {
  mount(nativeSource(sectioned(NO_LINKS)));
  await loaded();
  expect(within(tile()).getByText('0')).toBeTruthy();
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
  expect(within(effort()).getByTestId('effort-headline').textContent).toBe(
    '$1.25+last 7 days1 response has no price',
  );
  const notes = within(await method()).getByTestId('effort-notes').textContent;
  expect(notes).toContain(
    '1 of 2 selected responses could not be priced (codex-auto-review: model not in the price catalog, 1 response); they add nothing to the bars',
  );
  expect(notes).toContain(
    '1 Codex response recorded no service tier and is priced at OpenAI’s default (standard) tier',
  );
  expect(within(screen.getByRole('dialog')).getByText('$1.25+')).toBeTruthy();
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // The partial state reads in view without opening anything, and never as a total.
  const partial = day(
    '2026-09-03: $1.25+. synthetic-model $1.25 (100%). No price: 1 codex-auto-review response',
  );
  expect(within(partial).getByTestId('effort-partial').textContent).toBe('+');
  expect(day('2026-09-01: no usage')).toBeTruthy();
  expect(screen.queryByTestId('effort-unmeasured')).toBeNull();
  expect(barred()).toEqual(['2026-09-03']);
  // Agent time is measured for the same cohort whatever the pricing says.
  fireEvent.click(measure('agent h'));
  expect(total()).toBe('0.5 agent h');
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
  expect(day('2026-09-03: cost unknown. No price: 3 codex-auto-review responses')).toBeTruthy();
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
  expect(within(effort()).getByTestId('effort-headline').textContent).toBe(
    '26 agent hlast 7 days1 day above 24 h',
  );
  const busy = day(
    '2026-09-05: 26 h. gpt-6-astra 20 h (77%), claude-opus-5-5 6 h (23%). Above 24 h: agents ran at the same time',
  );
  fireEvent.focus(busy);
  fireEvent.pointerEnter(busy);
  fireEvent.mouseEnter(busy);
  const card = await screen.findByTestId('effort-day-card');
  expect(
    within(card)
      .getAllByTestId('effort-day-model')
      .map((row) => row.textContent),
  ).toEqual(['gpt-6-astra20 h77%', 'claude-opus-5-56 h23%']);
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
  await waitFor(() => expect(measure('cost').getAttribute('aria-checked')).toBe('true'));
  expect(document.activeElement).toBe(measure('cost'));
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(source.dashboard).toHaveBeenCalledWith(14));
  await loaded();
  expect(measure('cost').getAttribute('aria-checked')).toBe('true');
});

// --- manual refresh ----------------------------------------------------------

const refreshButton = () => screen.getByRole('button', { name: 'Refresh PR facts…' });
const dialog = () => screen.getByRole('dialog', { name: 'Refresh pull-request facts' });
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
  fireEvent.click(refreshButton());
  await within(dialog()).findByRole('list', { name: 'Indexed pull requests' });
  expect(list).toHaveBeenCalledOnce();
  const text = dialog().textContent!;
  expect(text).toContain('It only reads: nothing is written to GitHub');
  expect(text).toContain('only the Refresh button starts a batch');
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
  fireEvent.click(refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#11/ }));
  expect(within(dialog()).getByText(/1 of 20 selected/)).toBeTruthy();
  const before = dashboard.mock.calls.length;
  fireEvent.click(start());
  const report = await within(dialog()).findByTestId('pr-refresh-report');
  expect(report.textContent).toBe(
    'Requested 1: 1 refreshed, 0 failed, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
  await waitFor(() => expect(dashboard.mock.calls.length).toBeGreaterThan(before));
  expect(within(dialog()).getByText(/Refreshed · saved/)).toBeTruthy();
  // Repeat: the fixture stamps every attempt with its pinned instant, so the
  // same result is unchanged, nothing commits and the Dashboard is not re-read.
  // The selection is kept after a batch, so the same pull request goes again.
  const settled = dashboard.mock.calls.length;
  expect((option(/#11/) as HTMLInputElement).checked).toBe(true);
  fireEvent.click(start());
  await waitFor(() =>
    expect(within(dialog()).getByTestId('pr-refresh-report').textContent).toBe(
      'Requested 1: 1 refreshed, 0 failed, 0 skipped. No stored facts changed.',
    ),
  );
  expect(within(dialog()).getByText(/Refreshed · unchanged/)).toBeTruthy();
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(dashboard.mock.calls.length).toBe(settled);
});

it('a partial failure keeps earlier facts, and the whole refresh shows what the export read after it', async () => {
  const source = new FixtureDataSource(exported);
  mount(source);
  await loaded();
  expect(tile().textContent).toContain('0 + 2 unknown');
  fireEvent.click(refreshButton());
  for (const number of [11, 12, 13])
    fireEvent.click(
      await within(dialog()).findByRole('checkbox', { name: new RegExp(`#${number}`) }),
    );
  fireEvent.click(start());
  expect((await within(dialog()).findByTestId('pr-refresh-report')).textContent).toBe(
    'Requested 3: 2 refreshed, 1 failed, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
  expect(within(dialog()).getByText(/Failed: rate limited; earlier facts are kept/)).toBeTruthy();
  await waitFor(() =>
    expect(within(dialog()).getByText(/failed \(rate limited\); never refreshed/)).toBeTruthy(),
  );
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(tile().textContent).toContain('0 + 1 unknown'));
  expect(screen.getByTestId('pr-refresh').textContent).toContain(
    'Confirmed-linked pull requests: 1 refreshed, 1 failed, never refreshed',
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

it('selects at most twenty pull requests and sends exactly those', async () => {
  const source = nativeSource(sectioned(NO_LINKS), {
    pullRequests: async () => ({ rows: rows(25) }),
    refreshPullRequests: async (ids) => cancelledReport(ids),
  });
  mount(source);
  await loaded();
  fireEvent.click(refreshButton());
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
  const source = nativeSource(sectioned(NO_LINKS), {
    pullRequests: async () => ({ rows: rows(2) }),
    refreshPullRequests: () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
    cancelPullRequestRefresh: async () => true,
  });
  mount(source);
  await loaded();
  const reads = source.dashboard.mock.calls.length;
  fireEvent.click(refreshButton());
  for (const box of await within(dialog()).findAllByRole('checkbox')) fireEvent.click(box);
  fireEvent.click(start());
  const cancel = await within(dialog()).findByRole('button', { name: 'Cancel' });
  expect(screen.getByText('Refreshing pull-request facts…')).toBeTruthy();
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
    'Requested 2: 0 refreshed, 0 failed, 2 skipped. The batch was cancelled. No stored facts changed.',
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
  fireEvent.click(refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#100/ }));
  fireEvent.click(start());
  expect((await within(dialog()).findByTestId('pr-refresh-error')).textContent).toBe(
    'The refresh did not run: The GitHub CLI was not found. Install gh, then try again.',
  );
  expect(source.dashboard).toHaveBeenCalledTimes(reads);
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(tile().textContent).toContain('1 + 1 unknown');
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
  mount(nativeSource(sectioned(MANY_TYPES), { pullRequests: async () => ({ rows: [stale] }) }));
  await loaded();
  fireEvent.click(refreshButton());
  const item = (
    await within(dialog()).findByRole('checkbox', { name: /#100 · feat: kept/ })
  ).closest('li')!;
  expect(item.textContent).toContain('merged Sep 5, 12:00');
  expect(item.textContent).toContain(
    'stale: last refresh failed (rate limited); facts from Sep 6, 00:00',
  );
});

it('opens from the keyboard and returns focus to its button on Escape', async () => {
  mount(nativeSource(sectioned(NO_LINKS), { pullRequests: async () => ({ rows: rows(1) }) }));
  await loaded();
  const button = refreshButton();
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
  fireEvent.click(refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#11/ }));
  fireEvent.click(start());
  await within(dialog()).findByTestId('pr-refresh-report');
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  // #11 is refreshed; the SHA link #12 has no facts yet, so the count stays
  // unknown; the inferred #13 is not counted by the confirmed-only Dashboard.
  await waitFor(() => expect(tile().textContent).toContain('0 + 1 unknown'));
  expect(screen.getByTestId('pr-refresh').textContent).toContain(
    'Confirmed-linked pull requests: 1 refreshed, 1 never refreshed',
  );
});

it('keeps a running batch, its Cancel and its result across a range change and a route return', async () => {
  let finish: (report: PrRefreshReport) => void = () => {};
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const source = nativeSource(sectioned(NO_LINKS), {
    pullRequests: async () => ({ rows: rows(2) }),
    refreshPullRequests: () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
    cancelPullRequestRefresh: async () => true,
  });
  source.dashboard.mockImplementation(async (days: number) => {
    if (days === 14) await gate;
    return sectioned(NO_LINKS)(days);
  });
  mount(source);
  await loaded();
  fireEvent.click(refreshButton());
  for (const box of await within(dialog()).findAllByRole('checkbox')) fireEvent.click(box);
  fireEvent.click(start());
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(screen.getByText('Refreshing pull-request facts…')).toBeTruthy();
  // An uncached range replaces the whole report, this control included.
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await screen.findByText(/Reading Dashboard metrics for the last 14d/);
  expect(screen.queryByTestId('pr-refresh')).toBeNull();
  await act(async () => release());
  await loaded();
  // The batch is still running, still cancellable, and cannot be doubled.
  expect(screen.getByText('Refreshing pull-request facts…')).toBeTruthy();
  fireEvent.click(refreshButton());
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
  await waitFor(() => expect(screen.queryByTestId('pr-refresh')).toBeNull());
  await act(async () => finish(cancelledReport([1, 2])));
  act(() => go('/dashboard'));
  await loaded();
  expect(screen.queryByText('Refreshing pull-request facts…')).toBeNull();
  fireEvent.click(refreshButton());
  expect((await within(dialog()).findByTestId('pr-refresh-report')).textContent).toBe(
    'Requested 2: 0 refreshed, 0 failed, 2 skipped. The batch was cancelled. No stored facts changed.',
  );
  expect(source.refreshPullRequests).toHaveBeenCalledOnce();
});

it('a replaced source starts with no batch and ignores the old one', async () => {
  let finish: (report: PrRefreshReport) => void = () => {};
  const first = nativeSource(sectioned(NO_LINKS), {
    pullRequests: async () => ({ rows: rows(1) }),
    refreshPullRequests: () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  });
  const second = nativeSource(sectioned(NO_LINKS), {
    pullRequests: async () => ({ rows: rows(1) }),
  });
  const view = mount(first);
  await loaded();
  fireEvent.click(refreshButton());
  fireEvent.click(await within(dialog()).findByRole('checkbox', { name: /#100/ }));
  fireEvent.click(start());
  fireEvent.keyDown(dialog(), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  view.rerender(tree(second));
  await loaded();
  expect(screen.queryByText('Refreshing pull-request facts…')).toBeNull();
  const reads = second.dashboard.mock.calls.length;
  await act(async () => finish({ ...cancelledReport([1]), committed: true }));
  expect(screen.queryByText('Refreshing pull-request facts…')).toBeNull();
  fireEvent.click(refreshButton());
  await within(dialog()).findByRole('checkbox', { name: /#100/ });
  expect(within(dialog()).queryByTestId('pr-refresh-report')).toBeNull();
  expect(second.dashboard).toHaveBeenCalledTimes(reads);
  expect(second.refreshPullRequests).not.toHaveBeenCalled();
});
