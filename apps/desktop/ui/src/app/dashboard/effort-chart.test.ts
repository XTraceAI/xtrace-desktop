import { expect, it } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { MetricPrEffort } from '../../data/generated/MetricPrEffort';
import {
  NO_MODEL,
  dayCenter,
  dayLabel,
  effortChart,
  niceTop,
  rangeText,
  scaleText,
  tickIndices,
} from './effort-chart';
import {
  F19_SHARED,
  MANY_TYPES,
  NO_LINKS,
  PARTIAL_COST,
  SYNTHETIC_MODEL,
  UNKNOWN_FACTS,
  withPrEffort,
  type SectionSpec,
} from './pr-effort.synthetic';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const report = (days: number) =>
  structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
/** A synthetic M-19 section over the generated window of `days` local days. */
const section = (spec: SectionSpec, days = 7): MetricPrEffort =>
  withPrEffort(report(days), spec).pr_effort.current;
const HOUR = 3_600_000;
const ty = (work_type: string) => ({ kind: 'type' as const, work_type });

it('draws one bar per day for the whole cohort, every assignment added, on one shared scale', () => {
  const current = section({
    tile: { known_merged: 0 },
    groups: [
      {
        assignment: ty('feat'),
        sessions: 2,
        days: { 1: { agentMs: 2 * HOUR }, 2: { agentMs: HOUR } },
      },
      {
        assignment: ty('fix'),
        sessions: 1,
        days: { 1: { agentMs: HOUR }, 2: { agentMs: 4 * HOUR, model: 'other-model' } },
      },
    ],
  });
  const chart = effortChart(current, 'agent', 7);
  expect(chart.bars).toHaveLength(7);
  expect(chart.bars.map((bar) => bar.value)).toEqual([null, 3, 5, null, null, null, null]);
  expect(chart.bars.map((bar) => bar.state)).toEqual([
    'none',
    'measured',
    'measured',
    'none',
    'none',
    'none',
    'none',
  ]);
  // The busiest day, 5 h, is the scale's round top: every bar on the same one.
  expect(chart.scale).toEqual({ top: 5, topText: '5 h', midText: '2.5 h' });
  expect(chart.bars[1]!.fraction).toBe(0.6);
  expect(chart.bars[2]!.fraction).toBe(1);
  // Below 12 h nothing reaches a full day: no 24 h line.
  expect(chart.reference).toBeNull();
  expect(chart.unit).toBe('hours');
  // Each day's models, largest first, with their share of the day.
  expect(chart.bars[2]!.models).toEqual([
    { name: 'other-model', value: 4, text: '4h00m', share: '80%' },
    { name: SYNTHETIC_MODEL, value: 1, text: '1h00m', share: '20%' },
  ]);
  expect(chart.bars[2]!.name).toBe(
    `${chart.days[2]!.date}: 5h00m. other-model 4h00m (80%), ${SYNTHETIC_MODEL} 1h00m (20%)`,
  );
  // A day without agent time is a measured zero, written as one.
  expect(chart.bars[0]!.name).toBe(`${chart.days[0]!.date}: 0h00m`);
  expect(chart.headline).toEqual({ text: '8h00m agent', range: 'last 7 days', note: null });
});

it('draws the 24 h line from a 12 h day, extends the scale to it and notes days above it', () => {
  const chart = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [
        {
          assignment: ty('feat'),
          sessions: 4,
          days: { 0: { agentMs: 13 * HOUR }, 3: { agentMs: 30 * HOUR, model: 'gpt-6-astra' } },
        },
      ],
    }),
    'agent',
    7,
  );
  expect(chart.scale).toEqual({ top: 30, topText: '30 h', midText: '15 h' });
  expect(chart.reference).toEqual({ fraction: 0.8, text: '24 h' });
  expect(chart.bars.map((bar) => bar.overFullDay)).toEqual([
    false,
    false,
    false,
    true,
    false,
    false,
    false,
  ]);
  expect(chart.bars[3]!.name).toBe(
    `${chart.days[3]!.date}: 30h00m. gpt-6-astra 30h00m (100%). Above 24 h: agents ran at the same time`,
  );
  expect(chart.headline).toEqual({
    text: '43h00m agent',
    range: 'last 7 days',
    note: '1 day above 24 h',
  });
  // 12 h exactly draws the line on a scale stretched to a full day.
  const half = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [{ assignment: ty('feat'), sessions: 1, days: { 0: { agentMs: 12 * HOUR } } }],
    }),
    'agent',
    7,
  );
  expect(half.scale).toEqual({ top: 24, topText: '24 h', midText: '12 h' });
  // The line sits on the top line, which the scale's own label names: no second label.
  expect(half.reference).toEqual({ fraction: 1, text: null });
  // Cost never has the line.
  expect(effortChart(section(MANY_TYPES), 'dollars', 7).reference).toBeNull();
});

