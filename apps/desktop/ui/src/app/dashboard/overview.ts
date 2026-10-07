import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricHumanHoursDay } from '../../data/generated/MetricHumanHoursDay';
import type { MetricPrAnalyticsRow } from '../../data/generated/MetricPrAnalyticsRow';
import type { MetricPrMarker } from '../../data/generated/MetricPrMarker';
import { clock } from '../../kit/clock';
import { agentDuration, agentTime } from '../agent-duration';
import { continuous, handsOffTime } from '../metric-format';
import { dayLabel } from './effort-chart';
import { plural, zoneOf } from './present';

/**
 * Presentation only. Every value here is the Rust report's, each day's
 * leverage included; takeaways only point at the largest value a series
 * already has.
 */

const HOUR_MS = 3_600_000;

/** What the card shows, in place of a rule's technical text. */
export const OVERVIEW_DEFINITION =
  'Four numbers for the selected range. Each change compares with the previous period of the same length; Leverage counts whole days, so its change compares with as many whole days before. Each chart shows the range day by day, every day measured on its own; its dashed line is the number for the whole range.';

export const CONCURRENCY_DEFINITION =
  'How many agent sessions ran at the same time, on average over the time any ran. Max is the most at once.';

export const HANDS_OFF_DEFINITION =
  'The median time an agent worked on its own, with at least one tool call, between your message and its last step before your next message. p90: nine in ten stretches were shorter.';

/** One day of a tile's line: its date and value; `null` leaves a gap. */
export interface DayValue {
  date: string;
  value: number | null;
}

/**
 * The whole local days a number covers, e.g. `Sep 28–Oct 5, whole days`:
 * human time and the agent hours beside them, never the rolling range.
 */
export function wholeDaysText(days: readonly { date: string }[]) {
  if (days.length === 0) return 'whole days';
  const first = dayLabel(days[0]!.date);
  const last = dayLabel(days[days.length - 1]!.date);
  return `${first === last ? first : `${first}–${last}`}, whole days`;
}

/**
 * What a whole-day change compares with: the same number of whole days just
 * before, e.g. `the previous 8 whole days` when a 7-day range starts mid-day.
 * The rolling numbers keep the range's own `vs. previous 7 days`.
 */
export const previousWholeDaysText = (days: readonly { date: string }[]) =>
  `the previous ${days.length} whole ${days.length === 1 ? 'day' : 'days'}`;

/**
 * Leverage's exact span and what its change compares with, for its
 * definition and accessible description, not the tile's face; e.g.
 * `Both sides cover Sep 28–Oct 5, whole days. Its change compares with the previous 8 whole days.`
 */
export const leverageSpanText = (days: readonly { date: string }[]) =>
  days.length === 0
    ? undefined
    : `Both sides cover ${wholeDaysText(days)}. Its change compares with ${previousWholeDaysText(days)}.`;

/**
 * The report's leverage for each whole local day. A day with no human
 * time, or unknown human time, has no value: a gap, never infinity or zero.
 */
export const leverageByDay = (report: DashboardMetrics): DayValue[] =>
  report.leverage.by_day.map((day) => ({ date: day.date, value: day.value }));

export const concurrencyByDay = (report: DashboardMetrics): DayValue[] =>
  report.concurrency_by_day.map((day) => ({ date: day.date, value: day.mean }));

export const handsOffByDay = (report: DashboardMetrics): DayValue[] =>
  report.hands_off_by_day.map((day) => ({ date: day.date, value: day.median_min }));

/** The day with the largest value; the earliest on a tie; none when no day has one. */
export function largest<T extends { date: string }>(
  days: readonly T[],
  value: (day: T) => number | null,
): T | null {
  let best: T | null = null;
  let top = -Infinity;
  for (const day of days) {
    const v = value(day);
    if (v !== null && Number.isFinite(v) && v > top) {
      top = v;
      best = day;
    }
  }
  return best;
}

export function concurrencyTakeaway(report: DashboardMetrics): string | null {
  const best = largest(report.concurrency_by_day, (day) => day.max);
  if (!best || best.max === null) return null;
  return `Most at once on ${dayLabel(best.date)}: ${plural(best.max, 'session')}.`;
}

