import { expect, test, type Locator, type Page } from '@playwright/test';
import { writeFile } from 'node:fs/promises';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import {
  shapedEnvironment,
  syntheticEnvironment,
} from '../src/app/dashboard/environment.synthetic';
import { withPrEffort, type SectionSpec } from '../src/app/dashboard/pr-effort.synthetic';
import type { DashboardMetrics } from '../src/data/generated/DashboardMetrics';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { MetricTile } from '../src/data/generated/MetricTile';
import type { NativeHostState } from '../src/data/generated/NativeHostState';
import type { NativeIndexStatus } from '../src/data/generated/NativeIndexStatus';
import { freshnessText, phaseText } from '../src/app/native-index-text';

/**
 * The Dashboard fits the window: nothing primary is below the fold, and no
 * document, outlet or page scroll exists to reach anything. Every primary
 * panel and control sits inside the viewport, the two flexible rows show at
 * least three rows each, the four tiles share one row, and every secondary
 * detail opens over the page without moving it. Effort and
 * Environment are layered as Sessions is — the title row on the canvas, the
 * body in the kit's inset surface panel — while the tiles stay flat. Agent /
 * human hours and Caught by your rules are not drawn, and no row is kept for
 * them.
 *
 * Measured at the native minimum (1120×720), a middle size (1280×800) and the
 * default (1440×900), in both schemes, over the generated F1 export and over
 * synthetic data (not real history, not a design sample) that makes every
 * part of the page as long and as full as it gets at once: dense tiles with
 * comparisons, a long favorite model, a week of effort from eight assignments with a partly priced day, eleven environment identities with unresolved and
 * untimed observations, fourteen truncated lane sessions with long
 * repositories, four-figure partial costs, a priced total with unpriced tiers,
 * partial coverage rows, hands-off exclusions and untimed history.
 */
const SIZES = [
  [1120, 720],
  [1280, 800],
  [1440, 900],
] as const;
const SCHEMES = ['light', 'dark'] as const;
const base = fixture as FixtureExport;

const measured = (tile: MetricTile, value: number, pct: number | null): MetricTile => ({
  ...tile,
  value,
  reason: null,
  current_n: 12,
  previous_n: 10,
  delta: { previous: value, pct, suppressed: pct === null },
});

/** Eight assignments, one unresolved, on five days, beside five merge markers on three days. */
const STRESS_EFFORT: SectionSpec = {
  tile: { known_merged: 5, unknown_facts: 1 },
  markers: [
    { number: 1, day: 1, workType: 'docs' },
    { number: 2, day: 2, workType: 'perf' },
    { number: 3, day: 2, workType: 'refactor' },
    { number: 4, day: 3, workType: 'feat', confidence: 'sha' },
    {
      number: 5,
      day: 3,
      workType: 'fix',
      freshness: { state: 'failed_after_refresh', error: 'rate_limited' },
    },
  ],
  groups: [
    ...(
      [
        ['docs', 3],
        ['perf', 2],
        ['refactor', 4],
        ['feat', 12],
        ['fix', 7],
      ] as const
    ).map(([type, sessions], index) => ({
      assignment: { kind: 'type' as const, work_type: type },
      sessions,
      days: { [index + 1]: { agentMs: 3_600_000 * (index + 1), usd: 12.5 * (index + 1) } },
    })),
    { assignment: { kind: 'mixed' }, sessions: 2, days: { 3: { agentMs: 1_800_000, usd: 0.2 } } },
    { assignment: { kind: 'other' }, sessions: 9, days: { 4: { agentMs: 7_200_000, usd: 0.2 } } },
    {
      assignment: { kind: 'unresolved' },
      sessions: 1,
      days: { 5: { agentMs: 1_800_000, usd: 0.6, selected: 2, priced: 1 } },
    },
  ],
};

const agentDays = [9.8, 0, 3.1, 4.6, 0.4, 0.02, 0, 0, 1.6, 0.5, 0.7, 6.9, 6.3, 5.7];
const humanDays = [5.2, 0, 2.1, 3.0, 0.4, 0.01, 0, 0, 1.3, 0.5, 0.6, 4.1, 3.9, 3.5];

function stressReport(report: DashboardMetrics): DashboardMetrics {
  const out = withPrEffort(report, STRESS_EFFORT);
  const { tiles } = out;
  tiles.sessions = measured(tiles.sessions, 1234, 8.5);
  tiles.human_messages = measured(tiles.human_messages, 34_567, 1.2);
  tiles.tool_calls = measured(tiles.tool_calls, 123_456, -3.4);
  tiles.agent_hours = measured(tiles.agent_hours, 123.4, 12.3);
  tiles.human_hours_est = measured(tiles.human_hours_est, 45.6, -5.2);
  tiles.ratio = measured(tiles.ratio, 2.7, null);
  tiles.agent_hours_per_day = measured(tiles.agent_hours_per_day, 12.4, 8.5);
  tiles.concurrency_mean = measured(tiles.concurrency_mean, 1.8, -12.3);
  tiles.concurrency_max = measured(tiles.concurrency_max, 8, null);
  tiles.hands_off_median = measured(tiles.hands_off_median, 3.2, -4.1);
  tiles.hands_off_p90 = measured(tiles.hands_off_p90, 14.8, null);
  out.favorite.current = {
    ...out.favorite.current,
    model: 'claude-sonnet-4-5-20250929-extended-thinking-preview',
    unknown_reason: null,
  };
  out.favorite.previous = { ...out.favorite.previous, model: 'claude-opus-4-1-20250805' };
  out.days = out.days.map((day, index) => ({
    ...day,
    agent_hours: agentDays[index % agentDays.length],
    human_hours_est: humanDays[index % humanDays.length],
  }));
  out.tokens.counters.total_tokens = 123_456_789;
  out.cost = {
    ...out.cost,
    total_usd: 1234.56,
    priced_subtotal_usd: 1234.56,
    selected_observations: 1000,
    priced_observations: 900,
    unpriced_observations: 100,
    unpriced: [
      {
        model: 'synthetic-model',
        service_tier: null,
        reason: 'missing_service_tier',
        observations: 60,
      },
      { model: null, service_tier: 'priority', reason: 'missing_service_tier', observations: 40 },
    ],
  };
  const usage = (sessions: number, measuredSessions: number) => ({
    sessions,
    measured: measuredSessions,
    pct: (measuredSessions / sessions) * 100,
    gaps: measuredSessions < sessions ? ['no_selected_usage' as const] : [],
  });
  out.usage_coverage = {
    total: usage(1234, 1080),
    by_host: [],
    by_surface: [
      { host: 'claude', surface: 'cli', usage: usage(900, 900) },
      { host: 'claude', surface: 'desktop', usage: usage(200, 150) },
      { host: 'cursor', surface: null, usage: usage(134, 30) },
    ],
  };
  out.capture_coverage = [
    {
      host: 'claude',
      surface: 'cli',
      inventory: 'unknown',
      observed_sessions: 900,
      captured_sessions: 450,
      unknown_start_sessions: 3,
      pct: 50,
      incomplete_reasons: [],
    },
  ];
  out.hands_off_excluded_surfaces = [
    { host: 'claude', surface: 'batch', qualifying_sessions: 4, degenerate_sessions: 2 },
  ];
  out.untimed_history = {
    records: 2318,
    by_surface: [
      { host: 'claude', surface: 'cli', records: 1204 },
      { host: 'claude', surface: 'desktop', records: 806 },
      { host: 'cursor', surface: null, records: 308 },
    ],
  };
  // Fourteen sessions with up to three spans each, newest first, capped.
  const minute = 60_000;
  const start = out.lane_start_ms;
  out.lanes = [];
  out.lane_sessions = [];
  for (let index = 0; index < 14; index += 1) {
    const id = `stress-session-${String(index).padStart(2, '0')}-0123456789abcdef`;
    for (let span = 0; span <= index % 3; span += 1) {
      const offset = 2800 - index * 180 - span * 50;
      out.lanes.push({
        session_id: id,
        host: ['claude', 'codex', 'cursor'][index % 3],
        start_ms: start + offset * minute,
        end_ms: start + (offset + 25) * minute,
      });
    }
    out.lane_sessions.push({
      session_id: id,
      host: ['claude', 'codex', 'cursor'][index % 3],
      repo:
        index % 4 === 3
          ? null
          : `/Users/developer/code/organisation/a-very-long-repository-name-for-layout-${index}`,
      branch: index % 2 ? `feature/a-long-branch-name-for-layout-checks-${index}` : 'main',
      // A saved title on some rows, an unknown start on others, and link
      // counts from zero to three digits with inferred evidence mixed in.
      title:
        index % 3 === 1 ? `A saved session title long enough to need an ellipsis ${index}` : null,
      automated_review: false,
      started_at_ms: index % 4 === 2 ? null : start - index * 86_400_000,
      pr_links: index % 5 === 0 ? 0 : index * 9,
      inferred_pr_links: index % 5 === 0 ? 0 : index % 2,
      // Four-figure costs, partial on some rows, unknown on others.
      cost:
        index % 5 === 4
          ? null
          : {
              total_usd: index % 3 === 0 ? null : 1_234.4 - index * 10,
              priced_subtotal_usd: 1_234.4 - index * 10,
              selected_observations: 9,
              priced_observations: index % 3 === 0 ? 8 : 9,
              unpriced_observations: index % 3 === 0 ? 1 : 0,
              assumed_tier_observations: 0,
              unpriced: [],
            },
    });
  }
  out.lanes_total = 1234;
  out.lanes_truncated = true;
  return out;
}

