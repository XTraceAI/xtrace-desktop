import { Tooltip } from '@base-ui/react/tooltip';
import { useEffect, useId, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react';
import type { AccountUsage } from '../data/generated/AccountUsage';
import type { AccountProviderUsage } from '../data/generated/AccountProviderUsage';
import type { AccountUsageIssue } from '../data/generated/AccountUsageIssue';
import type { AccountUsageDay } from '../data/generated/AccountUsageDay';
import type { AccountUsageWindow } from '../data/generated/AccountUsageWindow';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { clock } from './clock';
import { UNMEASURED } from './format';
import { HostGlyph } from './HostGlyph';
import { Icon } from './icons';
import '../styles/metrics.css';

const issueLabels: Record<AccountUsageIssue, string> = {
  no_login: 'No account login found',
  credential_access_required: 'Account access required',
  credential_unavailable: 'Account login unavailable',
  unauthorized: 'Account login expired',
  rate_limited: 'Provider rate limited',
  timeout: 'Provider timed out',
  network: 'Provider connection failed',
  invalid_response: 'Provider response unavailable',
  source_unavailable: 'Usage source unavailable',
  not_fetched: 'Refresh to load usage',
  reading: 'Reading…',
};

/**
 * The percent remaining as shown everywhere (header, rows and chart labels).
 * Do not round a small positive balance into a false zero, nor a small use
 * into a false 100%.
 */
export function remainingPercent(usedPercent: number): string {
  if (usedPercent === 0) return '100%';
  if (usedPercent === 100) return '0%';
  const remaining = 100 - usedPercent;
  // Absorb only subtraction noise before checking the tenth-percent boundary.
  const normalized = remaining + Number.EPSILON * 100;
  if (normalized < 0.1) return '<0.1%';
  const shown = Math.min(99.9, Math.floor(normalized * 10) / 10);
  return `${Number.isInteger(shown) ? shown : shown.toFixed(1)}%`;
}

export function remainingLabel(usedPercent: number): string {
  return `${remainingPercent(usedPercent)} remaining`;
}

/**
 * Whether a reading is stale: the app says so, or the time the app gave for
 * it to turn stale has come. The app decides both from one rule.
 */
export function isStale(provider: AccountProviderUsage, now: number = Date.now()): boolean {
  return (
    provider.state === 'stale' ||
    (provider.stale_at !== undefined &&
      Number.isFinite(provider.stale_at) &&
      now >= provider.stale_at * 1000)
  );
}

export function limitingWindow(windows: AccountUsageWindow[]): AccountUsageWindow | undefined {
  const valid = windows.filter(
    (window) =>
      Number.isFinite(window.used_percent) &&
      window.used_percent >= 0 &&
      window.used_percent <= 100,
  );
  const allModels = valid.filter((window) => window.scope === 'all_models');
  return (allModels.length ? allModels : valid).reduce<AccountUsageWindow | undefined>(
    (mostUsed, window) =>
      !mostUsed || window.used_percent > mostUsed.used_percent ? window : mostUsed,
    undefined,
  );
}

const minuteMs = 60_000;

const windowExpired = (window: AccountUsageWindow) =>
  window.resets_at !== null && window.resets_at * 1000 <= Date.now();

function localDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/**
 * A short local time: "today 1:10 PM", "Mon 8 PM" within six days, else
 * "Oct 12". No time-zone name; ":00" is left out.
 */
export function shortTime(seconds: number, now: number = Date.now()): string | null {
  if (!Number.isFinite(seconds)) return null;
  const exact = new Date(seconds * 1000);
  if (Number.isNaN(exact.getTime())) return null;
  // Provider instants can land just before a minute boundary. Round only the
  // displayed time; expiry checks keep the provider's exact instant.
  const date = new Date(Math.round(exact.getTime() / minuteMs) * minuteMs);
  const ms = date.getTime();
  // The app's one clock format; ":00" is dropped only from a 12-hour clock
  // ("8 PM"), and "20:00" stays whole.
  const days = Math.round((localDay(date) - localDay(new Date(now))) / 86_400_000);
  if (days === 0) return `today ${clock(ms, { shortHour: true })}`;
  if (Math.abs(days) <= 6) return clock(ms, { date: 'weekday', shortHour: true });
  return clock(ms, { date: 'day', time: false });
}

export function resetLabel(seconds: number | null): string {
  const when = seconds === null ? null : shortTime(seconds);
  return when ? `resets ${when}` : 'reset time unknown';
}

function windowTitle(window: AccountUsageWindow): string {
  return window.scope === 'all_models' ? window.window : `${window.name} · ${window.window}`;
}

/** The reading's age, shown only once it is 5 minutes old or stale. */
export function ageLabel(seconds: number | null, stale: boolean): string | null {
  if (seconds === null || !Number.isFinite(seconds)) return null;
  const milliseconds = seconds * 1000;
  if (!Number.isFinite(milliseconds) || Number.isNaN(new Date(milliseconds).getTime())) return null;
  const ageMinutes = Math.floor((Date.now() - milliseconds) / minuteMs);
  if (ageMinutes < 0) return 'read time uncertain';
  if (ageMinutes < 5 && !stale) return null;
  if (ageMinutes < 1) return 'just now';
  if (ageMinutes < 60) return `${ageMinutes} min ago`;
  if (ageMinutes < 1440) return `${Math.floor(ageMinutes / 60)} hr ago`;
  const days = Math.floor(ageMinutes / 1440);
  return `${days} ${days === 1 ? 'day' : 'days'} ago`;
}

/**
 * "Runs out ~Sat 3 PM" (a warning) when use is ahead of even use, "Behind
 * pace · ~68% used by reset" when it is behind, or "On pace · …" when it is
 * exactly even: the same comparison the tick and the bar color use.
 */
export function paceLabel(
  window: AccountUsageWindow,
  prefix = '',
): { text: string; warning: boolean } | null {
  const pace = window.pace;
  if (!pace || windowExpired(window)) return null;
  // A stale reading can project a run-out time that has already passed:
  // say nothing rather than "Runs out" in the past.
  if (pace.run_out_at !== null && pace.run_out_at * 1000 <= Date.now()) return null;
  const runOut = pace.run_out_at === null ? null : shortTime(pace.run_out_at);
  if (runOut)
    return { text: `${prefix ? `${prefix} runs` : 'Runs'} out ~${runOut}`, warning: true };
  if (!Number.isFinite(pace.projected_percent_at_reset)) return null;
  const projected = projectedUsed(pace.projected_percent_at_reset);
  const where = window.used_percent < pace.expected_percent ? 'behind' : 'on';
  const lead = prefix ? `${prefix} ${where}` : where[0].toUpperCase() + where.slice(1);
  return {
    text: `${lead} pace · ~${projected}% used by reset`,
    warning: false,
  };
}

/**
 * How far use is from even use at the reading's time: "4% ahead of pace"
 * (used faster than even), "3% behind pace", "<1% ahead of pace" for a gap
 * under a point, or "On pace" only when exactly even. Ahead is exactly when
 * the bar is orange and the app gives a run-out time.
 */
export function paceGapLabel(window: AccountUsageWindow): string | null {
  const expected = window.pace?.expected_percent;
  if (expected === undefined || !Number.isFinite(expected)) return null;
  const gap = window.used_percent - expected;
  if (!Number.isFinite(gap)) return null;
  if (gap === 0) return 'On pace';
  const size = Math.abs(gap) < 1 ? '<1%' : `${Math.round(Math.abs(gap))}%`;
  return `${size} ${gap > 0 ? 'ahead of' : 'behind'} pace`;
}

const clampPercent = (value: number) => Math.min(100, Math.max(0, value));

/**
 * The projected percent used at the reset, rounded once: "~50% used by reset"
 * and the screen reader's "about 50% left" (100 minus it) read this one value.
 */
const projectedUsed = (projected: number) => clampPercent(Math.round(projected));

/**
 * The summary bar's fill against the even-use mark. A bar that stops short of
 * the mark (less left than even use would leave) is orange; past the mark,
 * the extra length is green.
 */
export function paceFill(
  usedPercent: number,
  expectedPercent: number | undefined,
): { className?: string; split?: string } {
  if (expectedPercent === undefined || !Number.isFinite(expectedPercent)) return {};
  const remaining = 100 - usedPercent;
  const mark = 100 - clampPercent(expectedPercent);
  if (remaining < mark) return { className: 'is-behind-pace' };
  if (remaining > mark)
    return { className: 'is-ahead-of-pace', split: `${(mark / remaining) * 100}%` };
  return {};
}

/** The tick does nothing itself: keep a click or key press from toggling the row. */
function holdRow(event: MouseEvent | KeyboardEvent) {
  if ('key' in event && event.key !== 'Enter' && event.key !== ' ') return;
  event.preventDefault();
  event.stopPropagation();
}

/**
 * A mark on the main bar where even use would leave it at the reading's time.
 * The bar fills with what remains, so even use sits at 100% minus the share
 * of the window passed. Hover or focus says how far use is from it.
 */
function PaceTick({ window }: { window: AccountUsageWindow }) {
  const id = useId();
  const theme = useSurfaceTheme();
  const text = paceGapLabel(window);
  if (!text || !window.pace) return null;
  const left = 100 - clampPercent(window.pace.expected_percent);
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={
          <button
            type="button"
            // WebKit skips buttons on Tab unless they say otherwise.
            tabIndex={0}
            className="xt-account-pace-tick"
            style={{ left: `${left}%` }}
            aria-label={`Even use mark: ${text}`}
            onClick={holdRow}
            onKeyDown={holdRow}
            onKeyUp={holdRow}
          />
        }
        delay={0}
        closeOnClick={false}
      />
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="top"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover xt-account-tick-tip">
            {text}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/**
 * The burndown's drawing area, in viewBox units. The plot runs from `left`
 * to `right` and from `top` (100% remaining) to `bottom` (0% remaining); the
 * gutter on the left holds the 100% and 0% labels, the strip below holds
 * the day labels.
 */
const chart = { width: 200, height: 72, left: 32, right: 196, top: 6, bottom: 56 } as const;

/**
 * The readings form a line worth drawing only when there are at least two
 * and they span at least 2% of the window; a shorter span would be a dot or
 * a sliver that the legend's "Remaining" line does not match.
 */
export function readingsSpanVisible(
  points: { at: number }[],
  start: number,
  reset: number,
): boolean {
  if (points.length < 2 || !(reset > start)) return false;
  return (points[points.length - 1].at - points[0].at) / (reset - start) >= 0.02;
}

/** Local midnights strictly inside the window, as Unix seconds. */
function localMidnights(start: number, reset: number): number[] {
  const first = new Date(start * 1000);
  if (Number.isNaN(first.getTime())) return [];
  const midnights: number[] = [];
  for (let day = 1; day <= 62; day++) {
    const next =
      new Date(first.getFullYear(), first.getMonth(), first.getDate() + day).getTime() / 1000;
    if (next >= reset) break;
    if (next > start) midnights.push(next);
  }
  return midnights;
}

/** One sentence for screen readers that says what the burndown shows. */
export function burndownSummary(window: AccountUsageWindow): string {
  const parts = [`${remainingLabel(window.used_percent)}.`];
  const pace = paceLabel(window);
  const expected = window.pace?.expected_percent;
  if (pace && expected !== undefined && Number.isFinite(expected))
    parts.push(`Even use would leave ${Math.round(100 - clampPercent(expected))}%.`);
  const runOut =
    pace?.warning && window.pace?.run_out_at != null ? shortTime(window.pace.run_out_at) : null;
  if (runOut) parts.push(`At this pace it runs out ${runOut}.`);
  else if (pace && window.pace)
    parts.push(
      `At this pace about ${100 - projectedUsed(window.pace.projected_percent_at_reset)}% is left at the reset.`,
    );
  return parts.join(' ');
}

/**
 * Remaining percent over the week on a fixed 0–100% scale: the app's own
 * readings as a solid line (or, while they span too little of the week to
 * see, the latest reading as a labeled dot), even use as a dashed grey line
 * from 100% at the start to 0% at the reset, and the current pace as a
 * dashed line from the latest reading to where it runs out (a dot) or to the
 * reset. Nothing is drawn where there was no reading. 100% and 0% are
 * labeled on the left; local midnights are ticked along the bottom with the
 * day's name.
 */
function Burndown({ window }: { window: AccountUsageWindow }) {
  const reset = window.resets_at;
  const minutes = window.duration_minutes;
  if (reset === null || !minutes || !Number.isFinite(reset)) return null;
  const start = reset - minutes * 60;
  const points = (window.series ?? []).filter(
    (point) =>
      Number.isFinite(point.at) &&
      Number.isFinite(point.used_percent) &&
      point.at >= start &&
      point.at <= reset,
  );
  const x = (at: number) =>
    chart.left + ((at - start) / (reset - start)) * (chart.right - chart.left);
  const y = (used: number) => chart.top + (clampPercent(used) / 100) * (chart.bottom - chart.top);
  const xy = (at: number, used: number) => `${x(at).toFixed(1)},${y(used).toFixed(1)}`;
  // The current reading. Without one, only even use and the reset are drawn.
  const last = points.at(-1);
  const line = readingsSpanVisible(points, start, reset);
  const pace = last ? paceLabel(window) : null;
  const runOutAt =
    pace?.warning && window.pace?.run_out_at != null
      ? Math.min(reset, window.pace.run_out_at)
      : null;
  const projection =
    !pace || !window.pace
      ? null
      : runOutAt !== null
        ? { at: runOutAt, used: 100 }
        : { at: reset, used: Math.min(100, window.pace.projected_percent_at_reset) };
  const midnights = localMidnights(start, reset);
  // Short day names when a day is wide enough for them, else one letter (as
  // in the day chart below); on a long window, only every few days.
  const dayWidth = (86_400 / (reset - start)) * (chart.right - chart.left);
  const short = dayWidth >= 32;
  const format = new Intl.DateTimeFormat(undefined, { weekday: short ? 'short' : 'narrow' });
  const every = Math.max(1, Math.ceil((short ? 22 : 10) / dayWidth));
  // Half a label's width: a label that would run off the chart is left out.
  const half = short ? 10 : 4;
  const dayLabels = midnights.flatMap((at, index) => {
    const cx = x(at);
    if (index % every !== 0 || cx < half || cx > chart.width - half) return [];
    return [{ at, cx, text: format.format(new Date(at * 1000)) }];
  });
  // The latest reading's own label never sits right of the dot at or below
  // it, where the pace line runs (remaining only falls): left of the dot, or
  // early in the week (no room on the left) above and to its right, or below
  // and to its left when the dot is too near the top.
  const latestLabel = (() => {
    if (!last || line) return null;
    const cx = x(last.at);
    const cy = y(last.used_percent);
    if (cx > chart.left + 25) return { x: cx - 5, y: cy, anchor: 'end' as const };
    if (cy - 7 >= chart.top) return { x: cx + 3, y: cy - 7, anchor: 'start' as const };
    return { x: cx - 2, y: cy + 8, anchor: 'end' as const };
  })();
  return (
    <div className="xt-account-burndown">
      <span className="xt-account-reset">Remaining this week</span>
      <svg
        viewBox={`0 0 ${chart.width} ${chart.height}`}
        role="img"
        aria-label={burndownSummary(window)}
      >
        <g className="xt-account-burndown-axis" aria-hidden="true">
          <text className="is-y" x={chart.left - 3} y={chart.top} dy="0.35em" textAnchor="end">
            100%
          </text>
          <text className="is-y" x={chart.left - 3} y={chart.bottom} dy="0.35em" textAnchor="end">
            0%
          </text>
          {midnights.map((at) => (
            <line
              key={at}
              className="is-day-tick"
              x1={x(at)}
              y1={chart.bottom}
              x2={x(at)}
              y2={chart.bottom + 4}
            />
          ))}
          {dayLabels.map((label) => (
            <text
              key={label.at}
              className="is-day"
              x={label.cx}
              y={chart.height - 1}
              textAnchor="middle"
            >
              {label.text}
            </text>
          ))}
        </g>
        <line
          className="is-base"
          x1={chart.left}
          y1={chart.bottom}
          x2={chart.right}
          y2={chart.bottom}
        />
        <line
          className="is-reset"
          x1={chart.right}
          y1={chart.top}
          x2={chart.right}
          y2={chart.bottom}
        />
        <line className="is-even" x1={x(start)} y1={y(0)} x2={x(reset)} y2={y(100)} />
        {last && projection && (
          <line
            className="is-projection"
            x1={x(last.at)}
            y1={y(last.used_percent)}
            x2={x(projection.at)}
            y2={y(projection.used)}
          />
        )}
        {line && (
          <polyline
            className="is-actual"
            points={points.map((point) => xy(point.at, point.used_percent)).join(' ')}
          />
        )}
        {last && (
          <circle
            className={line ? 'is-now' : 'is-now is-latest'}
            cx={x(last.at)}
            cy={y(last.used_percent)}
            r={line ? 2 : 2.5}
          />
        )}
        {last && latestLabel && (
          <text
            className="is-latest-label"
            x={latestLabel.x}
            y={latestLabel.y}
            dy="0.35em"
            textAnchor={latestLabel.anchor}
            aria-hidden="true"
          >
            {remainingPercent(clampPercent(last.used_percent))}
          </text>
        )}
        {runOutAt !== null && (
          <circle className="is-run-out" cx={x(runOutAt)} cy={y(100)} r={2.5} />
        )}
      </svg>
      <ul className="xt-account-burndown-legend" aria-hidden="true">
        {line && (
          <li>
            <svg viewBox="0 0 12 6">
              <line className="is-actual" x1={0} y1={3} x2={12} y2={3} />
            </svg>
            Remaining
          </li>
        )}
        {last && !line && (
          <li>
            <svg viewBox="0 0 12 6">
              <circle className="is-now" cx={6} cy={3} r={2.5} />
            </svg>
            Latest reading
          </li>
        )}
        <li>
          <svg viewBox="0 0 12 6">
            <line className="is-even" x1={0} y1={3} x2={12} y2={3} />
          </svg>
          Even use
        </li>
        {projection && (
          <li>
            <svg viewBox="0 0 12 6">
              <line className="is-projection" x1={0} y1={3} x2={12} y2={3} />
            </svg>
            This pace
          </li>
        )}
        {runOutAt !== null && (
          <li>
            <svg viewBox="0 0 12 6">
              <circle className="is-run-out" cx={6} cy={3} r={2.5} />
            </svg>
            Runs out
          </li>
        )}
        <li>
          <svg viewBox="0 0 12 6">
            <line className="is-reset" x1={6} y1={0} x2={6} y2={6} />
          </svg>
          Reset
        </li>
      </ul>
    </div>
  );
}

/** Percentage points of the limit that fill a day's box. Taller bars are clipped and marked. */
export const dailyBoxPoints = 30;

type DayKind = 'known' | 'unknown' | 'upcoming';
export type WeekDay = AccountUsageDay & { kind: DayKind };

function parseDate(text: string): Date | null {
  const [year, month, date] = text.split('-').map(Number);
  const local = new Date(year, month - 1, date);
  return Number.isNaN(local.getTime()) ? null : local;
}

function dateKey(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/**
 * The app's days (window start through the reading's day), then every later
 * local day up to the reset's day: days up to today have no reading, later
 * days are upcoming.
 */
export function weekDays(
  days: AccountUsageDay[],
  resetsAt: number | null,
  now: number = Date.now(),
): WeekDay[] {
  const cells: WeekDay[] = days.map((day) => ({
    ...day,
    kind: day.used_points === null || !Number.isFinite(day.used_points) ? 'unknown' : 'known',
  }));
  const lastDate = days.length ? parseDate(days[days.length - 1].date) : null;
  if (!lastDate || resetsAt === null || !Number.isFinite(resetsAt)) return cells;
  // The window's last moment is just before the reset: a reset at midnight
  // adds no day of its own.
  const end = new Date(resetsAt * 1000 - 1000);
  if (Number.isNaN(end.getTime())) return cells;
  const endDay = localDay(end);
  const today = localDay(new Date(now));
  for (let step = 1; step <= 45; step++) {
    const date = new Date(lastDate.getFullYear(), lastDate.getMonth(), lastDate.getDate() + step);
    if (date.getTime() > endDay) break;
    cells.push({
      date: dateKey(date),
      used_points: null,
      partial: false,
      kind: date.getTime() > today ? 'upcoming' : 'unknown',
    });
  }
  return cells;
}

/**
 * The number over a day's bar, in percentage points of the week (the
 * heading says so; the % sign is left out to fit a narrow box): "5", "≥5"
 * for a lower bound, "<1" for a trace, ">30" for a day over the box's
 * scale. A lower bound under 1 point says nothing, so it reads "—", as a
 * day with no reading does.
 */
export function dayValueLabel(day: AccountUsageDay): string | null {
  const points = day.used_points;
  if (points === null || !Number.isFinite(points)) return null;
  if (points > dailyBoxPoints) return `>${dailyBoxPoints}`;
  if (day.partial) return points < 1 ? UNMEASURED : `≥${Math.floor(points)}`;
  if (points > 0 && points < 1) return '<1';
  return `${Math.round(points)}`;
}

function dayLabel(day: WeekDay): string {
  const local = parseDate(day.date);
  const name = !local
    ? day.date
    : new Intl.DateTimeFormat(undefined, {
        weekday: 'short',
        month: 'short',
        day: 'numeric',
      }).format(local);
  if (day.kind === 'upcoming') return `${name}: upcoming`;
  if (day.kind === 'unknown' || day.used_points === null) return `${name}: no reading`;
  const points = Math.round(day.used_points * 10) / 10;
  return `${name}: ${day.partial ? 'at least ' : ''}${points}% of the week used`;
}

/**
 * One box per local day of the week. A box's full height is
 * `dailyBoxPoints` percentage points of the week's limit, so a bar's height
 * means the same on every day; the number above it is exact.
 */
function DailyChart({ days, resetsAt }: { days: AccountUsageDay[]; resetsAt: number | null }) {
  const cells = weekDays(days, resetsAt);
  return (
    <div className="xt-account-daily">
      <span className="xt-account-reset">Week by day · % of week</span>
      <ul aria-label="Use per day this week">
        {cells.map((day) => {
          const local = parseDate(day.date);
          const letter = local
            ? new Intl.DateTimeFormat(undefined, { weekday: 'narrow' }).format(local)
            : '';
          const points = day.kind === 'known' ? (day.used_points ?? 0) : 0;
          const over = points > dailyBoxPoints;
          const height =
            points <= 0
              ? 0
              : Math.max(8, (Math.min(points, dailyBoxPoints) / dailyBoxPoints) * 100);
          const value =
            day.kind === 'known' ? dayValueLabel(day) : day.kind === 'unknown' ? UNMEASURED : null;
          const className = [
            `is-${day.kind}`,
            day.kind === 'known' && day.partial ? 'is-partial' : null,
            over ? 'is-over' : null,
          ]
            .filter(Boolean)
            .join(' ');
          return (
            <li key={day.date} title={dayLabel(day)} className={className}>
              <span className="sr-only">{dayLabel(day)}</span>
              <span className="xt-account-daily-value" aria-hidden="true">
                {value ?? ' '}
              </span>
              <span className="xt-account-daily-bar" aria-hidden="true">
                {day.kind === 'known' && <span style={{ height: `${height}%` }} />}
              </span>
              <span className="xt-account-daily-day" aria-hidden="true">
                {letter}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

const maxTimeoutMs = 2_147_483_647;

/**
 * One UI wakeup for the next reported reset or the next time a reading turns
 * stale; this never reads a provider.
 */
function useResetWakeup(usage: AccountUsage | undefined, failed: boolean) {
  const [wakeups, setWakeups] = useState(0);
  useEffect(() => {
    if (failed || !usage) return;
    const now = Date.now();
    const readable = [usage.claude, usage.codex].filter(
      (provider) => provider.state === 'available' || provider.state === 'stale',
    );
    const nextStale = readable
      .map((provider) => (provider.stale_at ?? Number.NaN) * 1000)
      .filter((at) => Number.isFinite(at) && at > now);
    const nextReset = readable
      .flatMap((provider) => provider.windows)
      .filter(
        (window) =>
          Number.isFinite(window.used_percent) &&
          window.used_percent >= 0 &&
          window.used_percent <= 100 &&
          window.resets_at !== null &&
          Number.isFinite(window.resets_at * 1000) &&
          !Number.isNaN(new Date(window.resets_at * 1000).getTime()) &&
          window.resets_at * 1000 > now,
      )
      .map((window) => window.resets_at! * 1000)
      .concat(nextStale)
      .reduce<number | null>(
        (earliest, at) => (earliest === null || at < earliest ? at : earliest),
        null,
      );
    if (nextReset === null) return;
    const delay = Math.min(maxTimeoutMs, Math.max(1, Math.ceil(nextReset - now)));
    const timer = window.setTimeout(() => setWakeups((count) => count + 1), delay);
    return () => window.clearTimeout(timer);
  }, [usage, failed, wakeups]);
}

function WindowCharts({ window }: { window: AccountUsageWindow }) {
  return (
    <>
      {window.series && <Burndown window={window} />}
      {window.daily && window.daily.length > 0 && (
        <DailyChart days={window.daily} resetsAt={window.resets_at} />
      )}
    </>
  );
}

function hasCharts(window: AccountUsageWindow): boolean {
  return window.series !== undefined || (window.daily !== undefined && window.daily.length > 0);
}

function sameWindow(a: AccountUsageWindow, b: AccountUsageWindow): boolean {
  return a.bucket_key === b.bucket_key && a.window_key === b.window_key;
}

function WindowRow({ window }: { window: AccountUsageWindow }) {
  const percent = window.used_percent;
  const pace = paceLabel(window);
  if (!Number.isFinite(percent) || percent < 0 || percent > 100) return null;
  if (windowExpired(window))
    return (
      <div className="xt-account-window">
        <span className="xt-account-window-head">
          <span>{windowTitle(window)}</span>
        </span>
        <span className="xt-account-reset">Reset passed · Refresh for current limit</span>
      </div>
    );
  return (
    <div className="xt-account-window">
      <span className="xt-account-window-head">
        <span>{windowTitle(window)}</span>
        <strong>{remainingLabel(percent)}</strong>
      </span>
      <span className="xt-account-track" aria-hidden="true">
        <span style={{ width: `${100 - percent}%` }} />
      </span>
      <span className="xt-account-reset">{resetLabel(window.resets_at)}</span>
      {pace && (
        <span className={`xt-account-pace${pace.warning ? ' is-warning' : ''}`}>{pace.text}</span>
      )}
      <WindowCharts window={window} />
    </div>
  );
}

/**
 * Whether a provider's details list its windows as rows under the charts.
 * Claude's details show only the week's charts: its session and model-specific
 * limits are not listed. Codex lists every window.
 */
const listsWindowRows: Record<'claude' | 'codex', boolean> = {
  claude: false,
  codex: true,
};

function Provider({
  host,
  provider,
}: {
  host: 'claude' | 'codex';
  provider: AccountProviderUsage | undefined;
}) {
  const label = host === 'claude' ? 'Claude' : 'Codex';
  const readable = provider?.state === 'available' || provider?.state === 'stale';
  const selected = readable ? limitingWindow(provider.windows) : undefined;
  const resetPassed = selected ? windowExpired(selected) : false;
  const main = resetPassed ? undefined : selected;
  const detail = readable
    ? provider.windows.filter(
        (window) =>
          Number.isFinite(window.used_percent) &&
          window.used_percent >= 0 &&
          window.used_percent <= 100,
      )
    : [];
  const issue = provider?.issue ? issueLabels[provider.issue] : undefined;
  const stale = provider ? isStale(provider) : false;
  const status = !provider
    ? 'Reading…'
    : provider.issue === 'not_fetched' ||
        (provider.issue === 'reading' && provider.state === 'unavailable')
      ? issueLabels[provider.issue]
      : resetPassed
        ? 'Reset passed · Refresh for current limits'
        : main
          ? `${remainingLabel(main.used_percent)} · ${windowTitle(main)}${main.used_percent === 100 ? ' · limit reached' : ''}${stale ? ' · stale' : ''}`
          : stale
            ? 'Usage stale'
            : provider.state === 'failed'
              ? 'Usage failed'
              : provider.state === 'unavailable'
                ? 'Usage unavailable'
                : 'No limits reported';
  const statusLine =
    provider?.state === 'unavailable' && issue ? issue : issue ? `${status} · ${issue}` : status;
  const checked =
    provider?.checked_at === null || provider?.checked_at === undefined
      ? null
      : new Date(provider.checked_at * 1000);
  const age = ageLabel(provider?.checked_at ?? null, stale);
  const readAt =
    provider?.checked_at === null || provider?.checked_at === undefined
      ? null
      : shortTime(provider.checked_at);
  // The week's pace goes under the main row (the app gives daily use only to
  // the all-models week); it names the week when the
  // main row shows another window (such as a fuller session).
  const week =
    main && readable ? provider.windows.find((window) => window.daily !== undefined) : undefined;
  const pace = week ? paceLabel(week, week === main ? '' : 'Week') : null;
  const sub = main
    ? [
        windowTitle(main),
        main.used_percent === 100 ? 'Limit reached' : null,
        resetLabel(main.resets_at),
        stale ? 'Stale' : null,
        age,
      ]
        .filter(Boolean)
        .join(' · ')
    : statusLine;
  // The header gives the main window's percent and reset; its pace line is
  // the main window's own when the main window is the week, or when the main
  // window has no pace of its own to show.
  const repeated =
    main && (week === main || !paceLabel(main))
      ? detail.find((window) => sameWindow(window, main))
      : undefined;
  const listsRows = listsWindowRows[host];
  // Without rows, the details still show the week's charts: those of the main
  // row when the header already says the rest, else the first current window
  // that has charts (the all-models week).
  const charted = listsRows
    ? repeated
    : repeated && hasCharts(repeated)
      ? repeated
      : detail.find((window) => hasCharts(window) && !windowExpired(window));
  const describedBy = useId();
  const gapId = useId();
  // The even-use mark goes only on the week's own bar, and only while its
  // pace line is shown. The row's description carries its text too.
  const gap = week === main && pace && main ? paceGapLabel(main) : null;
  const fill = main && gap ? paceFill(main.used_percent, main.pace?.expected_percent) : {};
  return (
    <details className={`xt-account-provider xt-host-${host}`}>
      <summary
        tabIndex={0}
        aria-label={`${label} account usage: ${status}`}
        aria-describedby={main ? (gap ? `${describedBy} ${gapId}` : describedBy) : undefined}
      >
        <span className="xt-host-glyph" aria-hidden="true">
          <HostGlyph host={host} label={label} />
        </span>
        <span className="xt-account-name">{label}</span>
        <span className="xt-account-value">
          {main ? remainingLabel(main.used_percent) : '—'}
          <span className="xt-account-chevron" aria-hidden="true">
            ⌄
          </span>
        </span>
        {main && (
          <span className="xt-account-summary-bar">
            <span className="xt-account-track xt-account-summary-track" aria-hidden="true">
              <span
                className={fill.className}
                style={{
                  width: `${100 - main.used_percent}%`,
                  ...(fill.split ? { '--pace-split': fill.split } : {}),
                }}
              />
            </span>
            {gap && <PaceTick window={main} />}
          </span>
        )}
        <span
          className="xt-account-sub"
          id={describedBy}
          title={
            checked && Number.isFinite(checked.getTime())
              ? `Read ${checked.toLocaleString()}`
              : undefined
          }
        >
          {sub}
          {/* A run-out warning stays in view; the on-pace line shows only
              when the row is open, but screen readers still hear it here. */}
          {pace?.warning && <span className="xt-account-pace is-warning">{pace.text}</span>}
          {pace && !pace.warning && <span className="sr-only">{pace.text}</span>}
        </span>
        {gap && (
          <span id={gapId} className="sr-only">
            {gap}
          </span>
        )}
      </summary>
      <div className="xt-account-detail">
        {!main && <p>{statusLine}</p>}
        {pace && !pace.warning && (
          <span className="xt-account-pace" aria-hidden="true">
            {pace.text}
          </span>
        )}
        {detail.length > 0 && (
          <>
            {/* When the header already says all the main window's row would
                (percent, reset and its own pace), the details add only its
                charts, then the other windows (for providers that list them). */}
            {charted && hasCharts(charted) && (
              <div className="xt-account-window">
                <WindowCharts window={charted} />
              </div>
            )}
            {listsRows &&
              detail
                .filter((window) => !repeated || !sameWindow(window, repeated))
                .map((window) => (
                  <WindowRow key={`${window.bucket_key}:${window.window_key}`} window={window} />
                ))}
            {readAt && <small>Read {readAt}</small>}
          </>
        )}
      </div>
    </details>
  );
}

const forecastSteps = [
  'Usage is read every 10 minutes for Claude, and every 5 minutes for Codex while this window is open. Readings stay on this Mac.',
  'Your speed is your average since the week started.',
  "That speed is extended to the reset. If it reaches 100% first, you see when you'd run out.",
  "The tick on the bar marks where you'd be with even use through the week.",
  "It assumes a steady pace, so nights and weekends aren't taken into account.",
];

/**
 * How the forecast works. Opens on hover and on keyboard focus. A click (or
 * Enter) on a closed or hover- or focus-opened note keeps it open, so touch
 * and trackpad-click users can read it; the next click or Enter closes it, as
 * do Escape and a press elsewhere. Only one tooltip shows at a time, so
 * hovering Refresh closes this note even when a click kept it open.
 */
function ForecastInfo() {
  const id = useId();
  const theme = useSurfaceTheme();
  const [open, setOpen] = useState(false);
  // How it was opened. A note opened by focus or kept by a click does not
  // close when the pointer passes over the button and leaves; one opened by
  // hover does.
  const openedBy = useRef<'hover' | 'focus' | 'click' | null>(null);
  const change = (next: boolean, reason: Tooltip.Root.ChangeEventReason) => {
    if (next) {
      if (!open) openedBy.current = reason === 'trigger-focus' ? 'focus' : 'hover';
      setOpen(true);
      return;
    }
    if (reason === 'trigger-hover' && openedBy.current !== 'hover') return;
    openedBy.current = null;
    setOpen(false);
  };
  const toggle = () => {
    // A click on a note kept by an earlier click closes it; otherwise the
    // click keeps the note open.
    if (open && openedBy.current === 'click') {
      openedBy.current = null;
      setOpen(false);
      return;
    }
    openedBy.current = 'click';
    setOpen(true);
  };
  return (
    <Tooltip.Root
      open={open}
      onOpenChange={(next, details) => change(next, details.reason)}
      disableHoverablePopup
    >
      <Tooltip.Trigger
        render={
          <button
            type="button"
            // WebKit skips buttons on Tab unless they say otherwise.
            tabIndex={0}
            className="xt-account-icon-button"
            aria-label="How the forecast works"
            aria-expanded={open}
            aria-describedby={open ? id : undefined}
            onClick={toggle}
          />
        }
        delay={0}
        closeOnClick={false}
      >
        <Icon name="info" size={14} />
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="top"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover xt-account-forecast-tip">
            <strong>How the forecast works</strong>
            <ul>
              {forecastSteps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ul>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

function RefreshButton({
  refreshing,
  disabled,
  onRefresh,
}: {
  refreshing: boolean;
  disabled: boolean;
  onRefresh: () => void;
}) {
  const theme = useSurfaceTheme();
  const label = refreshing ? 'Reading usage…' : 'Refresh usage';
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={
          <button
            type="button"
            // WebKit skips buttons on Tab unless they say otherwise.
            tabIndex={0}
            className="xt-account-icon-button xt-account-refresh"
            disabled={disabled}
            data-reading={refreshing ? '' : undefined}
            onClick={onRefresh}
            aria-label={label}
          />
        }
        delay={300}
      >
        <Icon name="refresh" size={14} />
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="top"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup role="tooltip" className="xt-rule-popover xt-account-tick-tip">
            {label}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export function AccountUsageWidget({
  usage,
  failed,
  refreshing,
  refreshEnabled = true,
  failureMessage,
  onRefresh,
}: {
  usage?: AccountUsage;
  failed: boolean;
  refreshing: boolean;
  refreshEnabled?: boolean;
  failureMessage?: string;
  onRefresh: () => void;
}) {
  useResetWakeup(usage, failed);
  return (
    <section className="xt-account-usage" aria-label="Account usage">
      <div className="xt-account-heading">
        <h2>Usage</h2>
        <span className="xt-account-actions">
          <ForecastInfo />
          <RefreshButton
            refreshing={refreshing}
            disabled={refreshing || !refreshEnabled}
            onRefresh={onRefresh}
          />
        </span>
      </div>
      {failed ? (
        <p className="xt-sidebar-empty" role="status">
          {failureMessage ??
            (refreshEnabled
              ? 'Account usage could not be read. Refresh to try again.'
              : 'Account usage is available in the desktop app.')}
        </p>
      ) : (
        <>
          <Provider host="claude" provider={usage?.claude} />
          <Provider host="codex" provider={usage?.codex} />
        </>
      )}
    </section>
  );
}
