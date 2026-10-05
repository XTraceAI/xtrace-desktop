import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import type { DashboardLaneCost } from '../../data/generated/DashboardLaneCost';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { MetricTile } from '../../data/generated/MetricTile';
import type { MetricTokenSummary } from '../../data/generated/MetricTokenSummary';
import { events, type DataEvent } from '../../data/ipc-names';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const f1 = (days = 7) => structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

/** The grouped lane table's body rows, without its header row. */
const laneRows = () =>
  within(screen.getByRole('table', { name: 'Session lanes' }))
    .getAllByRole('row')
    .slice(1);
const laneSpans = (row: HTMLElement) => [...row.querySelectorAll<HTMLElement>('.xt-lane-span')];

type Edit = (report: DashboardMetrics) => void;
/** Synthetic reports: the generated F1 export with the named fields replaced. */
const synthetic = (edit: Edit, days = 7) => {
  const report = f1(days);
  edit(report);
  return report;
};
/** A lane session's cost: every response priced, unless the patch says otherwise. */
const laneCost = (total: number, patch: Partial<DashboardLaneCost> = {}): DashboardLaneCost => ({
  total_usd: total,
  priced_subtotal_usd: total,
  selected_observations: 1,
  priced_observations: 1,
  unpriced_observations: 0,
  assumed_tier_observations: 0,
  unpriced: [],
  ...patch,
});
const tile = (value: number | null, patch: Partial<MetricTile> = {}): MetricTile => ({
  value,
  unit: 'test',
  rule_id: 'M-05',
  reason: value === null ? 'Synthetic unknown reason' : null,
  note: null,
  current_n: 10,
  previous_n: 10,
  sample_unit: 'sessions',
  delta: { previous: value, pct: null, suppressed: true },
  ...patch,
});
const tokens = (
  counters: Partial<MetricTokenSummary['counters']>,
  selected = 1,
): MetricTokenSummary => ({
  selected_responses: selected,
  measured_responses: selected,
  sessions: 1,
  measured_sessions: 1,
  counters: {
    input_tokens: null,
    output_tokens: null,
    cache_read_tokens: null,
    cache_creation_tokens: null,
    total_tokens: null,
    ...counters,
  },
});

function nativeSource(report: (days: number) => DashboardMetrics | Promise<DashboardMetrics>) {
  const listeners = new Map<DataEvent, Set<() => void>>();
  const source = {
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
    tokensByHost: vi.fn(async (days: number) => {
      const current = await report(days);
      return { window: current.window, hosts: current.tokens_by_host };
    }),
    today: async () => exported.today,
    environment: vi.fn(async (days: number) => {
      const report = exported.environments.find((entry) => entry.window.days === days);
      if (!report) throw new Error('Metric range must be 7, 14, or 30 days.');
      return report;
    }),
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: async (event: DataEvent, listener: () => void) => {
      const set = listeners.get(event) ?? new Set();
      set.add(listener);
      listeners.set(event, set);
      return () => void set.delete(listener);
    },
    emit: (event: DataEvent) => listeners.get(event)?.forEach((listener) => listener()),
  } satisfies DataSource & { emit: (event: DataEvent) => void };
  return source;
}