function emptyReport(report: DashboardMetrics): DashboardMetrics {
  const out = withPrEffort(report, { tile: { known_merged: 0 }, groups: [] });
  const zero = (tile: MetricTile) => ({ ...tile, value: 0, reason: null });
  const { tiles } = out;
  tiles.sessions = zero(tiles.sessions);
  tiles.human_messages = zero(tiles.human_messages);
  tiles.tool_calls = zero(tiles.tool_calls);
  tiles.agent_hours = zero(tiles.agent_hours);
  out.lanes = [];
  out.lane_sessions = [];
  out.lanes_total = 0;
  out.lanes_truncated = false;
  return out;
}

/** An unmeasured range: every tile and total unknown, with its reason. */
function partialReport(report: DashboardMetrics): DashboardMetrics {
  const out = structuredClone(report);
  const unknown = (tile: MetricTile, reason: string) => ({ ...tile, value: null, reason });
  const { tiles } = out;
  tiles.agent_hours_per_day = unknown(tiles.agent_hours_per_day, 'No active spans');
  tiles.concurrency_mean = unknown(tiles.concurrency_mean, 'No overlapping spans');
  tiles.concurrency_max = unknown(tiles.concurrency_max, 'No overlapping spans');
  tiles.hands_off_median = unknown(tiles.hands_off_median, 'No hands-off stretches');
  tiles.hands_off_p90 = unknown(tiles.hands_off_p90, 'No hands-off stretches');
  tiles.ratio = unknown(tiles.ratio, 'No human-in-the-loop time');
  tiles.human_hours_est = unknown(tiles.human_hours_est, 'No human messages');
  tiles.cost = unknown(tiles.cost, 'Selected usage is absent or unpriced');
  out.tokens.counters.total_tokens = null;
  out.cost = { ...out.cost, total_usd: null };
  out.favorite.current = { model: null, output_tokens: null, unknown_reason: 'no_measured_output' };
  out.days = out.days.map((day) => ({ ...day, human_hours_est: null }));
  return out;
}

type Shape = 'plain' | 'stress' | 'empty' | 'partial';
function exportFor(shape: Shape): FixtureExport {
  const out = structuredClone(base);
  const edit = {
    plain: undefined,
    stress: stressReport,
    empty: emptyReport,
    partial: partialReport,
  }[shape];
  if (edit) out.dashboards = out.dashboards.map(edit);
  if (shape === 'stress') out.environments = out.environments.map(syntheticEnvironment);
  if (shape === 'empty')
    out.environments = out.environments.map((report) => ({
      ...report,
      identities: [],
      selected: { ...report.selected, observed: [], unresolved: [] },
      totals: {
        ...report.totals,
        selected_calls: 0,
        strip_calls: 0,
        selected_unresolved_calls: 0,
        identities: 0,
      },
    }));
  return out;
}

const serve = (page: Page, body: FixtureExport) =>
  page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );

async function open(
  page: Page,
  body: FixtureExport,
  width: number,
  height: number,
  scheme: 'light' | 'dark',
) {
  await serve(page, body);
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await expect(page.getByTestId(/^environment-(columns|empty)$/)).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

/** The effort chart plots the chosen measure; the card has no cohort line above it. */
async function expectMeasure(page: Page, measure: 'agent h' | 'cost') {
  await expect(page.getByTestId('effort-chart')).toHaveAttribute(
    'aria-label',
    measure === 'cost' ? /daily dollars, one bar per day/ : /daily hours, one bar per day/,
  );
  await expect(page.getByTestId('effort-cohort')).toHaveCount(0);
}

type Box = {
  top: number;
  bottom: number;
  left: number;
  right: number;
  width: number;
  height: number;
};
/** A card's surfaces: its own background and title, and the kit panel inside it when layered. */
type Layer = {
  background: string;
  title: { color: string; weight: string; size: string };
  panels: number;
  panel: {
    background: string;
    border: string;
    radius: string;
    shadow: string;
    overflow: string;
    inset: { top: number; left: number; right: number; bottom: number };
  } | null;
};
type Geometry = {
  scroll: {
    doc: number[];
    outlet: number[];
    dash: number[];
    tops: number[];
  };
  outlet: Box;
  primary: Record<string, Box | null>;
  /** Measurement or index lines after Sessions; the page has none. */
  lines: number;
  /** Anything drawn of Agent / human hours or Caught by your rules; the page has none. */
  hidden: string[];
  /** From the block above the tiles to the tiles: the page's gap, with no row reserved. */
  tilesGap: number;
  /** Under the Sessions rows to the bottom of its panel: no blank interior. */
  sessionsBlank: number | null;
  tiles: Box[];
  controlsOutside: string[];
  rows: Record<string, number>;
  /** The Effort chart: what it draws, names and labels. */
  effort: {
    days: number;
    bars: number;
    partial: number;
    headline: string | null;
    /** From the total's bottom to the top of the scale's top label, which rises above the plot. */
    headlineClearance: number | null;
    plot: number | null;
    scale: string[];
    unmeasured: boolean;
    ticks: number;
    markers: number;
  } | null;
  clipped: string[];
  layers: {
    canvas: string;
    effort: Layer | null;
    environment: Layer | null;
    sessions: Layer | null;
    tiles: string[];
  };
};

/** Everything the assertions need, measured in one pass in the page. */
function geometry(): Geometry {
  const box = (element: Element | null): Box | null => {
    if (!element) return null;
    const b = element.getBoundingClientRect();
    return {
      top: b.top,
      bottom: b.bottom,
      left: b.left,
      right: b.right,
      width: b.width,
      height: b.height,
    };
  };
  const inside = (b: Box, frame: Box) =>
    b.top >= frame.top - 0.5 &&
    b.bottom <= frame.bottom + 0.5 &&
    b.left >= frame.left - 0.5 &&
    b.right <= frame.right + 0.5;
  const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
  const dash = document.querySelector<HTMLElement>('.xt-dashboard')!;
  const section = (name: string) => {
    const heading = [...document.querySelectorAll('.xt-dashboard h2')].find(
      (h) => h.textContent === name,
    );
    return box(heading?.closest('section') ?? null);
  };
  const view: Box = {
    top: 0,
    left: 0,
    right: window.innerWidth,
    bottom: window.innerHeight,
    width: window.innerWidth,
    height: window.innerHeight,
  };
  // Rows that scroll inside their own region are reachable by scrolling it;
  // every other control must sit in the viewport as drawn.
  const scrollers = '.xt-table-scroll, .xt-env-list';
  const controlsOutside = [...dash.querySelectorAll<HTMLElement>('button, a[href], [tabindex="0"]')]
    .filter((element) => !element.closest(scrollers) && element.getClientRects().length > 0)
    .filter((element) => !inside(box(element)!, view))
    .map(
      (element) =>
        `${element.tagName}: ${element.getAttribute('aria-label') ?? element.textContent?.trim().slice(0, 40)}`,
    );
  const visibleRows = (region: Element | null, selector: string) => {
    if (!region) return -1;
    const frame = box(region)!;
    return [...region.querySelectorAll(selector)].filter((row) => inside(box(row)!, frame)).length;
  };
  const lanes = document.querySelector('.xt-lanes .xt-table-scroll');
  const env = document.querySelector('.xt-env-list[data-compact]');
  const effortChart = document.querySelector<HTMLElement>('.xt-effort-chart');
  const compact =
    '.xt-env-list[data-compact] .xt-env-name, .xt-env-list[data-compact] .xt-kind, .xt-env-list[data-compact] .xt-env-host';
  const clipped = [...document.querySelectorAll<HTMLElement>('.xt-dashboard *')]
    .filter((element) => {
      if (element.closest('.xt-dash-table-scroll, .xt-table-scroll, .sr-only')) return false;
      if (element.matches(compact)) return false;
      const style = getComputedStyle(element);
      const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
      return clips && element.scrollWidth > element.clientWidth + 1;
    })
    .map((element) => `${element.className}: ${element.textContent?.slice(0, 60)}`);
  const layer = (name: string): Layer | null => {
    const heading = [...document.querySelectorAll('.xt-dashboard h2')].find(
      (h) => h.textContent === name,
    );
    const card = heading?.closest('section');
    if (!heading || !card) return null;
    const panel = card.querySelector(':scope > .xt-section-panel');
    const title = getComputedStyle(heading);
    const frame = card.getBoundingClientRect();
    const style = panel ? getComputedStyle(panel) : null;
    const rect = panel?.getBoundingClientRect();
    return {
      background: getComputedStyle(card).backgroundColor,
      title: { color: title.color, weight: title.fontWeight, size: title.fontSize },
      panels: card.querySelectorAll('.xt-section-panel').length,
      panel:
        style && rect
          ? {
              background: style.backgroundColor,
              border: `${style.borderTopWidth} ${style.borderTopStyle} ${style.borderTopColor}`,
              radius: style.borderTopLeftRadius,
              shadow: style.boxShadow,
              overflow: style.overflowY,
              inset: {
                top: Math.round(rect.top - frame.top),
                left: Math.round(rect.left - frame.left),
                right: Math.round(frame.right - rect.right),
                bottom: Math.round(frame.bottom - rect.bottom),
              },
            }
          : null,
    };
  };
  return {
    scroll: {
      doc: [
        document.documentElement.scrollHeight,
        window.innerHeight,
        document.documentElement.scrollWidth,
        window.innerWidth,
      ],
      outlet: [outlet.scrollHeight, outlet.clientHeight, outlet.scrollWidth, outlet.clientWidth],
      dash: [dash.scrollHeight, dash.clientHeight, dash.scrollWidth, dash.clientWidth],
      tops: [window.scrollY, outlet.scrollTop, dash.scrollTop],
    },
    outlet: box(outlet)!,
    primary: {
      heading: box(document.querySelector('.xt-dashboard h1')),
      summary: box(document.querySelector('[data-testid="dashboard-summary"]')),
      tiles: box(document.querySelector('.xt-dash-tiles')),
      effort: section('Effort'),
      environment: section('Environment'),
      sessions: section('Sessions'),
    },
    lines: document.querySelectorAll('.xt-dash-foot, [data-testid="dashboard-index"]').length,
    hidden: [
      ...[...document.querySelectorAll('.xt-dashboard h2')]
        .map((h) => h.textContent ?? '')
        .filter((title) => title === 'Agent / human hours' || title === 'Caught by your rules'),
      ...[
        ...document.querySelectorAll(
          '.xt-dash-hero-row, .xt-hero-chart, .xt-roll, .xt-dashboard [aria-label="Daily values"]',
        ),
      ].map((element) => element.className),
    ],
    tilesGap: (() => {
      const tiles = document.querySelector('.xt-dash-tiles');
      const above = tiles?.previousElementSibling;
      if (!tiles || !above) return Number.NaN;
      return tiles.getBoundingClientRect().top - above.getBoundingClientRect().bottom;
    })(),
    sessionsBlank: (() => {
      // The lane table (or its empty message) is the panel's last content: no
      // caption follows it.
      const table = document.querySelector('.xt-lanes')?.firstElementChild;
      const panel = table?.closest('.xt-section-panel');
      if (!table || !panel || document.querySelector('[data-testid="lanes-disclosure"]'))
        return null;
      return panel.getBoundingClientRect().bottom - table.getBoundingClientRect().bottom;
    })(),
    tiles: [...document.querySelectorAll('.xt-dash-tiles .xt-stat-tile')].map((tile) => box(tile)!),
    controlsOutside,
    rows: {
      lanes: visibleRows(lanes, '.xt-data-row'),
      lanesTotal: lanes ? lanes.querySelectorAll('.xt-data-row').length : -1,
      environment: visibleRows(env, '.xt-env-row'),
      environmentTotal: env ? env.querySelectorAll('.xt-env-row').length : -1,
    },
    effort: effortChart
      ? {
          days: effortChart.querySelectorAll('.xt-effort-column').length,
          bars: effortChart.querySelectorAll('.xt-effort-bar').length,
          partial: effortChart.querySelectorAll('.xt-effort-plus').length,
          headline: document.querySelector('.xt-effort-headline')?.textContent ?? null,
          headlineClearance: (() => {
            const total = document.querySelector('.xt-effort-headline');
            const top = effortChart.querySelector('.xt-effort-scale [data-at="top"]');
            return total && top
              ? top.getBoundingClientRect().top - total.getBoundingClientRect().bottom
              : null;
          })(),
          plot: box(effortChart.querySelector('.xt-effort-plot'))?.height ?? null,
          scale: [...effortChart.querySelectorAll('.xt-effort-scale span')].map(
            (label) => label.textContent ?? '',
          ),
          unmeasured: effortChart.querySelector('.xt-effort-unmeasured') !== null,
          ticks: effortChart.querySelectorAll('.xt-effort-ticks span').length,
          markers: effortChart.querySelectorAll('.xt-effort-marker').length,
        }
      : null,
    clipped,
    layers: {
      canvas: getComputedStyle(document.body).backgroundColor,
      effort: layer('Effort'),
      environment: layer('Environment'),
      sessions: layer('Sessions'),
      tiles: [...document.querySelectorAll('.xt-dash-tiles .xt-stat-tile')].map(
        (tile) => getComputedStyle(tile).backgroundColor,
      ),
    },
  };
}

/** The page fits its window, and every primary part is in view, in order, unclipped. */
function expectFits(g: Geometry, width: number, height: number) {
  const [docScroll, docClient, docScrollW, docClientW] = g.scroll.doc;
  expect(docScroll).toBeLessThanOrEqual(docClient);
  expect(docScrollW).toBe(docClientW);
  expect(docClientW).toBe(width);
  expect(docClient).toBe(height);
  for (const [scrollH, clientH, scrollW, clientW] of [g.scroll.outlet, g.scroll.dash]) {
    expect(scrollH).toBeLessThanOrEqual(clientH + 1);
    expect(scrollW).toBeLessThanOrEqual(clientW + 1);
  }
  expect(g.scroll.tops).toEqual([0, 0, 0]);
  // Sessions is the last block: no measurement or index line follows it.
  expect(g.lines).toBe(0);
  // Agent / human hours and Caught by your rules are not drawn, and the tiles
  // follow the block above them at the page's 5px gap: no row is kept for them.
  expect(g.hidden).toEqual([]);
  expect(g.tilesGap).toBeCloseTo(5, 0);
  const order = ['heading', 'summary', 'tiles', 'effort', 'sessions'] as const;
  for (const name of [...order, 'environment'] as const) {
    const part = g.primary[name];
    expect(part, name).not.toBeNull();
    expect(part!.top, `${name} top`).toBeGreaterThanOrEqual(g.outlet.top - 0.5);
    expect(part!.bottom, `${name} bottom`).toBeLessThanOrEqual(g.outlet.bottom + 0.5);
    expect(part!.left, `${name} left`).toBeGreaterThanOrEqual(g.outlet.left - 0.5);
    expect(part!.right, `${name} right`).toBeLessThanOrEqual(g.outlet.right + 0.5);
    expect(part!.height, `${name} height`).toBeGreaterThan(0);
  }
  // Top to bottom, nothing overlaps its neighbour.
  for (let index = 1; index < order.length; index += 1)
    expect(
      g.primary[order[index]]!.top,
      `${order[index]} below ${order[index - 1]}`,
    ).toBeGreaterThanOrEqual(g.primary[order[index - 1]]!.bottom - 0.5);
  // Sessions ends under its rows, whatever its row count: its panel's 2px
  // padding and 1px border, never a blank interior. A short list leaves the
  // spare height to effort and environment; a long one scrolls inside.
  expect(g.sessionsBlank, 'blank under the Sessions rows').not.toBeNull();
  expect(g.sessionsBlank!).toBeLessThanOrEqual(3.5);
  // The paired cards share their row.
  expect(g.primary.environment!.top).toBeCloseTo(g.primary.effort!.top, 0);
  expect(g.primary.environment!.height).toBeCloseTo(g.primary.effort!.height, 0);
  expect(g.primary.environment!.left).toBeGreaterThanOrEqual(g.primary.effort!.right);
  expect(g.primary.effort!.width / g.primary.environment!.width).toBeCloseTo(1.6, 1);
  // Four tiles on one row.
  expect(g.tiles).toHaveLength(4);
  expect(new Set(g.tiles.map((tile) => Math.round(tile.top))).size).toBe(1);
  for (let index = 1; index < 4; index += 1)
    expect(g.tiles[index].left).toBeGreaterThanOrEqual(g.tiles[index - 1].right);
  // At least three rows of each list are wholly in view, or all of them when fewer.
  expect(g.rows.lanes).toBeGreaterThanOrEqual(Math.min(3, g.rows.lanesTotal));
  expect(g.rows.environment).toBeGreaterThanOrEqual(Math.min(3, g.rows.environmentTotal));
  // Effort states the range's total above one column per day, over a plot of
  // at least its 50px floor with a scale down to zero and a labelled date
  // axis; with no work only the marker axis remains.
  expect(g.effort, 'effort chart').not.toBeNull();
  expect(g.effort!.ticks, 'effort ticks').toBeGreaterThan(0);
  if (g.effort!.days > 0) {
    expect(g.effort!.headline, 'effort total').not.toBeNull();
    if (g.effort!.headlineClearance !== null)
      expect(
        g.effort!.headlineClearance,
        'scale top label clear of the total',
      ).toBeGreaterThanOrEqual(0);
    expect(g.effort!.plot, 'effort plot').toBeGreaterThanOrEqual(50);
    if (!g.effort!.unmeasured) expect(g.effort!.scale.at(-1), 'effort scale').toBe('0');
  }
  expect(g.controlsOutside).toEqual([]);
  expect(g.clipped).toEqual([]);
  expectLayers(g);
}

/**
 * The margin under Sessions, the page's last block: the page's own 8px bottom
 * padding while the rows have spare height, less once a squeeze consumes it,
 * which the scroll heights above would hide. The contract is that it stays
 * positive — Sessions whole, inside the window — and its size is recorded as
 * the evidence of how much spare height a state leaves.
 */
function expectInsideMargin(g: Geometry) {
  expect(g.outlet.bottom - g.primary.sessions!.bottom, 'margin under Sessions').toBeGreaterThan(0);
}
const marginUnder = (g: Geometry) =>
  Math.round((g.outlet.bottom - g.primary.sessions!.bottom) * 100) / 100;

/**
 * Effort and Environment are layered as Sessions is: the title row on
 * the page's canvas, in the kit header's ink and weight, and the body in the
 * kit's inset surface panel with the same border, radius and lift, 6px inside
 * the card under a 36px band. The four tiles stay flat surface cards. Every colour is read from the page in whichever scheme
 * is on and compared with Sessions' own, never with a literal.
 */
function expectLayers(g: Geometry) {
  const { canvas, sessions, tiles } = g.layers;
  expect(sessions).not.toBeNull();
  const reference = sessions!.panel!;
  const surface = reference.background;
  expect(canvas).not.toBe(surface);
  expect(sessions!.background).toBe(canvas);
  expect(reference.inset).toEqual({ top: 37, left: 7, right: 7, bottom: 7 });
  for (const name of ['effort', 'environment'] as const) {
    const card = g.layers[name];
    expect(card, name).not.toBeNull();
    expect(card!.background, `${name} card`).toBe(canvas);
    expect(card!.panels, `${name} panels`).toBe(1);
    const panel = card!.panel!;
    expect(panel.background, `${name} panel`).toBe(surface);
    expect(panel.border, `${name} border`).toBe(reference.border);
    expect(panel.radius, `${name} radius`).toBe(reference.radius);
    expect(panel.shadow, `${name} lift`).toBe(reference.shadow);
    expect(panel.shadow, `${name} lift`).not.toBe('none');
    expect(panel.overflow, `${name} overflow`).toBe('visible');
    expect(panel.inset, `${name} inset`).toEqual(reference.inset);
    expect(card!.title, `${name} title`).toEqual(sessions!.title);
  }
  expect(tiles).toHaveLength(4);
  for (const tile of tiles) expect(tile).toBe(surface);
}

/** Neither the wheel nor the keyboard can move the page. */
async function expectPageStill(page: Page) {
  const heading = page.getByRole('heading', { level: 1 });
  const box = (await heading.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, 900);
  await page.waitForTimeout(150);
  await heading.click();
  await page.keyboard.press('PageDown');
  await page.keyboard.press('End');
  await page.waitForTimeout(150);
  const tops = await page.evaluate(() => [
    window.scrollY,
    document.querySelector('.xt-shell-outlet')!.scrollTop,
    document.querySelector('.xt-dashboard')!.scrollTop,
  ]);
  expect(tops).toEqual([0, 0, 0]);
}

for (const shape of ['plain', 'stress'] as const)
  for (const [width, height] of SIZES)
    for (const scheme of SCHEMES)
      test(`${shape} Dashboard fits ${width}x${height} ${scheme} without scrolling`, async ({
        page,
      }, info) => {
        const errors: string[] = [];
        page.on('pageerror', (error) => errors.push(error.message));
        page.on('console', (message) => {
          if (message.type() === 'error') errors.push(message.text());
        });
        await open(page, exportFor(shape), width, height, scheme);
        const g = await page.evaluate(geometry);
        await info.attach('geometry', { body: JSON.stringify(g, null, 2) });
        await writeFile(info.outputPath('geometry.json'), JSON.stringify(g, null, 2));
        await page.screenshot({
          path: info.outputPath(`viewport-${shape}-${width}-${scheme}.png`),
        });
        expectFits(g, width, height);
        if (shape === 'stress') {
          // Every list holds more than it shows at the minimum, and shows it all
          // only by scrolling inside itself.
          expect(g.rows.lanesTotal).toBe(14);
          expect(g.rows.environmentTotal).toBe(8);
          expect(g.effort).toMatchObject({ days: 7, bars: 5, markers: 3 });
          expect(g.effort!.headline).toBe('18 agent hlast 7 days');
          // Qualifications stay in view beside their measurements: the
          // unresolved counts as their panels' header triangles. Effort draws
          // no work type, so it has none.
          await expect(page.getByTestId('effort-unresolved')).toHaveCount(0);
          await expect(page.getByTestId('environment-unresolved')).toHaveAccessibleName(
            'Unresolved attribution: 38 unresolved observations, including untimed',
          );
          // The lanes are capped, but no caption under the rows says so.
          await expect(page.getByTestId('lanes-disclosure')).toHaveCount(0);
          await expect(page.getByTestId('coverage')).toHaveAccessibleName(
            /^Coverage: .*2,318 untimed records$/,
          );
          await expect(page.getByRole('button', { name: /^Merged PRs/ })).toContainText(
            '5 + 1 unknown',
          );
        }
        await expectPageStill(page);
        expect(errors).toEqual([]);
      });

for (const shape of ['empty', 'partial'] as const)
  test(`${shape} Dashboard fits 1120x720 without scrolling`, async ({ page }, info) => {
    await open(page, exportFor(shape), 1120, 720, shape === 'empty' ? 'light' : 'dark');
    if (shape === 'empty')
      await expect(page.getByText(/No agent activity was recorded in this range/)).toBeVisible();
    else await expect(page.getByTestId('dashboard-summary')).toContainText('Unmeasured');
    const g = await page.evaluate(geometry);
    await info.attach('geometry', { body: JSON.stringify(g, null, 2) });
    await page.screenshot({ path: info.outputPath(`viewport-${shape}-1120.png`) });
    expectFits(g, 1120, 720);
    await expectPageStill(page);
  });

test('a failed report and a failed environment read keep the page in the window', async ({
  page,
}) => {
  // The fixture serves no 14-day environment report and no 14-day Dashboard
  // report, so selecting that range fails each read in turn.
  const broken = exportFor('stress');
  broken.environments = broken.environments.filter((report) => report.window.days !== 14);
  await open(page, broken, 1120, 720, 'light');
  await page.getByRole('radio', { name: '14d' }).click();
  const environment = page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Environment', exact: true }),
  });
  await expect(environment.getByRole('alert')).toContainText(
    'Environment usage could not be loaded.',
  );
  await expect(environment.getByRole('button', { name: 'Retry environment usage' })).toBeVisible();
  expectFits(await page.evaluate(geometry), 1120, 720);
  await page.unrouteAll();
  const failing = exportFor('stress');
  failing.dashboards = failing.dashboards.filter((report) => report.window.days !== 14);
  await serve(page, failing);
  await page.goto('/dashboard');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await page.getByRole('radio', { name: '14d' }).click();
  await expect(page.getByRole('alert')).toContainText('Dashboard metrics could not be loaded.');
  const measured = await page.evaluate(() => {
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    return {
      fits: outlet.scrollHeight <= outlet.clientHeight + 1,
      width: document.documentElement.scrollWidth,
    };
  });
  expect(measured).toEqual({ fits: true, width: 1120 });
  await expect(page.getByRole('button', { name: 'Retry' })).toBeInViewport();
});

