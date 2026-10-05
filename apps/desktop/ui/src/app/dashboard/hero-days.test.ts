import { expect, it } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { DashboardDay } from '../../data/generated/DashboardDay';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { heroSeries } from './hero-days';

const exported = fixture as FixtureExport;
const template = exported.dashboards[0].days[0];
const day = (date: string, agent_hours: number, human_hours_est: number | null): DashboardDay => ({
  ...template,
  date,
  agent_hours,
  human_hours_est,
});

it('draws both series against the one peak either of them reached', () => {
  const { days, peak, humanMeasured } = heroSeries([
    day('2026-09-01', 4, 1),
    day('2026-09-02', 2, 8),
    day('2026-09-03', 0, 0),
  ]);
  // The human estimate is the largest measurement here, so it sets the scale
  // for both: a per-series scale would draw 2 agent hours as tall as 8.
  expect(peak).toBe(8);
  expect(humanMeasured).toBe(true);
  expect(days.map((d) => [d.agent.ratio, d.human.ratio])).toEqual([
    [0.5, 0.125],
    [0.25, 1],
    [0, 0],
  ]);
  expect(days.map((d) => [d.agent.state, d.human.state])).toEqual([
    ['measured', 'measured'],
    ['measured', 'measured'],
    ['measured', 'measured'],
  ]);
});

it('keeps a measured zero apart from an unknown estimate', () => {
  const { days, peak, humanMeasured } = heroSeries([day('2026-09-01', 0, null)]);
  expect(peak).toBe(0);
  expect(humanMeasured).toBe(false);
  // A zero is a measurement with nothing in it; an unknown has no value at all.
  expect(days[0].agent).toEqual({ state: 'measured', value: 0, ratio: 0 });
  expect(days[0].human).toEqual({ state: 'unknown', value: null, ratio: 0 });
});

it('keeps a positive day visible when it is far below the peak', () => {
  const { days } = heroSeries([day('2026-09-01', 100, 100), day('2026-09-02', 0.01, 0)]);
  // A tiny positive is a real fraction of the peak, never rounded away to zero.
  expect(days[1].agent.ratio).toBeCloseTo(0.0001, 12);
  expect(days[1].agent.value).toBe(0.01);
  expect(days[1].human.ratio).toBe(0);
});

it('reports an unmeasured human range on every one of its days', () => {
  const { days, peak, humanMeasured } = heroSeries([
    day('2026-09-01', 1, null),
    day('2026-09-02', 3, null),
  ]);
  // M-07 is unknown for a whole window or none of it, and agent time is a
  // separate measurement that stays drawn.
  expect(humanMeasured).toBe(false);
  expect(peak).toBe(3);
  expect(days.every((d) => d.human.state === 'unknown')).toBe(true);
  expect(days.map((d) => d.agent.ratio)).toEqual([1 / 3, 1]);
});

it('has no peak and no bars to scale when nothing positive was measured', () => {
  expect(heroSeries([]).days).toEqual([]);
  expect(heroSeries([]).peak).toBeNull();
  const unknown = heroSeries([day('2026-09-01', Number.NaN, null)]);
  // A nonfinite value is not a measurement and cannot become a height.
  expect(unknown.peak).toBeNull();
  expect(unknown.days[0].agent.state).toBe('unknown');
});

it('adds the generated report days back up to the hero totals it splits', () => {
  for (const report of exported.dashboards) {
    const { days, peak } = heroSeries(report.days);
    expect(days).toHaveLength(report.days.length);
    const agent = report.days.reduce((sum, d) => sum + d.agent_hours, 0);
    const human = report.days.reduce((sum, d) => sum + (d.human_hours_est ?? 0), 0);
    expect(agent).toBeCloseTo(report.tiles.agent_hours.value!, 9);
    expect(human).toBeCloseTo(report.tiles.human_hours_est.value!, 9);
    expect(peak).toBeCloseTo(report.tiles.agent_hours.value!, 9);
    expect(days.filter((d) => d.agent.ratio === 1)).toHaveLength(1);
  }
});
