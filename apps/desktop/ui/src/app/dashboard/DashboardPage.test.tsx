import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { MetricTile } from '../../data/generated/MetricTile';
import type { MetricTokenSummary } from '../../data/generated/MetricTokenSummary';
import { events, type DataEvent } from '../../data/ipc-names';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const f1 = (days = 7) => structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

type Edit = (report: DashboardMetrics) => void;
/** Synthetic reports: the generated F1 export with the named fields replaced. */
const synthetic = (edit: Edit, days = 7) => {
  const report = f1(days);
  edit(report);
  return report;
};
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
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
    sessionsList: async () => ({ rows: [], next: null }),
    dashboard: vi.fn(async (days: number) => report(days)),
    tokensByHost: vi.fn(async (days: number) => {
      const current = await report(days);
      return { window: current.window, hosts: current.tokens_by_host };
    }),
    environment: vi.fn(async (days: number) => {
      const report = exported.environments.find((entry) => entry.window.days === days);
      if (!report) throw new Error('Metric range must be 7, 14, or 30 days.');
      return report;
    }),
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

it('renders the generated F1 report with honest unknowns and the shared sidebar totals', async () => {
  const source = nativeSource((days) => f1(days));
  mount(source);
  await loaded();
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('What your agents did');
  // The report period sits in the TopBar, immediately before the range presets.
  const period = await screen.findByTestId('report-period');
  expect(period.textContent).toMatch(/^Sep 1\s–\s7, 2026, time zone UTC$/);
  expect(period.getAttribute('title')).toMatch(/Sep 1\s–\s7, 2026 · UTC/);
  expect(period.closest('.xt-topbar-tools')!.contains(screen.getByRole('radiogroup'))).toBe(true);
  const summary = screen.getByTestId('dashboard-summary').textContent;
  expect(summary).not.toMatch(/2026|UTC/);
  expect(summary).toContain('1 session');
  expect(summary).toContain('5 human messages');
  expect(summary).toContain('5 tool calls');
  expect(summary).toContain('fixture-model-v1');
  expect(source.dashboard).toHaveBeenCalledWith(7);
  expect(source.tokensByHost).toHaveBeenCalledWith(7);
  // Hero: agent/human hours and ratio; human time is marked as an estimate.
  const heroCard = card('Agent / human hours');
  const hero = within(heroCard);
  expect(hero.getByText('Agent hours', { exact: false }).parentElement!.textContent).toBe(
    'Agent hours 0.4',
  );
  expect(
    hero.getByText('Human-in-the-loop hours, estimated', { exact: false }).parentElement!
      .textContent,
  ).toBe('Human-in-the-loop hours, estimated 0.1');
  expect(hero.getByText('human est.')).toBeTruthy();
  expect(hero.getByText('Agent to human ratio', { exact: false }).parentElement!.textContent).toBe(
    'Agent to human ratio 2.8× agent : human',
  );
  // Suppressed comparisons are omitted, not replaced by placeholders or long prose.
  expect(heroCard.textContent).not.toMatch(/no comparison|vs previous|estimated from message/);
  // Tiles keep full labels; merged PRs is unknown with its reason, never zero.
  expect(tileNamed('Agent h/day').textContent).toContain('0.1h');
  expect(tileNamed('Concurrency').textContent).toContain('max 1');
  expect(tileNamed('Merged PRs').textContent).toContain(
    'Unmeasured: Merged PR data is unavailable',
  );
  expect(tileNamed('Hands-off median').textContent).toContain('p90 3');
  // Named unavailable sections keep their reasons.
  for (const [title, reason] of [
    ['Caught by your rules', 'Rule-fire data is unavailable'],
    ['Effort by type', 'Work-type data is unavailable'],
  ])
    expect(within(card(title)).getAllByText(new RegExp(reason)).length).toBeGreaterThan(0);
  // Environment reads its own observed usage instead of the report's unavailable entry.
  expect(
    (await within(card('Environment')).findByTestId('environment-summary')).textContent,
  ).toContain('5 observed calls · last 7d · 1 identity in either window');
  expect(card('Environment').textContent).not.toContain('Environment data is unavailable');
  // F1 is legitimately unpriced: no total, no subtotal masquerading as one.
  const cost = within(card('API-equivalent cost'));
  expect(screen.getByTestId('cost-total').textContent).toContain('Unmeasured');
  expect(screen.getByTestId('cost-subtotal').textContent).toBe(
    'None of 10 selected responses could be priced.',
  );
  expect(cost.getByText('service tier not recorded · 10 responses')).toBeTruthy();
  expect(card('API-equivalent cost').textContent).toContain(
    'catalog synthetic-v1 · as of 2026-09-17',
  );
  expect(cost.getByText(/Global public token API-equivalent/)).toBeTruthy();
  // Coverage: 100 percentage points display as 100%, and unknown inventory is not green.
  expect(screen.getByTestId('usage-coverage').textContent).toContain('100% of sessions (1 of 1');
  // No coverage rows cannot prove that no receipts exist; inventory stays unknown.
  expect(
    within(card('Coverage')).getByText('No session capture coverage rows available.'),
  ).toBeTruthy();
  expect(card('Coverage').textContent).not.toContain('No capture receipts');
  expect(within(card('Coverage')).getByText(/Receipts are historical/)).toBeTruthy();
  const pill = within(card('Coverage')).getByText('Inventory unknown');
  expect(pill.closest('[data-tone]')!.getAttribute('data-tone')).toBe('meta');
  expect(screen.getByTestId('dashboard-index').textContent).toContain('Disabled');
  // Sidebar: recorded tokens for the selected range, never a quota.
  expect(await screen.findByLabelText('Claude Code tokens: 1100')).toBeTruthy();
  expect(screen.getByText('Recorded tokens · 7d')).toBeTruthy();
  expect(screen.getByRole('link', { name: 'View sessions' }).getAttribute('href')).toBe(
    '/sessions',
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
  expect(tileNamed('Concurrency').textContent).toContain('1.8mean');
  expect(tileNamed('Concurrency').textContent).toContain('max 3');
  const rows = within(screen.getByRole('region', { name: 'Session lanes' })).getAllByRole(
    'listitem',
  );
  expect(rows).toHaveLength(3);
  const spans = rows.map((row) => row.querySelector<HTMLElement>('.xt-lane-span')!.style);
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
  expect(rows[0].textContent).toContain('claude session f2-session-c: active');
  expect(screen.getByTestId('lanes-disclosure').textContent).toBe(
    '3 active spans in the last 48 hours, whatever range is selected. A session can have several spans.',
  );
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
  expect(card('Tokens per day').textContent).toContain('total unmeasured · 0/1 session measured');
  expect(screen.getAllByRole('img', { name: /output tokens: no recorded usage/ })).toHaveLength(7);
  expect(screen.getByTestId('cost-total').textContent).toContain(
    'Unmeasured: Selected usage is absent or unpriced',
  );
  expect(screen.getByTestId('cost-subtotal').textContent).toBe(
    'No selected usage to price in this range.',
  );
  // Measured 0 percentage points is a real 0%, with the gap named.
  expect(screen.getByTestId('usage-coverage').textContent).toContain(
    '0% of sessions (0 of 1 session) · no recorded usage',
  );
  const surfaces = within(screen.getByRole('list', { name: 'Token measurement by surface' }));
  expect(surfaces.getByText('cursor · cli')).toBeTruthy();
  // Cursor is not a quota: an unmeasured host total shows a dash.
  expect(await screen.findByLabelText('Cursor tokens: unmeasured')).toBeTruthy();
});

it('converts F11 percentage-point deltas exactly once and hides suppressed ones', async () => {
  mount(
    nativeSource(() =>
      synthetic((report) => {
        report.tiles.agent_hours = tile(3, { delta: { previous: 2, pct: 50, suppressed: false } });
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
  expect(within(card('Agent / human hours')).getByText('agent ▲50% vs previous')).toBeTruthy();
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
      }),
    ),
  );
  await loaded();
  expect(screen.getByTestId('cost-total').textContent).not.toContain('$');
  expect(screen.getByTestId('cost-subtotal').textContent).toBe(
    'Partial subtotal $0.42 covers only 1 of 3 selected responses; it is not a total.',
  );
  expect(
    within(card('API-equivalent cost')).getByText('service tier not recorded · 2 responses'),
  ).toBeTruthy();
  expect(screen.getByTestId('usage-coverage').textContent).toContain(
    '66.7% of sessions (2 of 3 sessions) · incomplete token counters',
  );
});

it('shows a complete total, measured zeros, unknown ratio and hands-off exclusions', async () => {
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
        report.tiles.agent_hours = tile(0);
        report.tiles.ratio = tile(null, { rule_id: 'M-08', reason: 'No human-in-the-loop time' });
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
  expect(screen.getByTestId('cost-total').textContent).toBe('Total$1.23');
  expect(screen.queryByTestId('cost-subtotal')).toBeNull();
  expect(screen.getByTestId('dashboard-summary').textContent).toContain('0 sessions');
  expect(screen.getByText(/No agent activity was recorded in this range/)).toBeTruthy();
  const hero = card('Agent / human hours');
  expect(hero.textContent).toContain('Agent hours 0/');
  expect(hero.textContent).toContain('Unmeasured: No human-in-the-loop time');
  expect(tileNamed('Hands-off median').textContent).toContain(
    'Unmeasured: No hands-off stretches after exclusions',
  );
  expect(
    within(screen.getByRole('list', { name: 'Surfaces excluded from hands-off' })).getByText(
      'claude · batch: 2 of 4 qualifying sessions have batch-stamped timestamps',
    ),
  ).toBeTruthy();
  const pill = within(card('Coverage')).getByText('Inventory complete');
  expect(pill.closest('[data-tone]')!.getAttribute('data-tone')).toBe('success');
  // Measured zero, unmeasured and no-usage days are distinguishable per series.
  const output = within(screen.getByRole('group', { name: 'Output tokens per day' }));
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
  expect(screen.getByTestId('lanes-disclosure').textContent).toBe(
    'Showing the 200 most recent active spans of 205 in the last 48 hours. Metrics still include every span.',
  );
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() =>
    expect(
      within(screen.getByRole('group', { name: 'Fresh input tokens per day' })).getAllByRole('img'),
    ).toHaveLength(30),
  );
  expect(screen.getByRole('button', { name: 'Custom range' }).hasAttribute('disabled')).toBe(true);
});

