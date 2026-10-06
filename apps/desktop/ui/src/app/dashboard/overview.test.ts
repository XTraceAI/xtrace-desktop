// @vitest-environment node
import { expect, it } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { MetricHumanHoursDay } from '../../data/generated/MetricHumanHoursDay';
import type { MetricPrAnalyticsRow } from '../../data/generated/MetricPrAnalyticsRow';
import type { MetricPrMarker } from '../../data/generated/MetricPrMarker';
import {
  clockHours,
  concurrencyTakeaway,
  handsOffTakeaway,
  latestMerged,
  leverageByDay,
  leverageTakeaway,
  mergedTakeaway,
  timelineRows,
} from './overview';

const H = 3_600_000;
const report = () =>
  structuredClone((fixture as FixtureExport).dashboards.find((d) => d.window.days === 7)!);
const utc = { timezone: 'UTC' } as DashboardWindow;
const berlin = { timezone: 'Europe/Berlin' } as DashboardWindow;
const midnight = Date.UTC(2026, 8, 28);

const la = { timezone: 'America/Los_Angeles' } as DashboardWindow;
const instant = (text: string) => Date.parse(text);
function dstDay(
  date: string,
  midnightOffset: string,
  nextMidnight: string,
  stretches: [string, string][],
): MetricHumanHoursDay {
  const rows = stretches.map(([start, end]) => ({
    start_ms: instant(start),
    end_ms: instant(end),
  }));
  return {
    date,
    start_ms: instant(midnightOffset),
    end_ms: instant(nextMidnight),
    active_ms: rows.reduce((sum, row) => sum + row.end_ms - row.start_ms, 0),
    stretches: rows,
  };
}
const fall = (stretches: [string, string][]) =>
  dstDay('2026-11-01', '2026-11-01T00:00:00-07:00', '2026-11-02T00:00:00-08:00', stretches);
const spring = (stretches: [string, string][]) =>
  dstDay('2026-03-08', '2026-03-08T00:00:00-08:00', '2026-03-09T00:00:00-07:00', stretches);

it('draws a twenty-minute fall fold as two ten-minute pieces with one name and unchanged hours', () => {
  const day = fall([['2026-11-01T01:50:00-07:00', '2026-11-01T01:10:00-08:00']]);
  const before = structuredClone(day);
  const [row] = timelineRows([day], la);
  expect(row!.fold).toBe(true);
  expect(row!.bars).toHaveLength(1);
  const bar = row!.bars[0]!;
  expect(bar.text).toBe('01:50 PDT → 01:10 PST — 20 min');
  expect(bar.pieces).toHaveLength(2);
  expect(bar.pieces!.map((piece) => piece.lane)).toEqual([0, 1]);
  expect(bar.pieces![0]!.left * 24).toBeCloseTo(1 + 50 / 60);
  expect(bar.pieces![1]!.left * 24).toBe(1);
  for (const piece of bar.pieces!) expect(piece.width * 24 * 60).toBeCloseTo(10);
  expect(row!.total).toBe('0.3 h');
  expect(day).toEqual(before);
});

it('keeps long overlapping fold pieces and independent repeated-hour messages on distinct tracks', () => {
  const [row] = timelineRows(
    [
      fall([
        ['2026-11-01T00:30:00-07:00', '2026-11-01T02:30:00-08:00'],
        ['2026-11-01T01:15:00-08:00', '2026-11-01T01:15:00-08:00'],
      ]),
    ],
    la,
  );
  const [first, second] = row!.bars[0]!.pieces!;
  expect(first!.left * 24).toBe(0.5);
  expect(first!.width * 24).toBe(1.5);
  expect(second!.left * 24).toBe(1);
  expect(second!.width * 24).toBe(1.5);
  expect([first!.lane, second!.lane]).toEqual([0, 1]);
  expect(row!.bars[1]!.tick).toBe(true);
  expect(row!.bars[1]!.pieces![0]!.lane).toBe(1);
  expect(row!.bars[1]!.pieces![0]!.width).toBe(0);
  expect(row!.bars[1]!.text).toBe('01:15 PST');
  expect(row!.total).toBe('3 h');
});