function mount(source: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/dashboard']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}
const loaded = () => screen.findByTestId('dashboard-summary');
const card = (title: string) =>
  screen.getByRole('heading', { level: 2, name: title }).closest('section')!;
const tileNamed = (label: string) =>
  screen.getAllByRole('button').find((button) => button.textContent?.startsWith(label))!;
const METHOD = 'How effort is counted · daily values';
/** The report period each preset selects over the generated F1 export. */
const PERIOD = {
  '7d': /^Sep 1\s–\s7, 2026/,
  '14d': /^Aug 25\s–\sSep 7, 2026/,
  '30d': /^Aug 9\s–\sSep 7, 2026/,
} as const;
const period = () => screen.getByTestId('report-period').textContent;
/**
 * The Dashboard draws neither Agent / human hours nor Caught by your rules: no
 * heading, value, daily bar, control or unavailable line of either is on the page.
 */
function expectHiddenPanels() {
  for (const name of ['Agent / human hours', 'Caught by your rules']) {
    expect(screen.queryByRole('heading', { name })).toBeNull();
    expect(screen.queryByRole('button', { name: `${name} definition` })).toBeNull();
  }
  for (const name of [/^Agent hours/, /^Human-in-the-loop hours/, /^Agent to human ratio/])
    expect(screen.queryByRole('button', { name })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Daily values' })).toBeNull();
  expect(document.querySelector('.xt-hero-chart, .xt-hero-day, .xt-roll')).toBeNull();
  const page = document.querySelector('.xt-dashboard')!.textContent;
  expect(page).not.toMatch(/human est\.|Rule-fire data is unavailable/);
}
/**
 * The control that opens a supplementary measurement from the card it belongs
 * to: tokens and cost are sections of Effort's Method, coverage (with untimed
 * history) opens from the Sessions header.
 */
const detailTrigger = (title: string) =>
  title === 'Coverage'
    ? within(card('Sessions')).getByRole('button', { name: /^Coverage/ })
    : within(card('Effort')).getByRole('button', { name: 'Method' });
const usageSection: Record<string, string> = {
  'Tokens per day': 'usage-tokens',
  'API-equivalent cost': 'usage-cost',
};
/** Opens one measurement and returns it: the Coverage dialog, or its titled section of Method. */
async function detail(title: string) {
  fireEvent.click(detailTrigger(title));
  if (title === 'Coverage') return screen.findByRole('dialog', { name: title });
  const method = await screen.findByRole('dialog', { name: METHOD });
  return within(method).getByTestId(usageSection[title]!);
}
/** Closes the open dialog the way a keyboard does, and waits until it is gone. */
async function closeDialog(detail: HTMLElement) {
  fireEvent.keyDown(detail.closest<HTMLElement>('[role="dialog"]') ?? detail, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
}

it('renders the generated F1 report with honest unknowns and the shared sidebar totals', async () => {
  const source = nativeSource((days) => f1(days));
  mount(source);
  await loaded();
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('What your agents did');
  // The report period sits in the TopBar, immediately before the range presets.
  const period = await screen.findByTestId('report-period');
  expect(period.textContent).toMatch(/^Sep 1\s–\s7, 2026, time zone UTC$/);
  expect(period.getAttribute('title')).toMatch(/Sep 1\s–\s7, 2026 · UTC/);
  expect(
    period
      .closest('.xt-topbar-tools')!
      .contains(screen.getByRole('radiogroup', { name: /range/i })),
  ).toBe(true);
  const summary = screen.getByTestId('dashboard-summary').textContent;
  expect(summary).not.toMatch(/2026|UTC/);
  expect(summary).toContain('1 session');
  expect(summary).toContain('5 human messages');
  expect(summary).toContain('5 tool calls');
  expect(summary).toContain('fixture-model-v1');
  expect(source.dashboard).toHaveBeenCalledWith(7);
  expect(source.tokensByHost).toHaveBeenCalledWith(7);
  expectHiddenPanels();
  // Tiles keep full labels; merged PRs is unknown with its reason, never zero.
  expect(tileNamed('Agent h/day').textContent).toContain('0.1h');
  expect(tileNamed('Concurrency').textContent).toContain('max 1');
  // F1's confirmed links have never been refreshed: the count is unknown, and
  // the known subtotal is named as a subtotal, not as the answer.
  expect(tileNamed('Merged PRs').textContent).toContain(
    'Unmeasured: 0 known merged; 2 linked pull requests have no cached merge facts',
  );
  expect(tileNamed('Merged PRs').textContent).toContain('0 + 2 unknown');
  expect(tileNamed('Hands-off median').textContent).toContain('p90 3');
  expect(card('Effort').textContent).not.toContain('unavailable');
  // Environment reads its own observed usage instead of the report's unavailable entry.
  expect(
    (await within(card('Environment')).findByTestId('environment-empty')).textContent,
  ).toContain('No non-built-in tool calls were observed in the last 7d');
  expect(within(card('Environment')).queryByTestId('environment-summary')).toBeNull();
  expect(card('Environment').textContent).not.toContain('Environment data is unavailable');
  // F1 is legitimately unpriced: no total, no subtotal masquerading as one.
  const costDialog = await detail('API-equivalent cost');
  const cost = within(costDialog);
  expect(cost.getByTestId('cost-total').textContent).toContain('Unmeasured');
  expect(cost.getByTestId('cost-subtotal').textContent).toBe(
    'None of 10 selected responses could be priced.',
  );
  expect(cost.getByText('service tier not recorded · 10 responses')).toBeTruthy();
  expect(costDialog.textContent).toContain('catalog synthetic-v1 · as of 2026-09-17');
  expect(cost.getByText(/Global public token API-equivalent/)).toBeTruthy();
  await closeDialog(costDialog);
  // Coverage: 100 percentage points display as 100%. Nothing checks the session
  // inventory yet, so its unknown state is not shown anywhere in Coverage.
  expect(detailTrigger('Coverage').textContent).toBe('Coverage');
  expect(detailTrigger('Coverage').getAttribute('aria-label')).toBeNull();
  const coverage = await detail('Coverage');
  expect(coverage.textContent).toContain('tokens measured for 1/1 session');
  expect(coverage.textContent).not.toMatch(/inventory/i);
  expect(within(coverage).getByTestId('usage-coverage').textContent).toContain(
    '100% of sessions (1 of 1',
  );
  // No coverage rows cannot prove that no receipts exist.
  expect(within(coverage).getByText('No session capture coverage rows available.')).toBeTruthy();
  expect(coverage.textContent).not.toContain('No capture receipts');
  expect(within(coverage).getByText(/Receipts are historical/)).toBeTruthy();
  await closeDialog(coverage);
  // The index's state is the sidebar's and Settings', not a Dashboard line.
  expect(screen.queryByTestId('dashboard-index')).toBeNull();
  expect(screen.queryByText(/Native index/)).toBeNull();
  // Account limits remain separate from the Dashboard's recorded tokens.
  expect(screen.getByRole('region', { name: 'Account usage' })).toBeTruthy();
  // Sessions measures over the selected window, so the link into it carries
  // the range the Dashboard was showing.
  expect(screen.getByRole('link', { name: 'View sessions' }).getAttribute('href')).toBe(
    '/sessions?range=7d',
  );
});

it('shows F2 concurrency and overlapping lanes on the fixed recent axis', async () => {
  const start = f1().lane_start_ms;
  const minute = 60_000;
  const lanes = [
    {
      session_id: 'f2-session-c',
      host: 'claude',
      start_ms: start + 40 * minute,
      end_ms: start + 60 * minute,
    },
    {
      session_id: 'f2-session-b',
      host: 'claude',
      start_ms: start + 20 * minute,
      end_ms: start + 50 * minute,
    },
    {
      session_id: 'f2-session-a',
      host: 'claude',
      start_ms: start + 10 * minute,
      end_ms: start + 50 * minute,
    },
  ];
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.tiles.concurrency_mean = tile(1.8, { rule_id: 'M-06' });
        report.tiles.concurrency_max = tile(3, { rule_id: 'M-06' });
        report.lanes = lanes;
        report.lanes_total = 3;
      }),
    ),
  );
  await loaded();
  // The primary number is the mean with no visible unit; its definition says
  // so, from focus as from hover, and the max stays beside it.
  expect(tileNamed('Concurrency').textContent).toBe('Concurrency1.8max 3');
  fireEvent.focus(tileNamed('Concurrency'));
  const tip = await screen.findByRole('tooltip');
  expect(tip.textContent).toMatch(
    /^Concurrency\..*Displayed value is mean concurrency; max is the peak overlap\./,
  );
  expect(tip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
  expect(tileNamed('Concurrency').getAttribute('aria-describedby')).toBe(tip.id);
  fireEvent.blur(tileNamed('Concurrency'));
  const rows = laneRows();
  expect(rows).toHaveLength(3);
  const spans = rows.map((row) => laneSpans(row)[0].style);
  // 48 hours = 2880 minutes; b (20–50) overlaps both a (10–50) and c (40–60).
  const expected = [
    [40, 20],
    [20, 30],
    [10, 40],
  ];
  spans.forEach((style, index) => {
    expect(parseFloat(style.left)).toBeCloseTo((expected[index][0] / 2880) * 100, 9);
    expect(parseFloat(style.width)).toBeCloseTo((expected[index][1] / 2880) * 100, 9);
  });
  expect(rows[0].textContent).toContain(
    'claude session f2-session-c, repository unknown, branch unknown: 1 active span',
  );
  // The card states no caption under the rows and no line under its title.
  expect(screen.queryByTestId('lanes-disclosure')).toBeNull();
  expect(card('Sessions').textContent).not.toContain('One row per session');
  expect(card('Sessions').textContent).not.toContain('active spans over the last');
});

it('keeps F7 unmeasured tokens, cost, coverage and favorite model explicit', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.tiles.tokens = tile(null, { reason: 'No selected usage was measured' });
        report.tiles.cost = tile(null, { reason: 'Selected usage is absent or unpriced' });
        report.tokens = { ...tokens({}, 0), sessions: 1, measured_sessions: 0 };
        report.tokens_by_host = [{ host: 'cursor', tokens: report.tokens }];
        report.days = report.days.map((day) => ({ ...day, tokens: tokens({}, 0) }));
        report.cost = {
          ...report.cost,
          total_usd: null,
          priced_subtotal_usd: 0,
          selected_observations: 0,
          priced_observations: 0,
          unpriced_observations: 0,
          unpriced: [],
        };
        const usage = { sessions: 1, measured: 0, pct: 0, gaps: ['no_selected_usage' as const] };
        report.usage_coverage = {
          total: usage,
          by_host: [{ host: 'cursor', usage }],
          by_surface: [{ host: 'cursor', surface: 'cli', usage }],
        };
        report.favorite.current = {
          model: null,
          output_tokens: null,
          unknown_reason: 'no_measured_output',
        };
      }),
    ),
  );
  await loaded();
  expect(screen.getByTestId('dashboard-summary').textContent).toContain(
    'Unmeasured: No measured output tokens in this range',
  );
  const tokensDialog = await detail('Tokens per day');
  expect(tokensDialog.textContent).toContain('total unmeasured · 0/1 session measured');
  expect(
    within(tokensDialog).getAllByRole('img', { name: /output tokens: no recorded usage/ }),
  ).toHaveLength(7);
  await closeDialog(tokensDialog);
  const costDialog = await detail('API-equivalent cost');
  expect(within(costDialog).getByTestId('cost-total').textContent).toContain(
    'Unmeasured: Selected usage is absent or unpriced',
  );
  expect(within(costDialog).getByTestId('cost-subtotal').textContent).toBe(
    'No selected usage to price in this range.',
  );
  await closeDialog(costDialog);
  // Measured 0 percentage points is a real 0%, with the gap named.
  const coverage = await detail('Coverage');
  expect(within(coverage).getByTestId('usage-coverage').textContent).toContain(
    '0% of sessions (0 of 1 session) · no recorded usage',
  );
  const surfaces = within(
    within(coverage).getByRole('list', { name: 'Token measurement by surface' }),
  );
  expect(surfaces.getByText('cursor · cli')).toBeTruthy();
  await closeDialog(coverage);
  // Cursor is not a quota: an unmeasured host total shows a dash.
  expect(screen.getByRole('region', { name: 'Account usage' })).toBeTruthy();
});

