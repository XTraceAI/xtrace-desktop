import type { MetricEffortDay } from '../../data/generated/MetricEffortDay';
import type { MetricModelDayEffort } from '../../data/generated/MetricModelDayEffort';
import type { MetricPrEffort } from '../../data/generated/MetricPrEffort';
import type { MetricPrMarker } from '../../data/generated/MetricPrMarker';
import { agentHours, costCell, markerFreshnessText, type EffortMetric } from './pr-effort';
import { plural, usd } from './present';

/**
 * The geometry of the M-19 chart, derived from the report and nothing else:
 * one bar per local day for the whole cohort's total of the chosen measure,
 * every day on one shared scale from zero, and one date-to-x mapping that the
 * bars, the date ticks and the merge markers all position from. Each day's
 * models come from the report's own per-model split, which adds up to the
 * day exactly. Every value here is the report's; the only computation is
 * where to draw it and how to say it.
 */

/** Where a day sits across the axis, as a fraction of its width: day centres. */
export const dayCenter = (index: number, count: number) => (index + 0.5) / count;

/**
 * Which days carry a date label: every day of a week, every other day of a
 * fortnight and every fifth day of a month, counted back from the last day
 * so the most recent day is always labelled, plus the first day when it is
 * far enough from the nearest label to stay legible.
 */
export function tickIndices(count: number): number[] {
  if (count <= 0) return [];
  const step = count <= 7 ? 1 : count <= 14 ? 2 : 5;
  const ticks: number[] = [];
  for (let index = count - 1; index >= 0; index -= step) ticks.unshift(index);
  if (ticks[0] !== 0 && ticks[0]! >= 2) ticks.unshift(0);
  return ticks;
}

const HOUR_MS = 3_600_000;
/** One day of clock time: a day above it had agents running at the same time. */
export const FULL_DAY_HOURS = 24;
/** The 24 h line is drawn once the busiest day reaches half of it. */
const REFERENCE_FROM_HOURS = 12;
export const OVER_FULL_DAY = 'Above 24 h: agents ran at the same time';
export const NO_MODEL = 'no model recorded';

export type BarState = 'measured' | 'none' | 'unknown';

/** One model's part of a day, in the measure's unit. */
export interface ModelPart {
  name: string;
  value: number;
  /** The value in words: `12.3 h` or `$4.56`. */
  text: string;
  /** Its share of the day's plotted total: `42%`, or `<1%`. */
  share: string;
}
export interface UnpricedPart {
  name: string;
  responses: number;
}
export interface EffortBar {
  index: number;
  date: string;
  /** The day as the hover card names it, e.g. `Sep 14`. */
  label: string;
  /** `none`: nothing to draw; `unknown`: usage that not one response could price. */
  state: BarState;
  /** Hours or priced dollars; null unless measured. */
  value: number | null;
  /** Height as a fraction of the scale's top. */
  fraction: number;
  /** A priced subtotal that leaves some of the day's responses unpriced. */
  partial: boolean;
  /** The day's total in words, with `+` when partial. */
  totalText: string;
  /** Every model with a value on the day, largest first. */
  models: ModelPart[];
  /** Cost only: responses the catalog could not price, by model. */
  unpriced: UnpricedPart[];
  /** Hours only: more agent time than the day has hours. */
  overFullDay: boolean;
  merged: MetricPrMarker[];
  /** The day's accessible name: total, the largest models, notes and merges. */
  name: string;
}
export interface DayMarkers {
  index: number;
  date: string;
  merged: MetricPrMarker[];
  /** The count, then every pull request's identity, type, link confidence and freshness. */
  text: string;
}
export interface EffortHeadline {
  /** The range's total, e.g. `$14,066+` or `655 agent h`. */
  text: string;
  /** e.g. `last 30 days`. */
  range: string;
  /** e.g. `1,965 responses have no price` or `10 days above 24 h`; null when nothing to add. */
  note: string | null;
}
export interface EffortChart {
  metric: EffortMetric;
  /** The unit every plotted value is stated in. */
  unit: 'hours' | 'dollars';
  days: readonly MetricEffortDay[];
  bars: EffortBar[];
  /** The scale's top and middle; null when nothing above zero is measured. */
  scale: { top: number; topText: string; midText: string } | null;
  /** The 24 h line, as a fraction of the scale's top; hours only. */
  reference: { fraction: number; text: string | null } | null;
  /** Why the plot has nothing to draw, when no day has a measured value. */
  unmeasured: string | null;
  headline: EffortHeadline;
  ticks: number[];
  markers: DayMarkers[];
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
/** `2026-09-14` → `Sep 14`. */
export const dayLabel = (date: string) => {
  const [, month, day] = date.split('-');
  return `${MONTHS[Number(month) - 1] ?? month} ${Number(day)}`;
};

export const markerText = (marker: MetricPrMarker) =>
  `${marker.repository}#${marker.number} · ${marker.work_type ?? 'type unresolved'} · ${marker.confidence} link · ${markerFreshnessText(marker.freshness)}`;

/** Merge markers keyed by local merge day. */
export function markersByDay(markers: readonly MetricPrMarker[]) {
  const byDay = new Map<string, MetricPrMarker[]>();
  for (const marker of markers) byDay.set(marker.date, [...(byDay.get(marker.date) ?? []), marker]);
  return byDay;
}

/** Round tops whose halves are round too, per power of ten. */
const ROUND_TOPS = [1, 1.2, 1.6, 2, 3, 4, 5, 6, 8, 10];
/** The smallest round value at or above `max`: the scale's top, its middle half of it. */
export function niceTop(max: number) {
  const magnitude = 10 ** Math.floor(Math.log10(max));
  const factor = ROUND_TOPS.find((top) => top * magnitude >= max - 1e-9 * magnitude)!;
  return Math.round(factor * magnitude * 1e9) / 1e9;
}

/** Three significant digits: every round top and its half keep their own digits. */
const trimmed = (value: number) => String(Number(value.toPrecision(3)));
/**
 * The unit the agent-time scale is labelled in, from the busiest day: hours
 * from 0.1 h, then minutes from one minute, then seconds, so a short range's
 * labels name the values the chart reaches instead of rounding to `0 h`.
 */
export type TimeUnit = { factor: number; suffix: 'h' | 'min' | 's' };
export function timeUnit(maxHours: number): TimeUnit {
  if (maxHours >= 0.1) return { factor: 1, suffix: 'h' };
  if (maxHours * 60 >= 1) return { factor: 60, suffix: 'min' };
  return { factor: 3600, suffix: 's' };
}
const HOURS: TimeUnit = { factor: 1, suffix: 'h' };
/** A scale label: `24 h`, `6 min`, `12 s`, `$400`, `$1.5k`, `$0.003`. */
export function scaleText(value: number, metric: EffortMetric, unit: TimeUnit = HOURS) {
  if (metric === 'agent') return `${trimmed(value * unit.factor)} ${unit.suffix}`;
  return value >= 1000 ? `$${trimmed(value / 1000)}k` : `$${trimmed(value)}`;
}

const wholeUsd = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 0,
});
/** The headline's dollars: whole from $100, cents below. */
const bigUsd = (value: number) => (value >= 100 ? wholeUsd.format(value) : usd(value));
/** The headline's hours: whole from 10 h, one decimal below. */
const bigHours = (hours: number) =>
  hours >= 10
    ? Math.round(hours).toLocaleString('en-US')
    : hours > 0 && hours < 0.05
      ? '<0.1'
      : (Math.round(hours * 10) / 10).toString();