it('moves the Dashboard and sidebar to the same selected range', async () => {
  const source = nativeSource((days) => f1(days));
  mount(source);
  await loaded();
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() =>
    expect(screen.getByTestId('report-period').textContent).toMatch(/^Aug 25\s–\sSep 7, 2026/),
  );
  expect(source.dashboard).toHaveBeenLastCalledWith(14);
  expect(source.tokensByHost).toHaveBeenLastCalledWith(14);
  expect(screen.getByText('Recorded tokens · 14d')).toBeTruthy();
  // Other routes keep the selection without a range control of their own.
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(screen.queryByRole('radiogroup', { name: 'Date range' })).toBeNull();
  expect(screen.getByText('Recorded tokens · 14d')).toBeTruthy();
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
  expect(await screen.findByLabelText('Claude Code tokens: 1100')).toBeTruthy();
  state = 'enriched';
  act(() => source.emit(events.nativeIndexStatus));
  expect(
    await screen.findByLabelText('Claude Code tokens: 1250', {}, { timeout: 3000 }),
  ).toBeTruthy();
  await waitFor(() => expect(card('Tokens per day').textContent).toContain('1.25K recorded'));
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

it('keeps one lane row per returned span, including several spans of one session', async () => {
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
  const rows = within(screen.getByRole('region', { name: 'Session lanes' })).getAllByRole(
    'listitem',
  );
  expect(rows).toHaveLength(3);
  expect(
    rows.filter((row) => row.textContent?.includes('claude session split-session')),
  ).toHaveLength(2);
  expect(screen.getByTestId('lanes-disclosure').textContent).toBe(
    '3 active spans in the last 48 hours, whatever range is selected. A session can have several spans.',
  );
  expect(errors.mock.calls.some((call) => String(call[0]).includes('same key'))).toBe(false);
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
  expect(card('Tokens per day').textContent).toContain('1.1K recorded · 1/10 sessions measured');
  expect(card('Tokens per day').textContent).not.toContain('1/1 session measured');
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
  const chart = card('Tokens per day');
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
  expect(order).toEqual([
    'Agent / human hours',
    'Caught by your rules',
    'Effort by type',
    'Environment',
    'Sessions',
    'Tokens per day',
    'API-equivalent cost',
    'Coverage',
  ]);
  const tiles = document.querySelector('.xt-dash-tiles')!;
  const main = card('Effort by type').parentElement!;
  expect(main.className).toContain('xt-dash-main-row');
  expect(tiles.nextElementSibling).toBe(main);
  expect(main.nextElementSibling).toBe(card('Sessions'));
  for (const title of ['Tokens per day', 'API-equivalent cost', 'Coverage']) {
    const details = card(title).querySelector('details')!;
    expect(details.open).toBe(false);
    // Real content is still rendered inside, only collapsed.
    expect(details.querySelector('.xt-dash-extra-body')!.textContent!.length).toBeGreaterThan(0);
  }
  expect(card('Coverage').textContent).toContain(
    'Inventory unknown · tokens measured for 1/1 session',
  );
  // Diagnostics are a compact line with a link, not a panel.
  expect(screen.queryByRole('heading', { name: 'Native index' })).toBeNull();
  expect(
    within(screen.getByTestId('dashboard-index')).getByRole('link', { name: 'Index details' }),
  ).toBeTruthy();
  // The hands-off definition leads with the design copy, then the real rule context.
  fireEvent.focus(tileNamed('Hands-off median'));
  const tip = await screen.findByRole('tooltip');
  expect(tip.textContent).toMatch(
    /How long your agents run before they need you\. .*No surface is excluded/,
  );
});

it('hides the TopBar period off the Dashboard and while a range is loading', async () => {
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
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(screen.queryByTestId('report-period')).toBeNull();
});