it('converts F11 percentage-point deltas exactly once and hides suppressed ones', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.tiles.agent_hours_per_day = tile(1.5, {
          delta: { previous: 0.75, pct: 100, suppressed: false },
        });
        report.tiles.hands_off_median = tile(3, {
          rule_id: 'M-09',
          sample_unit: 'stretches',
          delta: { previous: 4, pct: -25, suppressed: false },
        });
        report.tiles.concurrency_mean = tile(2, {
          current_n: 1,
          previous_n: 1,
          delta: { previous: 1, pct: null, suppressed: true },
        });
      }),
    ),
  );
  await loaded();
  expect(tileNamed('Agent h/day').querySelector('.xt-stat-delta')!.textContent).toBe('▲100%');
  expect(tileNamed('Hands-off median').querySelector('.xt-stat-delta')!.textContent).toBe('▼25%');
  expect(tileNamed('Concurrency').querySelector('.xt-stat-delta')).toBeNull();
  expect(document.body.textContent).not.toContain('5,000%');
});

it('keeps a partial API-equivalent subtotal out of the total slot and names unpriced tiers', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.cost = {
          ...report.cost,
          total_usd: null,
          priced_subtotal_usd: 0.42,
          selected_observations: 3,
          priced_observations: 1,
          unpriced_observations: 2,
          assumed_tier_observations: 1,
          unpriced: [
            {
              model: 'synthetic-model',
              service_tier: null,
              reason: 'missing_service_tier',
              observations: 2,
            },
          ],
        };
        const usage = {
          sessions: 3,
          measured: 2,
          pct: 66.7,
          gaps: ['incomplete_counters' as const],
        };
        report.usage_coverage = { total: usage, by_host: [], by_surface: [] };
        report.usage_gate_14d = { ...report.usage_gate_14d, pct: 66.7, passes: false };
        report.capture_inventory = 'fresh_complete';
      }),
    ),
  );
  await loaded();
  // A complete inventory with no untimed history still flags, unopened, token
  // measurement that misses sessions with its gap and the failed 14-day gate.
  expect(detailTrigger('Coverage').textContent).toBe('Coverage');
  expect(detailTrigger('Coverage').getAttribute('aria-label')).toBe(
    'Coverage: tokens measured for 2/3 sessions (incomplete token counters), trailing 14 days below the 90% gate',
  );
  // No subtotal where a total belongs; the detail names it as partial.
  const costDialog = await detail('API-equivalent cost');
  expect(within(costDialog).getByTestId('cost-total').textContent).not.toContain('$');
  expect(within(costDialog).getByTestId('cost-subtotal').textContent).toBe(
    'Partial subtotal $0.42 covers only 1 of 3 selected responses; it is not a total.',
  );
  expect(within(costDialog).getByText('service tier not recorded · 2 responses')).toBeTruthy();
  // A Codex response priced at the default tier says so beside the counts.
  expect(within(costDialog).getByTestId('cost-assumed-tier').textContent).toBe(
    '1 Codex response at OpenAI’s default tier (no tier recorded)',
  );
  await closeDialog(costDialog);
  const coverage = await detail('Coverage');
  expect(within(coverage).getByTestId('usage-coverage').textContent).toContain(
    '66.7% of sessions (2 of 3 sessions) · incomplete token counters',
  );
  await closeDialog(coverage);
});

it('leaves an unknown inventory out of Coverage but names a known incomplete one', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.capture_coverage = [
          {
            host: 'claude',
            surface: 'cli',
            inventory: 'unknown',
            observed_sessions: 2,
            captured_sessions: 1,
            unknown_start_sessions: 0,
            pct: 50,
            incomplete_reasons: ['inventory_unknown', 'missing_native_start'],
          },
        ];
      }),
    ),
  );
  await loaded();
  let coverage = await detail('Coverage');
  const rows = within(coverage).getByRole('list', { name: 'Session capture by surface' });
  expect(rows.textContent).toContain('native start time missing');
  expect(coverage.textContent).not.toMatch(/inventory/i);
  await closeDialog(coverage);
  cleanup();
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.capture_inventory = 'incomplete';
      }),
    ),
  );
  await loaded();
  expect(detailTrigger('Coverage').getAttribute('aria-label')).toMatch(
    /^Coverage: Inventory incomplete/,
  );
  coverage = await detail('Coverage');
  expect(within(coverage).getByText('Inventory incomplete')).toBeTruthy();
  await closeDialog(coverage);
});

it('shows a complete total, measured zeros and hands-off exclusions', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.cost = {
          ...report.cost,
          total_usd: 1.234,
          priced_subtotal_usd: 1.234,
          unpriced: [],
        };
        report.tiles.sessions = tile(0);
        report.tiles.human_messages = tile(0);
        report.tiles.hands_off_median = tile(null, {
          rule_id: 'M-09',
          reason: 'No hands-off stretches after exclusions',
        });
        report.hands_off_excluded_surfaces = [
          { host: 'claude', surface: 'batch', qualifying_sessions: 4, degenerate_sessions: 2 },
        ];
        report.capture_inventory = 'fresh_complete';
        report.days[0].tokens = tokens({ output_tokens: 0, input_tokens: 5 });
        report.days[1].tokens = tokens({ input_tokens: 5 });
      }),
    ),
  );
  await loaded();
  const costDialog = await detail('API-equivalent cost');
  expect(within(costDialog).getByTestId('cost-total').textContent).toBe('Total$1.23');
  expect(within(costDialog).queryByTestId('cost-subtotal')).toBeNull();
  await closeDialog(costDialog);
  expect(screen.getByTestId('dashboard-summary').textContent).toContain('0 sessions');
  expect(screen.getByText(/No agent activity was recorded in this range/)).toBeTruthy();
  expect(tileNamed('Hands-off median').textContent).toContain(
    'Unmeasured: No hands-off stretches after exclusions',
  );
  // A complete inventory, every session's tokens measured, no failed gate and no untimed
  // history: nothing to flag in the control's name.
  expect(detailTrigger('Coverage').getAttribute('aria-label')).toBeNull();
  const coverage = await detail('Coverage');
  expect(
    within(
      within(coverage).getByRole('list', { name: 'Surfaces excluded from hands-off' }),
    ).getByText('claude · batch: 2 of 4 qualifying sessions have batch-stamped timestamps'),
  ).toBeTruthy();
  const pill = within(coverage).getByText('Inventory complete');
  expect(pill.closest('[data-tone]')!.getAttribute('data-tone')).toBe('success');
  await closeDialog(coverage);
  // Measured zero, unmeasured and no-usage days are distinguishable per series.
  const tokensDialog = await detail('Tokens per day');
  const output = within(within(tokensDialog).getByRole('group', { name: 'Output tokens per day' }));
  const [zero, unknown, none] = output.getAllByRole('img');
  expect(zero.getAttribute('aria-label')).toBe('2026-09-01, output tokens: 0');
  expect(zero.getAttribute('data-zero')).toBe('true');
  expect(unknown.getAttribute('aria-label')).toBe('2026-09-02, output tokens: unmeasured');
  expect(unknown.getAttribute('data-state')).toBe('unknown');
  expect(none.getAttribute('data-state')).toBe('none');
});