it('leaves spring missing hours empty and keeps elapsed widths at an exact transition endpoint', () => {
  const [row] = timelineRows(
    [
      spring([
        ['2026-03-08T01:50:00-08:00', '2026-03-08T03:10:00-07:00'],
        ['2026-03-08T01:50:00-08:00', '2026-03-08T03:00:00-07:00'],
      ]),
    ],
    la,
  );
  expect(row!.fold).toBeUndefined();
  const [before, after] = row!.bars[0]!.pieces!;
  expect(before!.left * 24 + before!.width * 24).toBeCloseTo(2);
  expect(after!.left * 24).toBe(3);
  expect(after!.width * 24 * 60).toBeCloseTo(10);
  expect(row!.bars[1]!.pieces).toHaveLength(1);
  expect(row!.bars[1]!.pieces![0]!.width * 24 * 60).toBeCloseTo(10);
  expect(row!.bars[0]!.text).toBe('01:50 PST → 03:10 PDT — 20 min');
});

it('ends the first fold track at its old clock and preserves midnight cuts', () => {
  const [row] = timelineRows(
    [
      fall([
        ['2026-11-01T01:50:00-07:00', '2026-11-01T01:00:00-08:00'],
        ['2026-11-01T23:00:00-08:00', '2026-11-02T00:00:00-08:00'],
      ]),
    ],
    la,
  );
  const end = row!.bars[0]!.pieces![0]!;
  expect(end.left * 24 + end.width * 24).toBeCloseTo(2);
  expect(row!.bars[0]!.pieces).toHaveLength(1);
  expect(row!.bars[1]!.pieces![0]!.left * 24).toBe(23);
  expect(row!.bars[1]!.pieces![0]!.width * 24).toBe(1);
  expect(row!.bars[1]!.text).toContain('24:00 PST');
});

it('draws the report’s own leverage for each day, with a gap where it has none', () => {
  const r: DashboardMetrics = report();
  r.leverage.by_day = r.leverage.by_day.slice(0, 3).map((day, index) => ({
    ...day,
    agent_ms: [6, 4, 2][index]! * H,
    // 2 h, none, unknown
    human_ms: [2 * H, 0, null][index]!,
    value: [3, null, null][index]!,
  }));
  expect(leverageByDay(r)).toEqual([
    { date: r.leverage.by_day[0]!.date, value: 3, yoursMs: 2 * H },
    { date: r.leverage.by_day[1]!.date, value: null, yoursMs: 0 },
    { date: r.leverage.by_day[2]!.date, value: null, yoursMs: null },
  ]);
  expect(leverageTakeaway(leverageByDay(r))).toBe('Highest on Sep 1: 3 agent h for each of yours.');
  expect(leverageTakeaway([{ date: '2026-09-01', value: null, yoursMs: null }])).toBeNull();
});

it('names the highest-leverage day only among days with at least half an hour of yours', () => {
  const day = (date: string, value: number, minutes: number) => ({
    date,
    value,
    yoursMs: minutes * 60_000,
  });
  // Five minutes of yours and 4 agent hours is 48×: not a takeaway.
  expect(
    leverageTakeaway([
      day('2026-09-01', 48, 5),
      day('2026-09-02', 6, 30),
      day('2026-09-03', 4, 90),
    ]),
  ).toBe('Highest on Sep 2: 6 agent h for each of yours.');
  expect(leverageTakeaway([day('2026-09-01', 48, 5), day('2026-09-02', 20, 29)])).toBeNull();
});

it('names the day with the most sessions at once and the longest hands-off median', () => {
  const r: DashboardMetrics = report();
  const day = (date: string) => ({ date, start_ms: 0, end_ms: 1 });
  r.concurrency_by_day = [
    { ...day('2026-09-29'), max: 3, mean: 1.5 },
    { ...day('2026-09-30'), max: 14, mean: 3.9 },
    { ...day('2026-10-01'), max: 14, mean: 2 },
    { ...day('2026-10-02'), max: null, mean: null },
  ];
  r.hands_off_by_day = [
    { ...day('2026-09-29'), n: 4, median_min: 2.25, p90_min: 9 },
    { ...day('2026-09-30'), n: null, median_min: null, p90_min: null },
  ];
  // A tie keeps the earlier day.
  expect(concurrencyTakeaway(r)).toBe('Most at once on Sep 30: 14 sessions.');
  expect(handsOffTakeaway(r)).toBe('Longest median on Sep 29: 2.3 min.');
  r.concurrency_by_day = [{ ...day('2026-09-29'), max: null, mean: null }];
  r.hands_off_by_day = [];
  expect(concurrencyTakeaway(r)).toBeNull();
  expect(handsOffTakeaway(r)).toBeNull();
});