const modelName = (model: MetricModelDayEffort) => model.model ?? NO_MODEL;

function shareText(value: number, total: number) {
  if (total <= 0) return '—';
  const pct = (value / total) * 100;
  return value > 0 && pct < 0.5 ? '<1%' : `${Math.round(pct)}%`;
}

/** Every model with a value on the day in this measure, largest first. */
function modelParts(day: MetricEffortDay, metric: EffortMetric, total: number): ModelPart[] {
  return day.models
    .filter((model) => (metric === 'agent' ? model.agent_ms > 0 : model.priced_observations > 0))
    .map((model) => {
      const value =
        metric === 'agent' ? model.agent_ms / HOUR_MS : model.priced_nano_usd / 1_000_000_000;
      return {
        name: modelName(model),
        value,
        text: metric === 'agent' ? agentHours(model.agent_ms) : usd(value),
        share: shareText(value, total),
      };
    })
    .sort((a, b) => b.value - a.value || a.name.localeCompare(b.name));
}

/** `No price: 128 codex-auto-review responses`. */
export function unpricedLine(unpriced: readonly UnpricedPart[]) {
  const count = unpriced.reduce((sum, part) => sum + part.responses, 0);
  return `No price: ${unpriced
    .map((part) => `${part.responses.toLocaleString('en-US')} ${part.name}`)
    .join(', ')} ${count === 1 ? 'response' : 'responses'}`;
}

/** How many of a day's models its accessible name lists before summarising the rest. */
const NAMED_MODELS = 3;