it('keeps unpriced days out of the bars: a + for a partial day, no bar for an unknown one', () => {
  const chart = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [
        {
          assignment: ty('feat'),
          sessions: 1,
          days: {
            0: { usd: 10 },
            1: { usd: 20, selected: 130, priced: 2, model: 'claude-opus-5-5' },
            2: { selected: 2, priced: 0 },
            3: { usd: 30 },
          },
        },
      ],
    }),
    'dollars',
    7,
  );
  expect(chart.bars.map((bar) => bar.state)).toEqual([
    'measured',
    'measured',
    'unknown',
    'measured',
    'none',
    'none',
    'none',
  ]);
  expect(chart.bars.map((bar) => bar.partial)).toEqual([
    false,
    true,
    false,
    false,
    false,
    false,
    false,
  ]);
  const [, partial, unknown] = chart.bars;
  expect(partial!.value).toBe(20);
  expect(partial!.totalText).toBe('$20.00+');
  expect(partial!.unpriced).toEqual([{ name: 'codex-auto-review', responses: 128 }]);
  expect(partial!.name).toBe(
    `${chart.days[1]!.date}: $20.00+. claude-opus-5-5 $20.00 (100%). No price: 128 codex-auto-review responses`,
  );
  // Nothing priced is unknown, never a zero, and has no height.
  expect(unknown!.value).toBeNull();
  expect(unknown!.fraction).toBe(0);
  expect(unknown!.totalText).toBe('cost unknown');
  expect(unknown!.name).toBe(
    `${chart.days[2]!.date}: cost unknown. No price: 2 codex-auto-review responses`,
  );
  expect(chart.bars[4]!.totalText).toBe('no usage');
  expect(chart.scale).toEqual({ top: 30, topText: '$30', midText: '$15' });
  expect(chart.unit).toBe('dollars');
  expect(chart.headline).toEqual({
    text: '$60.00+',
    range: 'last 7 days',
    note: '130 responses have no price',
  });
});

it('states an entirely unpriced plot instead of drawing a scale for it', () => {
  // A partial day is still a priced subtotal on the scale.
  const partial = effortChart(section(PARTIAL_COST), 'dollars', 7);
  expect(partial.unmeasured).toBeNull();
  expect(partial.bars[2]!.partial).toBe(true);
  // The same cohort's agent time is measured whatever the pricing says.
  const agent = effortChart(section(PARTIAL_COST), 'agent', 7);
  expect(agent.unmeasured).toBeNull();
  expect(agent.bars[2]!.value).toBe(0.5);
  const unknown = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [
        {
          assignment: ty('feat'),
          sessions: 1,
          days: Object.fromEntries(
            [0, 1, 2, 3, 4, 5, 6].map((day) => [day, { selected: 1, priced: 0 }]),
          ),
        },
      ],
    }),
    'dollars',
    7,
  );
  expect(unknown.scale).toBeNull();
  expect(unknown.unmeasured).toBe('No priced daily usage to plot.');
  expect(unknown.bars.every((bar) => bar.state === 'unknown')).toBe(true);
  expect(unknown.headline.text).toBe('cost unknown');
  // No usage at all is not an unmeasured plot; it is nothing to plot.
  const empty = effortChart(section({ tile: { known_merged: 0 }, groups: [] }), 'dollars', 7);
  expect(empty.scale).toBeNull();
  expect(empty.unmeasured).toBeNull();
  expect(empty.bars).toHaveLength(7);
  expect(empty.headline.text).toBe('no usage');
  // Sessions with no agent time say so in hours, as unpriced days do in cost.
  const idle = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [{ assignment: ty('feat'), sessions: 1, days: { 1: { usd: 1 } } }],
    }),
    'agent',
    7,
  );
  expect(idle.scale).toBeNull();
  expect(idle.unmeasured).toBe('No agent time in this range.');
  expect(empty.unmeasured).toBeNull();
});