it('discloses truncated lanes and renders every day of a 30-day range', async () => {
  const source = nativeSource((days) =>
    synthetic((report) => {
      const base = report.lane_end_ms - 3_600_000;
      report.lanes = Array.from({ length: 200 }, (_, index) => ({
        session_id: `synthetic-${index}`,
        host: 'codex',
        start_ms: base - index * 1000,
        end_ms: base,
      }));
      report.lanes_total = 205;
      report.lanes_truncated = true;
    }, days),
  );
  mount(source);
  await loaded();
  expect(laneRows()).toHaveLength(200);
  expect(screen.queryByTestId('lanes-disclosure')).toBeNull();
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(period()).toMatch(PERIOD['30d']));
  // The daily chart follows the range, one dialog away.
  const tokensDialog = await detail('Tokens per day');
  expect(
    within(
      within(tokensDialog).getByRole('group', { name: 'Fresh input tokens per day' }),
    ).getAllByRole('img'),
  ).toHaveLength(30);
  await closeDialog(tokensDialog);
  expect(screen.getByRole('button', { name: 'Custom range' }).hasAttribute('disabled')).toBe(true);
});

it('moves Dashboard reports to the selected range without a quota caption', async () => {
  const source = nativeSource((days) => f1(days));
  mount(source);
  await loaded();
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() =>
    expect(screen.getByTestId('report-period').textContent).toMatch(/^Aug 25\s–\sSep 7, 2026/),
  );
  expect(source.dashboard).toHaveBeenLastCalledWith(14);
  expect(source.tokensByHost).toHaveBeenLastCalledWith(14);
  expect(screen.getByRole('region', { name: 'Account usage' })).toBeTruthy();
  // Sessions measures the same selected window, so it keeps the shared range
  // control and selection rather than showing numbers from an implied period.
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(screen.getByRole('radio', { name: '14d' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('region', { name: 'Account usage' })).toBeTruthy();
  // The pull requests report measures the same window too.
  fireEvent.click(screen.getByRole('button', { name: 'Pull requests' }));
  expect(screen.getByRole('radio', { name: '14d' }).getAttribute('aria-checked')).toBe('true');
  // A route with no window of its own keeps the selection without the control.
  fireEvent.click(screen.getByRole('button', { name: 'Rulebook' }));
  expect(screen.queryByRole('radiogroup', { name: 'Date range' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Dashboard' }));
  expect((await screen.findByRole('radio', { name: '14d' })).getAttribute('aria-checked')).toBe(
    'true',
  );
});

it('shows loading, then a safe error with retry', async () => {
  let reject = true;
  const source = nativeSource(async (days) => {
    if (reject) throw new Error('backend-specific detail');
    return f1(days);
  });
  mount(source);
  expect(await screen.findByText(/Reading Dashboard metrics for the last 7d/)).toBeTruthy();
  expect(
    await screen.findByText('Dashboard metrics could not be loaded.', { exact: false }),
  ).toBeTruthy();
  expect(screen.queryByText(/backend-specific detail/)).toBeNull();
  reject = false;
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  expect(await loaded()).toBeTruthy();
});

it('refreshes on insertion and on enrichment that adds no session', async () => {
  let state: 'initial' | 'enriched' | 'inserted' = 'initial';
  const source = nativeSource((days) =>
    synthetic((report) => {
      if (state === 'initial') return;
      // Enrichment of an existing UUID: same session count, more measured tokens.
      report.tokens.counters.total_tokens = 1250;
      report.tokens_by_host[0].tokens.counters.total_tokens = 1250;
      if (state === 'inserted') report.tiles.sessions = tile(2);
    }, days),
  );
  mount(source);
  await loaded();
  expect(screen.getByRole('region', { name: 'Account usage' })).toBeTruthy();
  state = 'enriched';
  act(() => source.emit(events.nativeIndexStatus));
  await waitFor(() => expect(source.tokensByHost).toHaveBeenCalledTimes(2));
  // A measurement that is open reads the enriched report as it arrives.
  const tokensDialog = await detail('Tokens per day');
  await waitFor(() => expect(tokensDialog.textContent).toContain('1.25K recorded'));
  await closeDialog(tokensDialog);
  expect(screen.getByTestId('dashboard-summary').textContent).toContain('1 session');
  state = 'inserted';
  act(() => source.emit(events.importReceived));
  await waitFor(
    () => expect(screen.getByTestId('dashboard-summary').textContent).toContain('2 sessions'),
    { timeout: 3000 },
  );
});

it('does not read metrics in a browser preview', async () => {
  const source = { ...nativeSource((days) => f1(days)), kind: 'preview' as const };
  mount(source);
  expect(
    await screen.findByText('Open the desktop app to read local Dashboard metrics.'),
  ).toBeTruthy();
  expect(source.dashboard).not.toHaveBeenCalled();
  expect(source.tokensByHost).not.toHaveBeenCalled();
});

it('groups a session\u2019s spans into one row and still draws every span', async () => {
  const errors = vi.spyOn(console, 'error');
  const start = f1().lane_start_ms;
  const minute = 60_000;
  // Same host and session, separated by more than the 20-minute span gap.
  const lanes = [
    {
      session_id: 'split-session',
      host: 'claude',
      start_ms: start + 90 * minute,
      end_ms: start + 100 * minute,
    },
    {
      session_id: 'split-session',
      host: 'claude',
      start_ms: start + 10 * minute,
      end_ms: start + 30 * minute,
    },
    {
      session_id: 'split-session',
      host: 'codex',
      start_ms: start + 10 * minute,
      end_ms: start + 30 * minute,
    },
  ];
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.lanes = lanes;
        report.lanes_total = 3;
      }),
    ),
  );
  await loaded();
  // One row per session, not per span; the codex lane keeps its own row
  // because a row's glyph can only name one host.
  const rows = laneRows();
  expect(rows).toHaveLength(2);
  const claudeRow = rows.find((row) => row.textContent?.includes('claude session split-session'))!;
  const codexRow = rows.find((row) => row.textContent?.includes('codex session split-session'))!;
  // Every returned span is still drawn, on the same fixed 48-hour axis.
  expect(laneSpans(claudeRow)).toHaveLength(2);
  expect(laneSpans(codexRow)).toHaveLength(1);
  expect(laneSpans(claudeRow).map((span) => parseFloat(span.style.left))).toEqual([
    (90 / 2880) * 100,
    (10 / 2880) * 100,
  ]);
  expect(claudeRow.textContent).toContain('2 active spans');
  // No first-seen column: the spans' earliest start is not a session start.
  expect(within(claudeRow).queryByText('Sep 6, 00:10')).toBeNull();
  expect(errors.mock.calls.some((call) => String(call[0]).includes('same key'))).toBe(false);
});