function barName(bar: Omit<EffortBar, 'name'>) {
  const parts = [`${bar.date}: ${bar.totalText}`];
  if (bar.models.length > 0) {
    const named = bar.models
      .slice(0, NAMED_MODELS)
      .map((model) => `${model.name} ${model.text} (${model.share})`);
    const rest = bar.models.length - NAMED_MODELS;
    parts.push(
      rest > 0 ? `${named.join(', ')} and ${plural(rest, 'more model')}` : named.join(', '),
    );
  }
  if (bar.unpriced.length > 0) parts.push(unpricedLine(bar.unpriced));
  if (bar.overFullDay) parts.push(OVER_FULL_DAY);
  if (bar.merged.length > 0)
    parts.push(`merged ${bar.merged.map((marker) => `#${marker.number}`).join(', ')}`);
  return parts.join('. ');
}

export function effortChart(current: MetricPrEffort, metric: EffortMetric): EffortChart {
  const days = current.cohort.by_day;
  const count = days.length;
  const byDay = markersByDay(current.markers);

  const bare = days.map((day, index) => {
    const merged = byDay.get(day.date) ?? [];
    if (metric === 'agent') {
      const value = day.agent_ms / HOUR_MS;
      const measured = day.agent_ms > 0;
      return {
        index,
        date: day.date,
        label: dayLabel(day.date),
        state: (measured ? 'measured' : 'none') as BarState,
        value: measured ? value : null,
        partial: false,
        totalText: measured ? agentHours(day.agent_ms) : 'no agent time',
        models: modelParts(day, metric, value),
        unpriced: [],
        overFullDay: value > FULL_DAY_HOURS,
        merged,
      };
    }
    const cell = costCell(day.cost);
    const partial = cell.state === 'measured' && day.cost.unpriced_observations > 0;
    const unpriced = day.models
      .filter((model) => model.unpriced_observations > 0)
      .map((model) => ({ name: modelName(model), responses: model.unpriced_observations }))
      .sort((a, b) => b.responses - a.responses || a.name.localeCompare(b.name));
    return {
      index,
      date: day.date,
      label: dayLabel(day.date),
      state: cell.state,
      value: cell.value,
      partial,
      totalText:
        cell.state === 'measured'
          ? `${usd(cell.value)}${partial ? '+' : ''}`
          : cell.state === 'none'
            ? 'no usage'
            : 'cost unknown',
      models: modelParts(day, metric, cell.value ?? 0),
      unpriced,
      overFullDay: false,
      merged,
    };
  });

  // Display scale only: a round value at or above the busiest day, and in
  // hours at least one full day once the busiest day reaches half of one.
  const max = bare.reduce<number | null>(
    (top, bar) => (bar.value === null ? top : Math.max(top ?? 0, bar.value)),
    null,
  );
  const showReference = metric === 'agent' && max !== null && max >= REFERENCE_FROM_HOURS;
  // The round top is chosen in the unit the labels state, then kept in hours.
  const unit = metric === 'agent' && max !== null && max > 0 ? timeUnit(max) : HOURS;
  const top =
    max !== null && max > 0
      ? showReference
        ? Math.max(niceTop(max), FULL_DAY_HOURS)
        : niceTop(max * unit.factor) / unit.factor
      : null;

  const bars: EffortBar[] = bare.map((bar) => {
    const withFraction = {
      ...bar,
      fraction: top !== null && bar.value !== null ? Math.min(1, bar.value / top) : 0,
    };
    return { ...withFraction, name: barName(withFraction) };
  });

  // Stated neutrally: the days may be unknown or without usage in any mix,
  // and each day's own name says which.
  const worked = current.cohort.sessions > 0;
  const unmeasured =
    max !== null || !worked
      ? null
      : metric === 'dollars'
        ? bars.some((bar) => bar.state === 'unknown')
          ? 'No priced daily usage to plot.'
          : null
        : 'No agent time in this range.';

  const markers: DayMarkers[] = [];
  days.forEach(({ date }, index) => {
    const merged = byDay.get(date);
    if (!merged || merged.length === 0) return;
    markers.push({
      index,
      date,
      merged,
      text: `${date}: ${plural(merged.length, 'merged pull request')} · ${merged.map(markerText).join('; ')}`,
    });
  });

  return {
    metric,
    unit: metric === 'agent' ? 'hours' : 'dollars',
    days,
    bars,
    scale:
      top !== null
        ? {
            top,
            topText: scaleText(top, metric, unit),
            midText: scaleText(top / 2, metric, unit),
          }
        : null,
    // On the top line the scale's own label already names it: no second one.
    reference:
      showReference && top !== null
        ? {
            fraction: FULL_DAY_HOURS / top,
            text: top === FULL_DAY_HOURS ? null : `${FULL_DAY_HOURS} h`,
          }
        : null,
    unmeasured,
    headline: headline(current, metric, bars),
    ticks: tickIndices(count),
    markers,
  };
}

function headline(
  current: MetricPrEffort,
  metric: EffortMetric,
  bars: EffortBar[],
): EffortHeadline {
  const cohort = current.cohort;
  const range = `last ${plural(cohort.by_day.length, 'day')}`;
  if (metric === 'agent') {
    const over = bars.filter((bar) => bar.overFullDay).length;
    return {
      text: `${bigHours(cohort.agent_ms / HOUR_MS)} agent h`,
      range,
      note: over > 0 ? `${plural(over, 'day')} above 24 h` : null,
    };
  }
  const cell = costCell(cohort.cost);
  const unpriced = cohort.cost.unpriced_observations;
  return {
    text:
      cell.state === 'measured'
        ? `${bigUsd(cell.value)}${unpriced > 0 ? '+' : ''}`
        : cell.state === 'none'
          ? 'no usage'
          : 'cost unknown',
    range,
    note:
      unpriced > 0
        ? `${plural(unpriced, 'response')} ${unpriced === 1 ? 'has' : 'have'} no price`
        : null,
  };
}
