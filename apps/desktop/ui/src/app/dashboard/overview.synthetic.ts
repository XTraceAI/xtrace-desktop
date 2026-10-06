import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricHumanStretch } from '../../data/generated/MetricHumanStretch';
import type { MetricPrAnalyticsRow } from '../../data/generated/MetricPrAnalyticsRow';
import type { MetricTile } from '../../data/generated/MetricTile';
import type { PrAnalyticsPage } from '../../data/generated/PrAnalyticsPage';

/**
 * Test-only synthetic Overview and "your hours" data: not real history and
 * not a design sample. It fills a generated F1 report's days with a fixed,
 * repeatable working pattern (morning and afternoon stretches, a late evening
 * on some days, single messages, lighter weekends), agent hours to match, and
 * per-day concurrency and hands-off values, so the browser checks and
 * screenshots have something to draw at every range. Every derived number
 * (totals, leverage) is computed here from those synthetic values, the way
 * the Rust report would add them up.
 */
const H = 3_600_000;
const M = 60_000;

/** A small repeatable hash of a date, so every run draws the same days. */
function seed(date: string) {
  let h = 2166136261;
  for (const c of date) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  // Mix the bits, so dates one character apart land far apart.
  h = Math.imul(h ^ (h >>> 16), 2246822507);
  h = Math.imul(h ^ (h >>> 13), 3266489909);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}
const weekend = (date: string) => {
  const [y, m, d] = date.split('-').map(Number);
  return [0, 6].includes(new Date(Date.UTC(y!, m! - 1, d!)).getUTCDay());
};

function dayStretches(date: string, midnight: number): [number, number][] {
  const r = seed(date);
  const at = (hours: number) => midnight + Math.round(hours * 60) * M;
  if (weekend(date))
    return r < 0.4
      ? []
      : [
          [at(11 + r * 2), at(11.8 + r * 2)],
          [at(20.5), at(20.5)],
        ];
  const q = seed(`${date}q`);
  const out: [number, number][] = [
    [at(8.6 + r * 2), at(10.4 + r * 2 + q)],
    [at(13 + q * 1.5), at(15.2 + q * 1.5 + r * 2)],
    [at(17.5 + q), at(17.5 + q)],
  ];
  if (q > 0.5) out.push([at(19.4 + r), at(20.1 + r)]);
  if (r > 0.55) out.push([at(21.5 + q), at(24)]);
  return out;
}

const measured = (tile: MetricTile, value: number, previous: number): MetricTile => ({
  ...tile,
  value,
  reason: null,
  current_n: 40,
  previous_n: 36,
  delta: {
    previous,
    pct: Math.round(((value - previous) / previous) * 1000) / 10,
    suppressed: false,
  },
});

/**
 * `effort: false` keeps the report's own Effort chart and agent hours;
 * `tiles: false` keeps its tiles. Your hours and the daily lines are always
 * replaced.
 */