it('names a lane by its indexed repository, branch and identity, and prices it', async () => {
  const start = f1().lane_start_ms;
  const minute = 60_000;
  const lane = (session_id: string, host: string, offset: number) => ({
    session_id,
    host,
    start_ms: start + offset * minute,
    end_ms: start + (offset + 10) * minute,
  });
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.lanes = [
          lane('ctx-repo', 'claude', 60),
          lane('ctx-bare', 'codex', 30),
          lane('ctx-zero', 'claude', 10),
        ];
        report.lanes_total = 3;
        report.lane_sessions = [
          {
            session_id: 'ctx-bare',
            title: null,
            automated_review: false,
            started_at_ms: null,
            pr_links: 0,
            inferred_pr_links: 0,
            host: 'codex',
            repo: null,
            branch: null,
            // Indexed, but nothing selected to price.
            cost: laneCost(0, {
              total_usd: null,
              selected_observations: 0,
              priced_observations: 0,
            }),
          },
          {
            session_id: 'ctx-repo',
            title: null,
            automated_review: false,
            started_at_ms: null,
            pr_links: 0,
            inferred_pr_links: 0,
            host: 'claude',
            repo: '/Users/dev/code/acme-api',
            branch: 'feature/lanes',
            cost: laneCost(12.4, { selected_observations: 3, priced_observations: 3 }),
          },
          {
            session_id: 'ctx-zero',
            title: null,
            automated_review: false,
            started_at_ms: null,
            pr_links: 0,
            inferred_pr_links: 0,
            host: 'claude',
            repo: null,
            branch: null,
            cost: laneCost(0),
          },
        ];
      }),
    ),
  );
  await loaded();
  const [repo, bare, zero] = laneRows();
  // The repository's own name leads the row, with its branch and enough of the
  // identity to tell two rows apart.
  expect(repo.querySelector('.xt-lane-repo')!.textContent).toBe('acme-api · feature/lanes');
  expect(repo.querySelector('.xt-lane-find')!.textContent).toBe('Session ctx-repo');
  expect(repo.querySelector('.xt-lane-repo')!.getAttribute('title')).toBe(
    '/Users/dev/code/acme-api · feature/lanes',
  );
  // A history carrying neither fact says so; the identity stays visible.
  expect(bare.querySelector('.xt-lane-repo')!.textContent).toBe('Unknown repository');
  expect(bare.querySelector('.xt-lane-find')!.textContent).toBe('Session ctx-bare');
  // The link opens that one session, names it in full, and carries the list
  // state that would find it so the page it opens can offer a way back.
  expect(
    within(repo).getByRole('link', { name: 'Open session ctx-repo' }).getAttribute('href'),
  ).toBe('/sessions/ctx-repo?q=ctx-repo&host=claude&range=7d');
  // A priced cost, a measured zero and an unknown one stay distinct.
  const costCell = (row: HTMLElement) => [...row.querySelectorAll('[role="cell"]')].at(-1)!;
  expect(costCell(repo).textContent).toBe('$12.40');
  expect(costCell(zero).textContent).toBe('$0.00');
  expect(
    within(bare).getByText(
      'Unmeasured: No selected responses in this window, so there is nothing to price',
    ),
  ).toBeTruthy();
  // The row's own description carries the context and the measurement.
  expect(repo.textContent).toContain(
    'claude session ctx-repo, repository /Users/dev/code/acme-api, branch feature/lanes: 1 active span',
  );
  expect(repo.textContent).toContain(
    'Over the whole 48 hours: $12.40 API-equivalent cost of 3 responses.',
  );
  expect(bare.textContent).toContain('Over the whole 48 hours: no selected responses to price.');
});

it('aligns lane columns to session, Compactions, PRs, started, activity and cost', async () => {
  const start = f1().lane_start_ms;
  const minute = 60_000;
  const lane = (session_id: string, offset: number) => ({
    session_id,
    host: 'claude',
    start_ms: start + offset * minute,
    end_ms: start + (offset + 10) * minute,
  });
  const context = {
    host: 'claude',
    repo: '/Users/dev/code/acme-api',
    branch: 'main',
    cost: laneCost(0.42),
  };
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.lanes = [
          lane('titled', 90),
          lane('untitled', 60),
          lane('unlinked', 30),
          lane('orphan', 10),
        ];
        report.lanes_total = 4;
        report.lane_sessions = [
          {
            ...context,
            session_id: 'titled',
            title: 'Saved title',
            // Months before the 48-hour axis: shown as the recorded start.
            automated_review: false,
            started_at_ms: Date.UTC(2026, 5, 1, 8, 30),
            pr_links: 3,
            inferred_pr_links: 1,
          },
          {
            ...context,
            session_id: 'untitled',
            title: null,
            // Valid spans, but no start is known.
            automated_review: false,
            started_at_ms: null,
            pr_links: 2,
            inferred_pr_links: 0,
          },
          {
            ...context,
            session_id: 'unlinked',
            title: null,
            automated_review: false,
            started_at_ms: start,
            pr_links: 0,
            inferred_pr_links: 0,
          },
        ];
      }),
    ),
  );
  await loaded();
  const table = screen.getByRole('table', { name: 'Session lanes' });
  const headers = within(table)
    .getAllByRole('columnheader')
    .map((cell) => cell.textContent?.trim());
  // Host glyph, session, Compactions, PRs, started, activity axis, cost — no spans or first seen.
  expect(headers[1]).toBe('session');
  expect(headers[2]).toBe('Compactions');
  expect(headers[3]).toBe('PRs');
  expect(headers[4]).toBe('started');
  expect(headers[5]).toContain('Activity');
  expect(headers[5]).not.toContain('48h');
  expect(headers[6]).toBe('cost');
  expect(headers.join(' ')).not.toMatch(/first seen|spans$/);
  const [titled, untitled, unlinked, orphan] = laneRows();
  // A saved title names the row; the identity stays in the link's name.
  const link = within(titled).getByRole('link', { name: 'Open session Saved title, titled' });
  expect(link.textContent).toBe('Saved title');
  expect(link.getAttribute('href')).toBe('/sessions/titled?q=titled&host=claude&range=7d');
  expect(untitled.querySelector('.xt-lane-find')!.textContent).toBe('Session untitled');
  // Recorded links, inferred evidence marked and spoken; zero for an indexed
  // session with no link; unknown only when no context row exists.
  const prs = titled.querySelector('.xt-lane-prs')!;
  expect(prs.getAttribute('data-inferred')).toBe('true');
  expect(prs.querySelector('.xt-evidence')).toBeTruthy();
  expect(prs.textContent).toContain('3 recorded pull request links, 1 inferred');
  expect(untitled.querySelector('.xt-lane-prs')!.hasAttribute('data-inferred')).toBe(false);
  expect(prs.querySelector('svg.xt-lane-prs-icon')).toBeTruthy();
  expect(prs.hasAttribute('data-empty')).toBe(false);
  expect(unlinked.querySelector('.xt-lane-prs > span[aria-hidden]')!.textContent).toBe('0');
  expect(unlinked.querySelector('.xt-lane-prs svg.xt-lane-prs-icon')).toBeTruthy();
  expect(unlinked.querySelector('.xt-lane-prs')!.getAttribute('data-empty')).toBe('true');
  expect(unlinked.querySelector('.xt-lane-prs')!.textContent).toContain(
    'no recorded pull request link',
  );
  expect(orphan.querySelector('.xt-lane-prs')).toBeNull();
  // PRs, started and cost each say so.
  expect(
    within(orphan).getAllByText('Unmeasured: No indexed context was read for this session'),
  ).toHaveLength(3);
  // The recorded start in the report zone, with year and zone in its title,
  // however far before the axis it is.
  const started = titled.querySelector('time')!;
  expect(started.textContent).toBe('Jun 1, 08:30');
  expect(started.getAttribute('title')).toBe('Jun 1, 2026, 08:30 (UTC)');
  expect(started.getAttribute('dateTime')).toBe('2026-06-01T08:30:00.000Z');
  // Unknown start stays unknown, never the first span.
  expect(untitled.querySelector('time')).toBeNull();
  expect(
    within(untitled).getByText(
      'Unmeasured: No start is known for this session; its first active span is not its start',
    ),
  ).toBeTruthy();
  // The span count moved into the row's description.
  expect(titled.textContent).toContain('1 active span in the last 48 hours');
  expect(titled.textContent).toContain('started Jun 1, 2026, 08:30 (UTC)');
  expect(untitled.textContent).toContain('start unknown');
});