it('labels a tiny range with values the chart reaches, never 0 h or $0 at the top', () => {
  const tiny = section({
    tile: { known_merged: 0 },
    groups: [{ assignment: ty('feat'), sessions: 1, days: { 2: { agentMs: 12_000, usd: 0.003 } } }],
  });
  const seconds = effortChart(tiny, 'agent', 7);
  expect([seconds.scale!.topText, seconds.scale!.midText]).toEqual(['12 s', '6 s']);
  expect(seconds.bars[2]!.fraction).toBe(1);
  const cents = effortChart(tiny, 'dollars', 7);
  expect([cents.scale!.topText, cents.scale!.midText]).toEqual(['$0.003', '$0.0015']);
  // Tops from 0.01 to 0.03 h read in seconds or minutes, each top distinct from its middle.
  for (const [ms, top, mid] of [
    [36_000, '40 s', '20 s'],
    [72_000, '1.2 min', '0.6 min'],
    [90_000, '1.6 min', '0.8 min'],
    [108_000, '2 min', '1 min'],
  ] as const) {
    const chart = effortChart(
      section({
        tile: { known_merged: 0 },
        groups: [{ assignment: ty('feat'), sessions: 1, days: { 2: { agentMs: ms } } }],
      }),
      'agent',
      7,
    );
    expect([chart.scale!.topText, chart.scale!.midText]).toEqual([top, mid]);
  }
  // From 0.1 h the scale is in hours, with the digits its round top needs.
  const tenth = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [{ assignment: ty('feat'), sessions: 1, days: { 2: { agentMs: 0.11 * HOUR } } }],
    }),
    'agent',
    7,
  );
  expect([tenth.scale!.topText, tenth.scale!.midText]).toEqual(['0.12 h', '0.06 h']);
});

it('lists every model of a day, names no model recorded, and marks a tiny share', () => {
  const chart = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [
        { assignment: ty('feat'), sessions: 1, days: { 1: { agentMs: 10 * HOUR, usd: 500 } } },
        {
          assignment: ty('fix'),
          sessions: 1,
          days: { 1: { agentMs: 60_000, usd: 0.5, model: 'claude-haiku' } },
        },
        {
          assignment: { kind: 'other' },
          sessions: 1,
          days: { 1: { agentMs: HOUR, model: null } },
        },
        {
          assignment: { kind: 'mixed' },
          sessions: 1,
          days: { 1: { agentMs: HOUR, usd: 2, model: 'a' } },
        },
        { assignment: ty('docs'), sessions: 1, days: { 1: { agentMs: HOUR, usd: 1, model: 'b' } } },
      ],
    }),
    'agent',
    7,
  );
  const day = chart.bars[1]!;
  expect(day.models.map((model) => [model.name, model.share])).toEqual([
    [SYNTHETIC_MODEL, '77%'],
    ['a', '8%'],
    ['b', '8%'],
    [NO_MODEL, '8%'],
    ['claude-haiku', '<1%'],
  ]);
  // The name lists the three largest and counts the rest.
  expect(day.name).toBe(
    `${chart.days[1]!.date}: 13h01m. ${SYNTHETIC_MODEL} 10h00m (77%), a 1h00m (8%), b 1h00m (8%) and 2 more models`,
  );
  // Cost: models are those with a priced response; no model recorded had none.
  const cost = effortChart(
    section({
      tile: { known_merged: 0 },
      groups: [
        { assignment: ty('feat'), sessions: 1, days: { 1: { usd: 500 } } },
        { assignment: ty('fix'), sessions: 1, days: { 1: { usd: 0.5, model: 'claude-haiku' } } },
        { assignment: { kind: 'other' }, sessions: 1, days: { 1: { agentMs: HOUR, model: null } } },
      ],
    }),
    'dollars',
    7,
  ).bars[1]!;
  expect(cost.models.map((model) => [model.name, model.text, model.share])).toEqual([
    [SYNTHETIC_MODEL, '$500', '100%'],
    ['claude-haiku', '$0.50', '<1%'],
  ]);
});

it('adds each day’s models up to the day, in both measures, as the report does', () => {
  for (const spec of [F19_SHARED, UNKNOWN_FACTS, NO_LINKS, MANY_TYPES, PARTIAL_COST])
    for (const metric of ['agent', 'dollars'] as const) {
      const current = section(spec);
      const chart = effortChart(current, metric, 7);
      chart.bars.forEach((bar, index) => {
        const day = current.cohort.by_day[index]!;
        const sum = bar.models.reduce((total, model) => total + model.value, 0);
        if (metric === 'agent') expect(sum).toBeCloseTo(day.agent_ms / HOUR, 12);
        else expect(sum).toBeCloseTo(day.cost.priced_subtotal_usd, 9);
        expect(bar.unpriced.reduce((total, part) => total + part.responses, 0)).toBe(
          metric === 'dollars' ? day.cost.unpriced_observations : 0,
        );
      });
      expect(chart.markers.flatMap((day) => day.merged)).toEqual(current.markers);
    }
});