export function syntheticOverview(
  report: DashboardMetrics,
  { effort: drawEffort = true, tiles: drawTiles = true } = {},
): DashboardMetrics {
  const human = report.human_hours.current;
  let yours = 0;
  let agent = 0;
  human.by_day = human.by_day.map((day, index) => {
    // A first day that starts mid-day keeps only what is in it.
    const midnight = day.end_ms - 24 * H;
    const stretches: MetricHumanStretch[] = dayStretches(day.date, midnight)
      .map(([start, end]) => ({
        start_ms: Math.max(start, day.start_ms),
        end_ms: Math.min(end, day.end_ms),
      }))
      .filter((s) => s.start_ms <= s.end_ms && s.start_ms >= day.start_ms);
    const active = stretches.reduce((sum, s) => sum + (s.end_ms - s.start_ms), 0);
    yours += active;
    const agentHours = drawEffort
      ? Math.round((active / H) * (2.5 + seed(`${day.date}a`) * 6) * 10) / 10
      : report.days[index]!.agent_hours;
    agent += agentHours;
    report.days[index]!.agent_hours = agentHours;
    // The Effort bars draw the same synthetic agent hours, on one model.
    const effort = report.pr_effort.current.cohort.by_day[index];
    if (effort && drawEffort) {
      effort.agent_ms = Math.round(agentHours * H);
      effort.models = [
        {
          model: 'synthetic-model',
          priced_nano_usd: 0,
          priced_observations: 0,
          unpriced_observations: 0,
          agent_ms: effort.agent_ms,
        },
      ];
    }
    return { ...day, active_ms: active, stretches };
  });
  human.active_ms = yours;
  // Leverage's agent hours are the same synthetic hours, over the same days.
  report.leverage = {
    agent_ms: Math.round(agent * H),
    previous_agent_ms: Math.round(agent * H * 0.99),
    by_day: human.by_day.map((day, index) => {
      const agentMs = Math.round(report.days[index]!.agent_hours * H);
      return {
        date: day.date,
        start_ms: day.start_ms,
        end_ms: day.end_ms,
        agent_ms: agentMs,
        human_ms: day.active_ms,
        value: day.active_ms ? agentMs / day.active_ms : null,
      };
    }),
  };
  const cohort = report.pr_effort.current.cohort;
  if (drawEffort) cohort.agent_ms = cohort.by_day.reduce((sum, day) => sum + day.agent_ms, 0);
  human.messages = 40 * human.by_day.length;
  report.human_hours.previous_active_ms = Math.round(yours * 0.9);
  report.concurrency_by_day = report.concurrency_by_day.map((day) => {
    const r = seed(`${day.date}c`);
    const quiet = weekend(day.date);
    return {
      ...day,
      max: quiet ? 2 + Math.round(r * 2) : 4 + Math.round(r * 10),
      mean: Math.round((quiet ? 1.2 + r : 2 + r * 3) * 10) / 10,
    };
  });
  report.hands_off_by_day = report.hands_off_by_day.map((day) => {
    const r = seed(`${day.date}h`);
    return {
      ...day,
      n: 20 + Math.round(r * 40),
      median_min: Math.round((1.4 + r * 3.5) * 10) / 10,
      p90_min: Math.round((9 + r * 15) * 10) / 10,
    };
  });
  if (!drawTiles) return report;
  const { tiles } = report;
  tiles.agent_hours = measured(tiles.agent_hours, agent, agent * 0.8);
  const leverage = agent / (yours / H);
  const previous = (agent * 0.99) / (Math.round(yours * 0.9) / H);
  tiles.leverage = measured(tiles.leverage, leverage, previous);
  tiles.concurrency_mean = measured(tiles.concurrency_mean, 3.1, 2.4);
  tiles.concurrency_max = measured(tiles.concurrency_max, 14, 11);
  tiles.hands_off_median = measured(tiles.hands_off_median, 2.6, 3.2);
  tiles.hands_off_p90 = measured(tiles.hands_off_p90, 18.4, 21);
  tiles.merged_prs = measured(tiles.merged_prs, 12, 9);
  report.pr_effort.current.tile = {
    ...report.pr_effort.current.tile,
    complete: true,
    known_merged: MERGED,
    unknown_facts: 0,
    merged: MERGED,
  };
  // The merged pull requests the number counts: the latest ones are the
  // synthetic PRs page's rows, which add their titles and hours.
  report.pr_effort.current.markers = Array.from({ length: MERGED }, (_, index) => {
    const merged_at_ms = mergedAt(report.window.end_ms, index);
    return {
      repository: 'example/app',
      number: prNumber(index),
      url: `https://github.com/example/app/pull/${prNumber(index)}`,
      merged_at: new Date(merged_at_ms).toISOString(),
      merged_at_ms,
      date: new Date(merged_at_ms).toISOString().slice(0, 10),
      work_type: null,
      confidence: 'exact' as const,
      freshness: { state: 'refreshed' as const },
    };
  });
  return report;
}

/** Synthetic merged pull requests in the range; the latest `TITLES.length` have page rows. */
const MERGED = 12;
/** The `index`th synthetic merged pull request, oldest first, 9 hours apart, the last 9 h before the end. */
const prNumber = (index: number) => 200 + (index - (MERGED - TITLES.length)) * 3;
const mergedAt = (end: number, index: number) => end - (MERGED - index) * 9 * H;

const TITLES = [
  'Usage bar color states',
  'Compaction calculation spinner',
  'Tooltip length on dashboard',
  'Running session indicator',
  'Settings layout for narrow windows',
  'Retry a failed refresh once',
];

/** Synthetic merged pull requests in a PRs page report, latest last. */
export function syntheticMergedPage(page: PrAnalyticsPage): PrAnalyticsPage {
  const end = page.window.end_ms;
  page.report.rows = TITLES.map((title, offset) => {
    const index = MERGED - TITLES.length + offset;
    return {
      repository: 'example/app',
      number: prNumber(index),
      url: `https://github.com/example/app/pull/${prNumber(index)}`,
      title,
      merged_at: new Date(mergedAt(end, index)).toISOString(),
      merged_at_ms: mergedAt(end, index),
      agent_ms: Math.round((0.8 + seed(title) * 6) * H),
    } as MetricPrAnalyticsRow;
  });
  return page;
}