it('explains the PRs column from the keyboard as recorded links', async () => {
  mount(nativeSource(() => f1()));
  await loaded();
  const header = screen.getByRole('button', {
    name: 'Recorded pull request links, definition',
  });
  act(() => header.focus());
  const tip = await screen.findByRole('tooltip');
  expect(tip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
  expect(tip.textContent).toContain(
    'every evidence level (exact, commit, inferred), all indexed time, not merge status',
  );
  expect(tip.textContent).toContain('Zero means no link is recorded');
});

it('gives the lane cost column its own definition and window', async () => {
  mount(nativeSource(() => f1()));
  await loaded();
  const header = screen.getByRole('button', {
    name: 'Cost over the last 48 hours, definition',
  });
  expect(header.textContent).toBe('cost');
  // The definition is reachable by keyboard, not only by pointer.
  act(() => header.focus());
  const tip = await screen.findByRole('tooltip');
  expect(tip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
  expect(tip.textContent).toContain(
    "API-equivalent cost of every token each session used over the whole 48-hour window, including active spans this table does not draw, at each model's public rate.",
  );
  expect(tip.textContent).toContain('"+" means some responses have no published price');
  expect(tip.textContent).toContain('"—" means nothing could be priced');
  expect(tip.textContent).toContain(
    "Codex responses with no service tier recorded are priced at OpenAI's default (standard) tier.",
  );
});

it('marks a partial lane cost, an unpriced one, and keeps small and large amounts readable', async () => {
  const start = f1().lane_start_ms;
  const minute = 60_000;
  const ids = ['partial', 'unpriced', 'tiny', 'large', 'missing'];
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.lanes = ids.map((session_id, index) => ({
          session_id,
          host: 'codex',
          start_ms: start + (100 - index * 10) * minute,
          end_ms: start + (105 - index * 10) * minute,
        }));
        report.lanes_total = ids.length;
        const context = {
          title: null,
          automated_review: false,
          started_at_ms: null,
          pr_links: 0,
          inferred_pr_links: 0,
          host: 'codex',
          repo: null,
          branch: null,
        };
        report.lane_sessions = [
          {
            ...context,
            session_id: 'large',
            cost: laneCost(1_234.4, { assumed_tier_observations: 1 }),
          },
          { ...context, session_id: 'missing', cost: null },
          {
            ...context,
            session_id: 'partial',
            cost: laneCost(0, {
              total_usd: null,
              priced_subtotal_usd: 12.4,
              selected_observations: 5,
              priced_observations: 3,
              unpriced_observations: 2,
              unpriced: [
                {
                  model: 'codex-auto-review',
                  service_tier: null,
                  reason: 'unknown_model',
                  observations: 2,
                },
              ],
            }),
          },
          { ...context, session_id: 'tiny', cost: laneCost(0.004) },
          {
            ...context,
            session_id: 'unpriced',
            cost: laneCost(0, {
              total_usd: null,
              selected_observations: 4,
              priced_observations: 0,
              unpriced_observations: 4,
              unpriced: [
                {
                  model: 'codex-auto-review',
                  service_tier: null,
                  reason: 'unknown_model',
                  observations: 3,
                },
                { model: null, service_tier: null, reason: 'missing_model', observations: 1 },
              ],
            }),
          },
        ];
      }),
    ),
  );
  await loaded();
  const [partial, unpriced, tiny, large, missing] = laneRows();
  const costCell = (row: HTMLElement) =>
    [...row.querySelectorAll('[role="cell"]')].at(-1)!.querySelector('.xt-metric-cell')!;
  expect(costCell(partial).textContent).toBe('$12.40+');
  expect(costCell(partial).getAttribute('title')).toBe(
    'At least $12.40: 3 of 5 responses priced; 2 codex-auto-review have no published price; the unpriced ones are not included.',
  );
  expect(partial.textContent).toContain(
    'Over the whole 48 hours: at least $12.40 API-equivalent cost: 3 of 5 responses priced; 2 codex-auto-review have no published price.',
  );
  expect(
    within(unpriced).getByText(
      'Unmeasured: codex-auto-review has no published price; no model recorded',
    ),
  ).toBeTruthy();
  expect(unpriced.textContent).toContain(
    'cost unknown: codex-auto-review has no published price; no model recorded',
  );
  expect(costCell(tiny).textContent).toBe('<$0.01');
  expect(costCell(large).textContent).toBe('$1,234');
  expect(costCell(large).getAttribute('title')).toBe(
    "$1,234.40 API-equivalent, 1 response priced. 1 Codex response recorded no service tier and is priced at OpenAI's default (standard) tier.",
  );
  expect(
    within(missing).getByText(
      'Unmeasured: No indexed session owns this identifier, so nothing was priced',
    ),
  ).toBeTruthy();
});

it('keeps a truncated lane row’s cost covering the whole window', async () => {
  const start = f1().lane_start_ms;
  mount(
    nativeSource(() =>
      synthetic((report) => {
        // One drawn span of a session whose earlier spans the cap left out.
        report.lanes = [
          {
            session_id: 'capped',
            host: 'claude',
            start_ms: start + 2_820 * 60_000,
            end_ms: start + 2_830 * 60_000,
          },
        ];
        report.lanes_total = 205;
        report.lanes_truncated = true;
        report.lane_sessions = [
          {
            session_id: 'capped',
            title: null,
            automated_review: false,
            started_at_ms: null,
            pr_links: 0,
            inferred_pr_links: 0,
            host: 'claude',
            repo: '/Users/dev/code/acme-api',
            branch: null,
            cost: laneCost(42),
          },
        ];
      }),
    ),
  );
  await loaded();
  const [row] = laneRows();
  expect(row.textContent).toContain('1 active span');
  expect(within(row).getByText('$42.00')).toBeTruthy();
});

it('keeps a lane honest when the report carries no context row for it', async () => {
  const start = f1().lane_start_ms;
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.lanes = [
          { session_id: 'orphan', host: 'claude', start_ms: start, end_ms: start + 60_000 },
        ];
        report.lanes_total = 1;
        report.lane_sessions = [
          {
            session_id: 'other',
            title: null,
            automated_review: false,
            started_at_ms: null,
            pr_links: 0,
            inferred_pr_links: 0,
            host: 'claude',
            repo: '/Users/dev/code/acme-api',
            branch: 'main',
            cost: laneCost(9),
          },
        ];
      }),
    ),
  );
  await loaded();
  const [row] = laneRows();
  // Another session's repository is never borrowed for this row.
  expect(row.textContent).not.toContain('acme-api');
  expect(row.querySelector('.xt-lane-repo')!.textContent).toBe('Unknown repository');
  expect(row.querySelector('.xt-lane-find')!.textContent).toBe('Session orphan');
  expect(
    within(row).getAllByText('Unmeasured: No indexed context was read for this session'),
  ).toHaveLength(3);
  expect(row.textContent).toContain('cost unknown: no indexed context');
});

it('captions token measurement with usage coverage sessions, including ones without usage', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        // The token report only knows the one session with selected usage.
        report.tokens = { ...report.tokens, sessions: 1, measured_sessions: 1 };
        const usage = { sessions: 10, measured: 1, pct: 10, gaps: ['no_selected_usage' as const] };
        report.usage_coverage = { total: usage, by_host: [], by_surface: [] };
      }),
    ),
  );
  await loaded();
  const tokensDialog = await detail('Tokens per day');
  expect(tokensDialog.textContent).toContain('1.1K recorded · 1/10 sessions measured');
  expect(tokensDialog.textContent).not.toContain('1/1 session measured');
});