it('positions bars, date ticks and merge markers from one day-centre mapping over 7, 14 and 30 days', () => {
  for (const days of [7, 14, 30] as const) {
    const chart = effortChart(section(F19_SHARED, days), 'dollars', days);
    expect(chart.days).toHaveLength(days);
    expect(chart.bars).toHaveLength(days);
    expect(chart.bars.map((bar) => bar.index)).toEqual([...Array(days).keys()]);
    // One $2.50 session on its event day (the third day).
    expect(chart.bars.filter((bar) => bar.state === 'measured').map((bar) => bar.index)).toEqual([
      2,
    ]);
    expect(chart.headline.range).toBe(`last ${days} days`);
    // The merge markers sit on the fifth and sixth days, by date, on the same axis.
    expect(chart.markers.map((marker) => marker.index)).toEqual([4, 5]);
    expect(chart.bars[4]!.merged.map((marker) => marker.number)).toEqual([1]);
    expect(chart.bars[4]!.name).toBe(`${chart.days[4]!.date}: no usage. merged #1`);
    expect(chart.markers[0]!.text).toBe(
      `${chart.days[4]!.date}: 1 merged pull request · xtrace/app#1 · feat · exact link · checked`,
    );
    for (const index of chart.ticks) expect(index).toBeLessThan(days);
    expect(chart.ticks.at(-1)).toBe(days - 1);
  }
  expect(tickIndices(7)).toEqual([0, 1, 2, 3, 4, 5, 6]);
  expect(tickIndices(14)).toEqual([1, 3, 5, 7, 9, 11, 13]);
  expect(tickIndices(30)).toEqual([0, 4, 9, 14, 19, 24, 29]);
  expect(tickIndices(1)).toEqual([0]);
  expect(tickIndices(0)).toEqual([]);
  expect(dayCenter(0, 7)).toBeCloseTo(1 / 14, 12);
  expect(dayCenter(29, 30)).toBeCloseTo(59 / 60, 12);
  // Two pull requests merged on one day are one count with every identity in words.
  const many = effortChart(section(MANY_TYPES), 'agent', 7);
  const day3 = many.markers.find((marker) => marker.index === 3)!;
  expect(day3.merged).toHaveLength(2);
  expect(day3.text).toMatch(
    /^2026-09-04: 2 merged pull requests · xtrace\/app#4 · feat · commit link · checked; xtrace\/app#5 · fix · exact link · stale after a failed check \(rate limited\)$/,
  );
});

it('rounds the scale up to a round value and labels it compactly', () => {
  expect(niceTop(5)).toBe(5);
  expect(niceTop(5.2)).toBe(6);
  expect(niceTop(26)).toBe(30);
  expect(niceTop(420)).toBe(500);
  expect(niceTop(648.49)).toBe(800);
  expect(niceTop(1100)).toBe(1200);
  expect(niceTop(0.333)).toBe(0.4);
  expect(niceTop(30)).toBe(30);
  expect(scaleText(1500, 'dollars')).toBe('$1.5k');
  expect(scaleText(400, 'dollars')).toBe('$400');
  expect(scaleText(0.2, 'agent')).toBe('0.2 h');
  expect(scaleText(0.0015, 'dollars')).toBe('$0.0015');
  expect(dayLabel('2026-09-04')).toBe('Sep 4');
});

it('names the selected range in the headline, not the count of local-day buckets', () => {
  // A rolling 7-day range that starts mid-day touches eight local days.
  const current = section({ tile: { known_merged: 0 }, groups: [] });
  const last = current.cohort.by_day.at(-1)!;
  current.cohort.by_day = [...current.cohort.by_day, { ...last, date: '2026-09-08' }];
  expect(current.cohort.by_day).toHaveLength(8);
  for (const metric of ['agent', 'dollars'] as const)
    expect(effortChart(current, metric, 7).headline.range).toBe('last 7 days');
  expect(rangeText(7)).toBe('last 7 days');
  expect(rangeText(1)).toBe('last 1 day');
});