export function handsOffTakeaway(report: DashboardMetrics): string | null {
  const best = largest(report.hands_off_by_day, (day) => day.median_min);
  if (!best || best.median_min === null) return null;
  return `Longest median on ${dayLabel(best.date)}: ${handsOffTime(best.median_min)}.`;
}

/** One listed merged pull request: the report's marker, with the PRs page's title and hours. */
export interface MergedItem {
  repository: string;
  number: number;
  title: string | null;
  /** Agent hours of its linked sessions; `null` when the PRs page has no row for it. */
  agentMs: number | null;
}

/**
 * The merged pull requests to list, the latest first, at most `limit`. The
 * pull requests are the report's own markers, the same set its Merged PRs
 * number counts; the PRs page's rows only add each one's title and hours.
 */
export function latestMerged(
  markers: readonly MetricPrMarker[],
  rows: readonly MetricPrAnalyticsRow[],
  limit: number,
): MergedItem[] {
  const byId = new Map(rows.map((row) => [`${row.repository}#${row.number}`, row]));
  return [...markers]
    .sort((a, b) => b.merged_at_ms - a.merged_at_ms)
    .slice(0, limit)
    .map((marker) => {
      const row = byId.get(`${marker.repository}#${marker.number}`);
      return {
        repository: marker.repository,
        number: marker.number,
        title: row?.title ?? null,
        agentMs: row?.agent_ms ?? null,
      };
    });
}

/**
 * The Merged PRs takeaway: how many of the counted merged pull requests are
 * listed, then what the PRs page's titles and hours are doing: read, still
 * being read, failed, or not asked for (the browser preview).
 */
export function mergedTakeaway({
  merged,
  shown,
  complete,
  list,
}: {
  merged: number;
  shown: number;
  complete: boolean;
  list: 'read' | 'reading' | 'failed' | 'none';
}): string | null {
  if (merged === 0) return complete ? 'No pull request merged in this range.' : null;
  const latest = `Latest ${shown === merged ? plural(merged, 'merged PR') : `${shown} of ${merged} merged`}${complete ? '' : ' so far'}`;
  switch (list) {
    case 'read':
      return `${latest}, with agent hours on each.`;
    case 'reading':
      return `${latest}; reading their titles and agent hours…`;
    case 'failed':
      return `${latest}; their titles and agent hours could not be read.`;
    case 'none':
      return `${latest}.`;
  }
}

/**
 * Where an instant sits on its local day's clock, in hours from midnight, in
 * the report's zone. `end` places the next midnight at 24, not 0.
 */
export function clockHours(ms: number, window: DashboardWindow, end = false) {
  const parts = new Intl.DateTimeFormat('en-US', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hourCycle: 'h23',
    timeZone: zoneOf(window),
  }).formatToParts(ms);
  const part = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0);
  const hours = part('hour') + part('minute') / 60 + (part('second') + (ms % 1000) / 1000) / 3600;
  return end && hours === 0 ? 24 : hours;
}

/**
 * A time of day from hours after midnight, written as every clock time in the
 * app is (`1:56 PM` or `13:56`, as this Mac prefers). A stretch cut at the
 * next midnight ends at midnight.
 */
export const clockText = (hours: number) =>
  clock(Math.round(hours * 60) * 60_000, { timeZone: 'UTC' });

export interface TimelineBar {
  /** Fractions of the day's 24 hours. */
  left: number;
  width: number;
  /** A single message: drawn as a thin tick. */
  tick: boolean;
  /** e.g. `1:56 PM–5:08 PM`, or `9:42 AM` for a single message. */
  text: string;
  /** Constant-offset pieces of one stretch; one accessible name and tooltip entry. */
  pieces?: { left: number; width: number; lane: 0 | 1 }[];
}
export interface TimelineRow {
  date: string;
  label: string;
  weekend: boolean;
  bars: TimelineBar[];
  /** The day's total, written as all agent time is (`0h00m` for a measured zero); null when unknown. */
  total: string | null;
  /** The row's accessible name. */
  name: string;
  fold?: boolean;
}