it('labels a measured peak, a measured zero peak and a series without any measurement', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.days = report.days.map((day, index) => ({
          ...day,
          // Output measured as zero every day; fresh input unmeasured on usage days only.
          tokens:
            index < 2
              ? tokens({ output_tokens: 0, cache_read_tokens: index === 0 ? 40 : 0 })
              : tokens({}, 0),
        }));
      }),
    ),
  );
  await loaded();
  const chart = await detail('Tokens per day');
  const label = (series: string) =>
    [...chart.querySelectorAll('.xt-token-series-label')]
      .find((item) => item.textContent?.startsWith(series))!
      .querySelector('small')!.textContent;
  expect(label('Fresh input')).toBe('no recorded measurement');
  expect(label('Output')).toBe('measured peak 0');
  expect(label('Cache read')).toBe('measured peak 40');
  expect(chart.textContent).not.toContain('peak —');
});

it('follows the design composition and keeps supplementary measurements in collapsed details', async () => {
  mount(nativeSource((days) => f1(days)));
  await loaded();
  const order = [...document.querySelectorAll('.xt-dashboard h2')].map((h) => h.textContent);
  expect(order).toEqual(['Effort', 'Environment', 'Sessions']);
  expectHiddenPanels();
  // The tiles follow the summary directly: no row is kept for the hidden panels.
  const tiles = document.querySelector('.xt-dash-tiles')!;
  expect(tiles.previousElementSibling).toBe(screen.getByTestId('dashboard-summary'));
  expect(document.querySelector('.xt-dash-hero-row')).toBeNull();
  const main = card('Effort').parentElement!;
  expect(main.className).toContain('xt-dash-main-row');
  expect(tiles.nextElementSibling).toBe(main);
  expect(main.nextElementSibling).toBe(card('Sessions'));
  // Sessions is the last block: no measurement line follows it. Each
  // supplementary measurement opens over the page from the card it belongs
  // to; none is open.
  expect(card('Sessions').nextElementSibling).toBeNull();
  expect(screen.queryByRole('group', { name: 'Measurement details' })).toBeNull();
  expect(screen.queryByRole('dialog')).toBeNull();
  // Nothing on the page expands in place: no disclosure sits in the page flow.
  expect(document.querySelectorAll('.xt-dashboard details')).toHaveLength(0);
  for (const title of ['Tokens per day', 'API-equivalent cost', 'Coverage']) {
    const section = await detail(title);
    const level = title === 'Coverage' ? 2 : 3;
    expect(within(section).getByRole('heading', { level, name: title })).toBeTruthy();
    expect(section.textContent!.length).toBeGreaterThan(title.length);
    // Each keeps its definition beside its title.
    expect(within(section).getByRole('button', { name: `${title} definition` })).toBeTruthy();
    await closeDialog(section);
    expect(detailTrigger(title)).toBe(document.activeElement);
  }
  // Method's own daily values come first; the usage sections follow them.
  fireEvent.click(detailTrigger('Tokens per day'));
  const method = await screen.findByRole('dialog', { name: METHOD });
  expect([...method.querySelectorAll('h2, h3')].map((heading) => heading.textContent)).toEqual([
    METHOD,
    'Tokens per day',
    'API-equivalent cost',
  ]);
  await closeDialog(method);
  // The hands-off definition leads with the design copy, then the real rule context.
  fireEvent.focus(tileNamed('Hands-off median'));
  const tip = await screen.findByRole('tooltip');
  expect(tip.textContent).toMatch(
    /How long your agents run before they need you\. .*No surface is excluded/,
  );
});

it('hides the TopBar period off windowed routes and while a range is loading', async () => {
  let release!: () => void;
  const gate = new Promise<void>((done) => (release = done));
  const source = nativeSource(async (days) => {
    if (days === 30) await gate;
    return f1(days);
  });
  mount(source);
  await screen.findByTestId('report-period');
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(screen.queryByTestId('report-period')).toBeNull());
  act(() => release());
  expect((await screen.findByTestId('report-period')).textContent).toMatch(
    /^Aug 9\s–\sSep 7, 2026/,
  );
  // Sessions reports the same window, so the period travels with it; a route
  // that measures no window shows none.
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(screen.getByTestId('report-period').textContent).toMatch(/^Aug 9\s–\sSep 7, 2026/);
  fireEvent.click(screen.getByRole('button', { name: 'Rulebook' }));
  expect(screen.queryByTestId('report-period')).toBeNull();
});

/**
 * Every continuous measurement on the page set to one value: the two means,
 * and the hands-off median with its p90 aside. Each tile keeps its own rule so
 * the definitions it offers are the real ones.
 */
const continuousTiles = (value: number | null) => (report: DashboardMetrics) => {
  for (const key of [
    'agent_hours_per_day',
    'concurrency_mean',
    'hands_off_median',
    'hands_off_p90',
  ] as const)
    report.tiles[key] = tile(value, { rule_id: report.tiles[key].rule_id });
};
const continuousText = async (value: number | null) => {
  mount(nativeSource(() => synthetic(continuousTiles(value))));
  await loaded();
  return {
    perDay: tileNamed('Agent h/day').textContent,
    concurrency: tileNamed('Concurrency').textContent,
    handsOff: tileNamed('Hands-off median').textContent,
  };
};

it('reads a small positive Dashboard measurement as below the scale, not as a measured zero', async () => {
  // 0.03 hours, h/day, lanes and minutes are all real work that the shared
  // one-decimal scale would otherwise print as the 0 this page keeps for a
  // measured zero — including the hands-off p90 aside.
  expect(await continuousText(0.03)).toEqual({
    perDay: 'Agent h/day<0.1h',
    concurrency: 'Concurrency<0.1max 1',
    handsOff: 'Hands-off median<0.1minp90 <0.1',
  });
  cleanup();
  // A measured zero keeps saying zero: it is the one thing "0" means here.
  expect(await continuousText(0)).toEqual({
    perDay: 'Agent h/day0h',
    concurrency: 'Concurrency0max 1',
    handsOff: 'Hands-off median0minp90 0',
  });
  cleanup();
  // A value the scale can show is shown by the scale, unchanged.
  expect(await continuousText(1.24)).toEqual({
    perDay: 'Agent h/day1.2h',
    concurrency: 'Concurrency1.2max 1',
    handsOff: 'Hands-off median1.2minp90 1.2',
  });
  cleanup();
  // An unmeasured value never reaches a formatter: it still states its reason,
  // and the p90 aside is dropped rather than made up.
  const unknown = await continuousText(null);
  for (const text of Object.values(unknown)) {
    expect(text).toContain('Unmeasured: Synthetic unknown reason');
    expect(text).not.toContain('<0.1');
  }
  expect(unknown.handsOff).not.toContain('p90');
});

it('draws a below-scale value in the same cell at the same size, in light and in dark', async () => {
  // jsdom lays nothing out, so this holds the two things the page itself
  // decides about the string's width: the cell it is drawn in, which clips
  // rather than wraps, and that cell's size. Both must match an ordinary
  // value's and must not differ between themes, so the one extra character of
  // `<0.1` cannot displace the unit or the aside beside it.
  const seen = [];
  for (const [theme, value] of [
    ['dark', 1.24],
    ['dark', 0.03],
    ['light', 0.03],
  ] as const) {
    localStorage.setItem('xt.theme', theme);
    mount(nativeSource(() => synthetic(continuousTiles(value))));
    await loaded();
    expect(document.documentElement.dataset.theme).toBe(theme);
    const cells = ['Agent h/day', 'Concurrency', 'Hands-off median'].map((label) =>
      tileNamed(label).querySelector<HTMLElement>('.xt-metric-cell')!,
    );
    seen.push({
      handsOff: tileNamed('Hands-off median').textContent,
      cells: cells.map(
        (cell) => `${cell.className} ${cell.style.fontSize} ${cell.style.textAlign}`,
      ),
    });
    cleanup();
  }
  const [ordinary, dark, light] = seen;
  expect(dark).toEqual(light);
  expect(dark.cells).toEqual(ordinary.cells);
  expect(ordinary.handsOff).toBe('Hands-off median1.2minp90 1.2');
  expect(dark.handsOff).toBe('Hands-off median<0.1minp90 <0.1');
});