/**
 * The secondary details, each one keyboard step from the page, from the card
 * it belongs to: tokens and cost are sections of Method, coverage and untimed
 * history open from the Sessions header, and each panel's unresolved data from
 * the triangle at its header's end.
 */
const OVERLAYS = [
  { name: /^Coverage/, dialog: 'Coverage' },
  { name: 'Method', dialog: 'How effort is counted · daily values' },
  { name: 'Refresh PR facts…', dialog: 'Refresh pull-request facts' },
  { name: '14 days · activity strips, fixed 14 local days', dialog: 'Activity strips' },
  { name: /^Unresolved attribution: /, dialog: 'Unresolved attribution' },
  { name: 'Observed identities · 11', dialog: 'Observed identities · last 7d' },
  { name: 'Configured components · 14', dialog: 'Configured components' },
] as const;

const within = async (dialog: Locator, width: number, height: number) => {
  const box = (await dialog.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(width);
  expect(box.y + box.height).toBeLessThanOrEqual(height);
  expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
};

for (const [width, height, scheme] of [
  [1120, 720, 'light'],
  [1440, 900, 'dark'],
] as const)
  test(`every detail opens over the page and returns to it at ${width}x${height} ${scheme}`, async ({
    page,
  }, info) => {
    await open(page, exportFor('stress'), width, height, scheme);
    const still = await page.evaluate(geometry);
    expectFits(still, width, height);
    const other = width === 1120 ? ([1440, 900] as const) : ([1120, 720] as const);
    for (const { name, dialog: title } of OVERLAYS) {
      const trigger = page.locator('.xt-dashboard').getByRole('button', { name });
      await trigger.focus();
      await expect(trigger).toBeFocused();
      await page.keyboard.press('Enter');
      const dialog = page.getByRole('dialog', { name: title });
      await expect(dialog).toBeVisible();
      await within(dialog, width, height);
      // Focus is inside the dialog, and the page behind it has not moved.
      expect(await dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true);
      const during = await page.evaluate(geometry);
      expect(during.primary).toEqual(still.primary);
      expect(during.scroll.tops).toEqual([0, 0, 0]);
      // Resizing while it is open keeps the dialog and the page in the window.
      await page.setViewportSize({ width: other[0], height: other[1] });
      await expect(dialog).toBeVisible();
      await within(dialog, other[0], other[1]);
      expectFits(await page.evaluate(geometry), other[0], other[1]);
      await page.setViewportSize({ width, height });
      await within(dialog, width, height);
      if (title === 'Coverage')
        await page.screenshot({
          path: info.outputPath(`overlay-${title.split(' ')[0]}-${width}-${scheme}.png`),
        });
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(trigger).toBeFocused();
      expect((await page.evaluate(geometry)).primary).toEqual(still.primary);
    }
    // The range changes with every dialog closed; the page still fits, and a
    // reopened detail follows the new range.
    await page.getByRole('radio', { name: '30d' }).click();
    await expect(page.locator('.xt-effort-column')).toHaveCount(30);
    expectFits(await page.evaluate(geometry), width, height);
    await page.getByRole('button', { name: 'Method', exact: true }).click();
    const tokens = page.getByRole('dialog', { name: 'How effort is counted · daily values' });
    await expect(
      tokens.getByRole('group', { name: 'Output tokens per day' }).getByRole('img'),
    ).toHaveCount(30);
    await within(tokens, width, height);
    await page.keyboard.press('Escape');
    await expect(tokens).toHaveCount(0);
  });

/**
 * The tables inside the dialogs scroll inside a named region that takes
 * focus from Tab, so the keyboard reaches every one of thirty days. Chromium
 * focuses such a region on its own; WebKit does not, which is why the
 * region is an explicit tab stop.
 */
test('the daily tables inside the dialogs scroll from the keyboard at 1120x720', async ({
  page,
}) => {
  await open(page, exportFor('stress'), 1120, 720, 'light');
  await page.getByRole('radio', { name: '30d' }).click();
  await expect(page.locator('.xt-effort-column')).toHaveCount(30);
  /** End reaches the last of the thirty rows and Home the first, inside `region`. */
  const scrollRows = async (region: Locator) => {
    await expect(region).toBeFocused();
    // A visible focus ring on the region itself.
    expect(await region.evaluate((node) => getComputedStyle(node).outlineStyle)).not.toBe('none');
    expect(await region.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
    const rows = region.locator('tbody tr');
    await expect(rows).toHaveCount(30);
    await page.keyboard.press('End');
    await expect
      .poll(() => region.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
      .toBeLessThanOrEqual(1);
    const [last, frame] = await Promise.all([rows.last().boundingBox(), region.boundingBox()]);
    expect(last!.y).toBeGreaterThanOrEqual(frame!.y - 1);
    expect(last!.y + last!.height).toBeLessThanOrEqual(frame!.y + frame!.height + 1);
    await page.keyboard.press('Home');
    await expect.poll(() => region.evaluate((node) => node.scrollTop)).toBe(0);
    const first = (await rows.first().boundingBox())!;
    expect(first.y).toBeGreaterThanOrEqual(frame!.y - 1);
    expect(first.y + first.height).toBeLessThanOrEqual(frame!.y + frame!.height + 1);
  };
  /** Escape closes the dialog, returns focus, and has left the page as it was. */
  const escape = async (dialog: Locator, trigger: Locator, before: Geometry) => {
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
    expect((await page.evaluate(geometry)).primary).toEqual(before.primary);
  };

  // Method: from Close, one Tab reaches the region; the day, both measures
  // and the merged numbers fit its width in dollars mode.
  await page.getByRole('radio', { name: 'cost' }).click();
  await expectMeasure(page, 'cost');
  // Dollars mode is a page change of its own; the dialog is measured against
  // the page as it is now.
  const before = await page.evaluate(geometry);
  expectFits(before, 1120, 720);
  const method = page.locator('.xt-dashboard').getByRole('button', { name: 'Method' });
  await method.focus();
  await page.keyboard.press('Enter');
  const methodDialog = page.getByRole('dialog', {
    name: 'How effort is counted · daily values',
  });
  await expect(methodDialog.getByRole('button', { name: 'Close' })).toBeFocused();
  await page.keyboard.press('Tab');
  const methodRegion = methodDialog.getByRole('region', {
    name: 'Effort per day scroll area',
  });
  await scrollRows(methodRegion);
  expect(await methodRegion.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(
    true,
  );
  await escape(methodDialog, method, before);

  // Tokens per day, Method's usage section: the daily token values open inside
  // it first, then one Tab from their summary reaches the region.
  await method.focus();
  await page.keyboard.press('Enter');
  await expect(methodDialog.getByRole('button', { name: 'Close' })).toBeFocused();
  const summary = methodDialog.getByTestId('usage-tokens').locator('summary');
  await summary.focus();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Tab');
  await scrollRows(
    methodDialog.getByRole('region', { name: 'Recorded tokens per day scroll area' }),
  );
  await escape(methodDialog, method, before);
});

test('the lists scroll inside themselves from the keyboard at 1120x720', async ({ page }) => {
  await open(page, exportFor('stress'), 1120, 720, 'light');
  const before = await page.evaluate(geometry);
  // The lanes always overflow here. The identity list shows at most eight rows,
  // which can all fit at this size; then it has nothing to scroll, and every
  // row must sit wholly inside it.
  for (const [region, last, mustScroll] of [
    [page.getByRole('region', { name: 'Session lanes scroll area' }), '.xt-data-row', true],
    [page.getByRole('list', { name: /^Most-called identities/ }), '.xt-env-row', false],
  ] as const) {
    await region.focus();
    await expect(region).toBeFocused();
    const overflows = await region.evaluate((node) => node.scrollHeight > node.clientHeight);
    if (mustScroll) expect(overflows).toBe(true);
    if (!overflows) {
      const [item, frame] = await Promise.all([
        region.locator(last).last().boundingBox(),
        region.boundingBox(),
      ]);
      expect(item!.y + item!.height).toBeLessThanOrEqual(frame!.y + frame!.height + 1);
      continue;
    }
    await page.keyboard.press('End');
    await expect
      .poll(() => region.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
      .toBeLessThanOrEqual(1);
    const rows = region.locator(last);
    const [item, frame] = await Promise.all([rows.last().boundingBox(), region.boundingBox()]);
    expect(item!.y + item!.height).toBeLessThanOrEqual(frame!.y + frame!.height + 1);
    await page.keyboard.press('Home');
    // WebKit's animated keyboard scroll can settle a pixel short of the top.
    await expect.poll(() => region.evaluate((node) => node.scrollTop)).toBeLessThanOrEqual(1);
  }
  // The effort days are tab stops in order, and each opens its card over the
  // page without moving it.
  const days = page.getByTestId('effort-day');
  await expect(days).toHaveCount(7);
  await days.first().focus();
  for (let step = 1; step < 7; step += 1) {
    await page.keyboard.press('Tab');
    await expect(days.nth(step)).toBeFocused();
  }
  // The open card only: a closing one can still be leaving as the next opens.
  await expect(page.locator('[data-testid="effort-day-card"][data-open]')).toBeVisible();
  await days.last().blur();
  // Scrolling a list never moved the page.
  const after = await page.evaluate(geometry);
  expect(after.primary).toEqual(before.primary);
  expect(after.scroll.tops).toEqual([0, 0, 0]);
});

/**
 * At 14 days every bar, date tick and merge marker is placed from the one
 * day-centre mapping: a day's marker sits exactly under that day's bar, and
 * the ticks label every other day counted back from the last. The chart
 * follows the measure too: one bar per day with agent time, then one per day
 * with priced dollars, the partly priced day marked +, on a scale in dollars.
 */
test('the chart, its ticks and its markers share one day mapping at 14d, 1120x720', async ({
  page,
}, info) => {
  await open(page, exportFor('stress'), 1120, 720, 'light');
  await page.getByRole('radio', { name: '14d' }).click();
  await expect(page.locator('.xt-effort-column')).toHaveCount(14);
  const agent = await page.evaluate(geometry);
  expectFits(agent, 1120, 720);
  expect(agent.effort).toMatchObject({ days: 14, bars: 5, partial: 0, ticks: 7, markers: 3 });
  expect(agent.effort!.scale).toEqual(['6 h', '3 h', '0']);
  const measured = await page.evaluate(() => {
    const chart = document.querySelector<HTMLElement>('.xt-effort-chart')!;
    const plot = chart.querySelector<HTMLElement>('.xt-effort-plot')!.getBoundingClientRect();
    // Where the mapping puts a day's centre on the screen, from the plot's own box.
    const centre = (index: number) => plot.left + ((index + 0.5) / 14) * plot.width;
    const middle = (element: Element) => {
      const box = element.getBoundingClientRect();
      return box.left + box.width / 2;
    };
    const markers = [...chart.querySelectorAll<HTMLElement>('.xt-effort-marker')].map((marker) => ({
      x: middle(marker),
      label: marker.getAttribute('aria-label')!,
    }));
    const ticks = [...chart.querySelectorAll<HTMLElement>('.xt-effort-ticks span')].map((tick) => ({
      x: middle(tick),
      text: tick.textContent,
    }));
    const bars = [...chart.querySelectorAll<HTMLElement>('.xt-effort-bar')].map((bar) => ({
      x: middle(bar),
      height: bar.getBoundingClientRect().height,
    }));
    return {
      markers,
      ticks,
      bars,
      plotHeight: plot.height,
      expected: {
        markers: [1, 2, 3].map(centre),
        ticks: [1, 3, 5, 7, 9, 11, 13].map(centre),
        bars: [1, 2, 3, 4, 5].map(centre),
      },
    };
  });
  await info.attach('effort-14d', { body: JSON.stringify(measured, null, 2) });
  // Every marker, tick and bar is drawn centred on its day, within a pixel.
  expect(measured.markers.map((marker) => marker.x)).toHaveLength(3);
  measured.markers.forEach((marker, index) =>
    expect(Math.abs(marker.x - measured.expected.markers[index]!)).toBeLessThanOrEqual(1),
  );
  expect(measured.bars).toHaveLength(5);
  measured.bars.forEach((bar, index) =>
    expect(Math.abs(bar.x - measured.expected.bars[index]!)).toBeLessThanOrEqual(1),
  );
  // One shared scale: the 6 h day fills the plot and the 3 h day half of it.
  expect(Math.abs(measured.bars[3]!.height - measured.plotHeight)).toBeLessThanOrEqual(1);
  expect(Math.abs(measured.bars[1]!.height * 3 - measured.plotHeight)).toBeLessThanOrEqual(1.5);
  expect(measured.ticks).toHaveLength(7);
  measured.ticks.forEach((tick, index) =>
    expect(Math.abs(tick.x - measured.expected.ticks[index]!)).toBeLessThanOrEqual(1),
  );
  expect(measured.ticks.map((tick) => tick.text)).toEqual([
    '08-26',
    '08-28',
    '08-30',
    '09-01',
    '09-03',
    '09-05',
    '09-07',
  ]);
  expect(measured.markers[0]!.label).toMatch(
    /^2026-08-26: 1 merged pull request · xtrace\/app#1 · docs · exact link · refreshed$/,
  );
  await page.screenshot({ path: info.outputPath('effort-14d-agent.png') });
  // Dollars: the same five days priced, the unresolved day a priced subtotal
  // beside one unpriced response and marked +, and no stale agent-hour content
  // anywhere on the chart.
  await page.getByRole('radio', { name: 'cost' }).click();
  await expectMeasure(page, 'cost');
  const dollars = await page.evaluate(geometry);
  expectFits(dollars, 1120, 720);
  expect(dollars.effort).toMatchObject({ days: 14, bars: 5, partial: 1, ticks: 7, markers: 3 });
  expect(dollars.effort!.scale).toEqual(['$80', '$40', '0']);
  expect(dollars.effort!.headline).toBe('$189+last 14 days1 response has no price');
  expect(await page.locator('.xt-effort-card').textContent()).not.toMatch(/\d h\b/);
  await page.screenshot({ path: info.outputPath('effort-14d-dollars.png') });
});

/**
 * Below the supported window height the outlet scrolls: no list gives up any
 * of its three rows, and the layered panels pass the card's height down as
 * before, so a squeeze shows as page scroll rather than as lost rows.
 */
test('a window too short for the page scrolls the outlet and keeps three rows in every list', async ({
  page,
}) => {
  await open(page, exportFor('stress'), 1120, 480, 'light');
  const g = await page.evaluate(geometry);
  const [scrollHeight, clientHeight] = g.scroll.outlet;
  expect(scrollHeight).toBeGreaterThan(clientHeight);
  expect(g.effort!.plot).toBeGreaterThanOrEqual(50);
  expect(g.rows.environment).toBeGreaterThanOrEqual(3);
  expect(g.rows.lanes).toBeGreaterThanOrEqual(3);
  expectLayers(g);
  const sessions = page.getByRole('link', { name: 'View sessions' });
  await sessions.scrollIntoViewIfNeeded();
  await expect(sessions).toBeInViewport();
});

/**
 * Environment reports shaped like the counts a real local index produced —
 * 13,558 observed calls, 48 identities, 4,913 unresolved observations
 * including untimed ones, which root saw wrap the card's summary at 1120×720
 * and push the page's last block out of the window — and a larger shape, so
 * the fix is not tuned to one numeral. Synthetic throughout: the rows are the
 * template identities repeated with computed shares (`shapedEnvironment`);
 * nothing is copied from a real index. Six index statuses, three of them with
 * the long diagnostic reasons (a system error, a path, a locked database) that
 * once added a footer line and scrolled the page: the Dashboard states none of
 * them, and Settings states each whole.
 */
const SHAPES = {
  observed: { calls: 13_558, identities: 48, unresolved: 4_913 },
  larger: { calls: 123_456, identities: 120, unresolved: 12_345 },
} as const;
const host = (name: string, state: NativeHostState) => ({
  host: name,
  state,
  detail: null,
  sessions_imported: 1,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 3,
  records_enriched: 0,
  diagnostics: 0,
});
const live: NativeIndexStatus = {
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/synthetic/python3' },
  readers: { state: 'verified', commit: '0'.repeat(40), plugin_version: '0.0.0' },
  hosts: [host('claude', 'complete'), host('codex', 'complete'), host('cursor', 'missing_source')],
  reconciles: 12,
  files_scanned: 1234,
};
const degraded = (reason: string): NativeIndexStatus => ({
  ...live,
  freshness: { freshness: 'degraded', reason },
});
/** Each status; the reasons are the shapes the watcher and the index report. */
const INDEX: Record<string, { status: NativeIndexStatus }> = {
  live: { status: live },
  scanning: {
    status: {
      ...live,
      phase: { phase: 'scanning' },
      freshness: { freshness: 'unknown' },
      hosts: [host('claude', 'pending'), host('codex', 'pending'), host('cursor', 'pending')],
      reconciles: 0,
    },
  },
  degraded: {
    status: degraded('watcher reported an error: too many open files (os error 24)'),
  },
  degradedPath: {
    status: degraded(
      '/Users/synthetic/.claude/projects could not be watched: it is absent or not a directory',
    ),
  },
  degradedWatcher: {
    status: degraded('filesystem watcher could not be created: Too many open files (os error 24)'),
  },
  disabled: {
    status: {
      ...live,
      phase: {
        phase: 'disabled',
        reason:
          'the index database could not be opened: /Users/synthetic/Library/Application Support/xtrace/index.sqlite is locked by another process (synthetic)',
      },
      freshness: { freshness: 'unknown' },
      hosts: [],
      reconciles: 0,
      files_scanned: 0,
    },
  },
};
function shapedExport(shape: keyof typeof SHAPES, index: NativeIndexStatus): FixtureExport {
  const out = exportFor('stress');
  out.environments = out.environments.map((report) => shapedEnvironment(report, SHAPES[shape]));
  out.native_index = index;
  return out;
}
for (const shape of ['observed', 'larger'] as const)
  for (const scheme of SCHEMES)
    test(`the ${shape} real-shaped Environment fits 1120x720 ${scheme} in every range, measure and index state`, async ({
      page,
    }, info) => {
      const { identities, unresolved } = SHAPES[shape];
      const count = (value: number) => value.toLocaleString('en-US');
      const margins: Record<string, number> = {};
      for (const [state, index] of Object.entries(INDEX)) {
        await page.unrouteAll();
        await open(page, shapedExport(shape, index.status), 1120, 720, scheme);
        // The index's state is the sidebar's and Settings'; the Dashboard adds no line for it
        // and carries none of its reasons.
        await expect(page.getByTestId('dashboard-index')).toHaveCount(0);
        const reasons = [index.status.phase, index.status.freshness]
          .map((part) => ('reason' in part ? part.reason : ''))
          .filter(Boolean);
        for (const reason of reasons)
          await expect(page.locator('.xt-dashboard')).not.toContainText(reason);
        for (const range of ['7d', '30d'] as const) {
          if (range !== '7d') await page.getByRole('radio', { name: range }).click();
          await expect(page.getByTestId('environment-columns')).toContainText(`last ${range}`);
          // The card shows no summary line of call and identity totals.
          await expect(page.getByTestId('environment-summary')).toHaveCount(0);
          for (const measure of ['agent h', 'cost'] as const) {
            await page.getByRole('radio', { name: measure }).click();
            await expectMeasure(page, measure);
            const g = await page.evaluate(geometry);
            await info.attach(`geometry-${state}-${range}-${measure}`, {
              body: JSON.stringify(g, null, 2),
            });
            expectFits(g, 1120, 720);
            expectInsideMargin(g);
            margins[`${state} ${range} ${measure}`] = marginUnder(g);
            await expect(page.getByTestId('environment-unresolved')).toHaveAccessibleName(
              `Unresolved attribution: ${count(unresolved)} unresolved observations, including untimed`,
            );
            await expect(
              page.getByRole('list', { name: `Most-called identities, top 8 of ${identities}` }),
            ).toBeVisible();
            await expect(
              page.getByRole('button', { name: `Observed identities · ${identities}` }),
            ).toBeVisible();
          }
        }
        await expectPageStill(page);
        if (state === 'live')
          await page.screenshot({ path: info.outputPath(`shaped-${shape}-${scheme}.png`) });
        // The full reason is on Settings, as the app's own sentences state it.
        await page.getByRole('button', { name: 'Settings', exact: true }).click();
        const settings = page.getByRole('region', { name: 'Native index' });
        await expect(settings).toContainText(phaseText(index.status));
        await expect(settings).toContainText(freshnessText(index.status));
      }
      await info.attach('margins under Sessions', { body: JSON.stringify(margins, null, 2) });
    });

/**
 * At 30d, in both measures, the page still fits — nothing scrolls, every
 * primary part and three rows of every list are in view, and nothing of the
 * hidden panels is drawn — and the margin under Sessions stays positive; the
 * margins are recorded.
 */
for (const scheme of SCHEMES)
  test(`the 30d state fits 1120x720 ${scheme}`, async ({ page }, info) => {
    await open(page, shapedExport('observed', live), 1120, 720, scheme);
    await page.getByRole('radio', { name: '30d' }).click();
    await expect(page.getByTestId('environment-columns')).toContainText('last 30d');
    const margins: Record<string, number> = {};
    for (const measure of ['agent h', 'cost'] as const) {
      await page.getByRole('radio', { name: measure }).click();
      await expectMeasure(page, measure);
      const g = await page.evaluate(geometry);
      expectFits(g, 1120, 720);
      expectInsideMargin(g);
      margins[measure] = marginUnder(g);
    }
    await expectPageStill(page);
    await info.attach('margins under Sessions', { body: JSON.stringify(margins, null, 2) });
  });