it('places a stretch on its local clock, and a piece cut at midnight ends at 24:00', () => {
  expect(clockHours(midnight + 13.5 * H, utc)).toBe(13.5);
  expect(clockHours(midnight + 24 * H, utc, true)).toBe(24);
  expect(clockHours(midnight + 24 * H, utc)).toBe(0);
  // 22:00 UTC is midnight in Berlin in September.
  expect(clockHours(midnight + 22 * H, berlin, true)).toBe(24);
  const day: MetricHumanHoursDay = {
    date: '2026-09-28',
    start_ms: midnight,
    end_ms: midnight + 24 * H,
    active_ms: 1.5 * H,
    stretches: [
      { start_ms: midnight + 9 * H, end_ms: midnight + 9 * H },
      { start_ms: midnight + 22.5 * H, end_ms: midnight + 24 * H },
    ],
  };
  const [row] = timelineRows([day], utc);
  expect(row!.label).toBe('Sep 28');
  expect(row!.weekend).toBe(false);
  expect(row!.total).toBe('1.5 h');
  expect(row!.bars).toEqual([
    { left: 9 / 24, width: 0, tick: true, text: '09:00' },
    { left: 22.5 / 24, width: 1.5 / 24, tick: false, text: '22:30–24:00' },
  ]);
  expect(row!.name).toBe('Sep 28: 1.5 h; one message at 09:00, 22:30–24:00');
});

it('says a day with no time, no messages or unknown classification as such', () => {
  const base = { start_ms: midnight, end_ms: midnight + 24 * H };
  const rows = timelineRows(
    [
      { ...base, date: '2026-10-03', active_ms: 0, stretches: [] },
      {
        ...base,
        date: '2026-10-04',
        active_ms: 0,
        stretches: [{ start_ms: midnight + H, end_ms: midnight + H }],
      },
      { ...base, date: '2026-10-05', active_ms: null, stretches: [] },
    ],
    utc,
  );
  expect(rows.map((row) => [row.weekend, row.total, row.name])).toEqual([
    [true, '–', 'Oct 3: no messages'],
    [true, '–', 'Oct 4: no time between messages; one message at 01:00'],
    [false, null, 'Oct 5: unknown'],
  ]);
});

it('lists the report’s own merged pull requests, latest first, with the PRs page’s titles and hours', () => {
  const marker = (number: number, merged_at_ms: number) =>
    ({ repository: 'example/app', number, merged_at_ms }) as MetricPrMarker;
  const row = (number: number, title: string, agent_ms: number) =>
    ({ repository: 'example/app', number, title, agent_ms }) as MetricPrAnalyticsRow;
  // The page read later also has #4, merged after the report was read: it is
  // not listed, since the number does not count it.
  const listed = latestMerged(
    [marker(1, 5), marker(2, 9), marker(3, 7)],
    [row(2, 'Two', 2 * H), row(4, 'Four', H)],
    2,
  );
  expect(listed).toEqual([
    { repository: 'example/app', number: 2, title: 'Two', agentMs: 2 * H },
    { repository: 'example/app', number: 3, title: null, agentMs: null },
  ]);
});

it('says what the Merged PRs list shows, without promising hours it has not read', () => {
  const base = { merged: 12, shown: 4, complete: true };
  expect(mergedTakeaway({ ...base, list: 'read' })).toBe(
    'Latest 4 of 12 merged, with agent hours on each.',
  );
  expect(mergedTakeaway({ ...base, list: 'reading' })).toBe(
    'Latest 4 of 12 merged; reading their titles and agent hours…',
  );
  expect(mergedTakeaway({ ...base, list: 'failed' })).toBe(
    'Latest 4 of 12 merged; their titles and agent hours could not be read.',
  );
  expect(mergedTakeaway({ ...base, list: 'none' })).toBe('Latest 4 of 12 merged.');
  expect(mergedTakeaway({ merged: 2, shown: 2, complete: false, list: 'read' })).toBe(
    'Latest 2 merged PRs so far, with agent hours on each.',
  );
  expect(mergedTakeaway({ merged: 0, shown: 0, complete: true, list: 'reading' })).toBe(
    'No pull request merged in this range.',
  );
  expect(mergedTakeaway({ merged: 0, shown: 0, complete: false, list: 'read' })).toBeNull();
});