it('shows the generated report at 14 days as below the scale rather than as zero', async () => {
  // The bug in the generated data itself: F1's 0.38 agent hours spread over 14
  // days is 0.027 per day, which the scale alone printed as "0" for a range
  // that recorded 0.4 hours of agent time.
  mount(nativeSource((days) => f1(days)));
  await loaded();
  expect(tileNamed('Agent h/day').textContent).toContain('0.1h');
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(tileNamed('Agent h/day').textContent).toContain('<0.1h'));
});

it('keeps Agent / human hours and Caught by your rules hidden at every range preset', async () => {
  mount(nativeSource((days) => f1(days)));
  await loaded();
  for (const preset of ['14d', '30d', '7d'] as const) {
    fireEvent.click(screen.getByRole('radio', { name: preset }));
    await waitFor(() => expect(period()).toMatch(PERIOD[preset]));
    await loaded();
    expectHiddenPanels();
    const order = [...document.querySelectorAll('.xt-dashboard h2')].map((h) => h.textContent);
    expect(order).toEqual(['Effort', 'Environment', 'Sessions']);
  }
});

it('discloses untimed indexed history with coverage, whatever range is selected', async () => {
  // F1 indexes nothing without a timestamp, so the Dashboard says nothing.
  mount(nativeSource((days) => f1(days)));
  await loaded();
  let coverage = await detail('Coverage');
  expect(within(coverage).queryByTestId('coverage-untimed')).toBeNull();
  expect(coverage.textContent).not.toContain('Untimed history');
  await closeDialog(coverage);
  cleanup();
  const disclosed = (report: DashboardMetrics) => {
    report.untimed_history = {
      records: 9,
      by_surface: [
        { host: 'claude', surface: 'cli', records: 6 },
        { host: 'cursor', surface: null, records: 3 },
      ],
    };
  };
  mount(nativeSource((days) => synthetic(disclosed, days)));
  await loaded();
  // The count is heard from the Sessions header's Coverage without opening it;
  // the explanation and the breakdown are in the dialog it opens.
  expect(detailTrigger('Coverage').getAttribute('aria-label')).toMatch(/9 untimed records$/);
  expect(screen.queryByRole('dialog')).toBeNull();
  coverage = await detail('Coverage');
  const untimed = within(coverage).getByTestId('coverage-untimed');
  expect(within(untimed).getByRole('heading', { level: 3, name: 'Untimed history' })).toBeTruthy();
  expect(within(untimed).getByTestId('untimed-summary').textContent).toBe(
    '9 records · outside dated measurements',
  );
  expect(untimed.textContent).toContain(
    'Some indexed history has no timestamps and cannot contribute to date-based measurements.',
  );
  await closeDialog(coverage);
  // The count is of all indexed history, so changing the range cannot move it,
  // and none of the range's own measurements change with it either.
  const summary = screen.getByTestId('dashboard-summary').textContent;
  for (const preset of ['14d', '30d'] as const) {
    fireEvent.click(screen.getByRole('radio', { name: preset }));
    await waitFor(() => expect(period()).toMatch(PERIOD[preset]));
    expect(detailTrigger('Coverage').getAttribute('aria-label')).toMatch(/9 untimed records$/);
    coverage = await detail('Coverage');
    expect(within(coverage).getByTestId('untimed-count').textContent).toContain(
      '9 records in all indexed history',
    );
    expect(coverage.textContent).toContain('cursor · unknown surface');
    await closeDialog(coverage);
  }
  expect(screen.getByTestId('dashboard-summary').textContent).toBe(summary);
  // Not a banner over the dated cards, and not a line of the page.
  expect(document.querySelector('.xt-dashboard .xt-untimed')).toBeNull();
});

it('keeps an open Method or Coverage open while another range loads', async () => {
  mount(nativeSource((days) => synthetic(() => {}, days)));
  await loaded();
  for (const [title, name] of [
    ['Tokens per day', METHOD],
    ['Coverage', 'Coverage'],
  ] as const) {
    fireEvent.click(detailTrigger(title));
    await screen.findByRole('dialog', { name });
    // The page under the dialog is inert to assistive technology, so the range
    // changes as it would from elsewhere; the detail is the page's, not its trigger's.
    fireEvent.click(
      screen.getByRole('radio', { name: title === 'Coverage' ? '30d' : '14d', hidden: true }),
    );
    await waitFor(() => expect(period()).toMatch(PERIOD[title === 'Coverage' ? '30d' : '14d']));
    const dialog = await screen.findByRole('dialog', { name });
    await closeDialog(dialog);
    fireEvent.click(screen.getByRole('radio', { name: '7d' }));
    await waitFor(() => expect(period()).toMatch(PERIOD['7d']));
  }
});

it('explains each card from a compact info control, with no rule ID in the chrome', async () => {
  mount(nativeSource((days) => f1(days)));
  await loaded();
  // No metric ID is drawn as a chip or as visible text anywhere on the page.
  expect(document.querySelector('.xt-rule-chip')).toBeNull();
  const visible = [...document.querySelectorAll('.xt-dashboard *')]
    .filter((node) => !node.closest('[role="tooltip"], .sr-only'))
    .flatMap((node) => [...node.childNodes])
    .filter((node) => node.nodeType === Node.TEXT_NODE)
    .map((node) => node.textContent ?? '')
    .join(' ');
  expect(visible).toContain('Effort');
  expect(visible).not.toMatch(/\b[MRC]-\d{2}\b/);
  const namedHelp = [
    ...document.querySelectorAll('.xt-dashboard [aria-label], .xt-dashboard [title]'),
  ]
    .flatMap((node) => [node.getAttribute('aria-label'), node.getAttribute('title')])
    .filter(Boolean)
    .join(' ');
  expect(namedHelp).not.toMatch(/\b[MRC]-\d{2}\b/);
  // Each card keeps its definition one tab stop away; a
  // measurement on the line keeps its definition beside its dialog's title.
  const definition = async (scope: HTMLElement, title: string, expected: string) => {
    const info = within(scope).getByRole('button', { name: `${title} definition` });
    expect(info.textContent).toBe('');
    fireEvent.focus(info);
    const tip = await screen.findByRole('tooltip');
    expect(tip.textContent).toContain(expected);
    expect(tip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
    expect(info.getAttribute('aria-describedby')).toBe(tip.id);
    fireEvent.blur(info);
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  };
  for (const [title, expected] of [
    ['Effort', 'distinct session IDs'],
    ['Environment', 'Environment usage'],
    ['Sessions', 'Active spans'],
  ] as const)
    await definition(card(title), title, expected);
  for (const [title, expected] of [
    ['Tokens per day', 'Tokens and cost'],
    ['API-equivalent cost', 'Tokens and cost'],
    ['Coverage', 'Surface capture coverage'],
  ] as const) {
    const dialog = await detail(title);
    await definition(dialog, title, expected);
    await closeDialog(dialog);
  }
});