/** Offset changes within the native day bounds; at most 48 probes, then millisecond binary search. */
function dayOffsets(day: MetricHumanHoursDay, window: DashboardWindow) {
  const format = new Intl.DateTimeFormat('en-US', {
    timeZone: zoneOf(window),
    timeZoneName: 'longOffset',
    hour: 'numeric',
  });
  const offset = (ms: number) => {
    const text = format.formatToParts(ms).find((part) => part.type === 'timeZoneName')!.value;
    const match = /^GMT([+-])(\d{2}):(\d{2})$/.exec(text);
    return match ? (match[1] === '-' ? -1 : 1) * (Number(match[2]) * 60 + Number(match[3])) : 0;
  };
  const changes: { at: number; before: number; after: number }[] = [];
  let prior = day.start_ms;
  let before = offset(prior);
  for (let probe = 1; probe <= 48; probe++) {
    const next = Math.floor(day.start_ms + ((day.end_ms - day.start_ms) * probe) / 48);
    const after = offset(next);
    if (after !== before) {
      let low = prior,
        high = next;
      while (high - low > 1) {
        const middle = Math.floor((low + high) / 2);
        if (offset(middle) === before) low = middle;
        else high = middle;
      }
      if (high < day.end_ms) changes.push({ at: high, before, after });
    }
    prior = next;
    before = after;
  }
  return { changes, offset };
}

const weekday = (date: string) => {
  const [year, month, day] = date.split('-').map(Number);
  return new Date(Date.UTC(year!, month! - 1, day!)).getUTCDay();
};

/** One 24-hour row per reported local day, oldest first. */
export function timelineRows(
  days: readonly MetricHumanHoursDay[],
  window: DashboardWindow,
): TimelineRow[] {
  return days.map((day) => {
    const { changes, offset } = dayOffsets(day, window);
    const fold = changes.find((change) => change.after < change.before);
    const bars = day.stretches.map((stretch) => {
      const start = clockHours(stretch.start_ms, window);
      const tick = stretch.end_ms === stretch.start_ms;
      const end = tick ? start : clockHours(stretch.end_ms, window, true);
      const bar: TimelineBar = {
        left: start / 24,
        width: (stretch.end_ms - stretch.start_ms) / (24 * HOUR_MS),
        tick,
        text: tick ? clockText(start) : `${clockText(start)}–${clockText(end)}`,
      };
      if (changes.length > 0) {
        const boundaries = [
          stretch.start_ms,
          ...changes
            .map((change) => change.at)
            .filter((at) => at > stretch.start_ms && at < stretch.end_ms),
          stretch.end_ms,
        ];
        bar.pieces = boundaries.slice(0, -1).map((at, index) => ({
          left: clockHours(at, window) / 24,
          width: (boundaries[index + 1]! - at) / (24 * HOUR_MS),
          lane: fold && at >= fold.at ? 1 : 0,
        }));
        if (fold || offset(stretch.start_ms) !== offset(stretch.end_ms)) {
          const zone = (ms: number) =>
            new Intl.DateTimeFormat('en-US', {
              timeZone: zoneOf(window),
              timeZoneName: 'short',
            })
              .formatToParts(ms)
              .find((part) => part.type === 'timeZoneName')!.value;
          bar.text = tick
            ? `${clockText(start)} ${zone(stretch.start_ms)}`
            : `${clockText(start)} ${zone(stretch.start_ms)} → ${clockText(end)} ${zone(stretch.end_ms)} — ${continuous((stretch.end_ms - stretch.start_ms) / 60_000)} min`;
        }
      }
      return bar;
    });
    const total = day.active_ms === null ? null : agentTime(day.active_ms);
    const label = dayLabel(day.date);
    const name =
      day.active_ms === null
        ? `${label}: unknown`
        : bars.length === 0
          ? `${label}: no messages`
          : `${label}: ${day.active_ms > 0 ? agentDuration(day.active_ms).spoken : 'no time between messages'}; ${bars
              .map((bar) => (bar.tick ? `one message at ${bar.text}` : bar.text))
              .join(', ')}`;
    return {
      date: day.date,
      label,
      weekend: [0, 6].includes(weekday(day.date)),
      bars,
      total,
      name,
      ...(fold ? { fold: true } : {}),
    };
  });
}
