import type { DashboardCapture } from '../../data/generated/DashboardCapture';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricCaptureGap } from '../../data/generated/MetricCaptureGap';
import type { MetricExcludedSurface } from '../../data/generated/MetricExcludedSurface';
import type { MetricFavoriteUnknown } from '../../data/generated/MetricFavoriteUnknown';
import type { MetricInventory } from '../../data/generated/MetricInventory';
import type { MetricTile } from '../../data/generated/MetricTile';
import type { MetricUnpricedReason } from '../../data/generated/MetricUnpricedReason';
import type { MetricUsageGap } from '../../data/generated/MetricUsageGap';
import { clock, dayRange } from '../../kit/clock';
import type { ControlTone } from '../../kit/control-tone';
import { surfaceLabel } from '../../kit/hosts';
import { rules, type RuleId } from '../../kit/rules';
import type { TimeRange } from '../../kit/TopBar';

/** Presentation only: every value here was calculated by the Rust report. */

/**
 * Money and a host's surface are each written in one place (the kit's `usd`
 * and `surfaceLabel`); they are re-exported here for the report's screens.
 */
export { usd } from '../../kit/format';
export { surfaceLabel };

export const rangeDays: Record<TimeRange, 7 | 14 | 30> = { '7d': 7, '14d': 14, '30d': 30 };

/**
 * Report percentages (`pct`, `delta.pct`) are percentage points: 50 means 50%.
 * The kit formatters expect a fraction, so this is the single conversion point.
 */
export const fromPoints = (points: number | null | undefined): number | null =>
  typeof points === 'number' && Number.isFinite(points) ? points / 100 : null;

/** A delta shown only when the report did not suppress it. */
export const tileDelta = (tile: MetricTile): number | undefined =>
  tile.delta.suppressed ? undefined : (fromPoints(tile.delta.pct) ?? undefined);

export const ruleId = (id: string, fallback: RuleId): RuleId =>
  Object.hasOwn(rules, id) ? (id as RuleId) : fallback;

export const plural = (count: number, one: string, many = `${one}s`) =>
  `${count.toLocaleString('en-US')} ${count === 1 ? one : many}`;

/** The report zone is an IANA name, or `system-local` when the OS reports none. */
export const zoneOf = (window: DashboardWindow) => {
  if (window.timezone === 'system-local') return undefined;
  try {
    new Intl.DateTimeFormat(undefined, { timeZone: window.timezone });
    return window.timezone;
  } catch {
    return undefined;
  }
};
export function windowLabel(window: DashboardWindow) {
  // end_ms is exclusive: the last included instant is one millisecond earlier.
  return dayRange(window.start_ms, window.end_ms - 1, zoneOf(window));
}
export const zoneLabel = (window: DashboardWindow) =>
  window.timezone === 'system-local' ? 'system time zone' : window.timezone;

/** An instant in the report's zone, with its day unless `withDate` is false. */
export function clockTime(ms: number, window: DashboardWindow, withDate = true) {
  return clock(ms, { date: withDate ? 'day' : 'none', timeZone: zoneOf(window) });
}

/**
 * A chart axis tick: the same clock, with `:00` left off a whole hour on a
 * 12-hour clock (`12 PM`) so quarter ticks clear each other in a narrow column.
 */
export function axisTime(ms: number, window: DashboardWindow, withDate = false) {
  return clock(ms, {
    date: withDate ? 'day' : 'none',
    timeZone: zoneOf(window),
    shortHour: true,
  });
}

/** An instant in full — year, date, time and the report zone — for a title. */
export function recordedTime(ms: number, window: DashboardWindow) {
  return `${clock(ms, { date: 'year', timeZone: zoneOf(window) })} (${zoneLabel(window)})`;
}

export const excludedSurfaceText = (item: MetricExcludedSurface) =>
  `${surfaceLabel(item.host, item.surface)}: ${item.degenerate_sessions} of ${plural(item.qualifying_sessions, 'qualifying session')} have batch-stamped timestamps`;

export const favoriteUnknownText: Record<MetricFavoriteUnknown, string> = {
  no_measured_output: 'No measured output tokens in this range',
  unknown_model_output: 'Measured output has an unknown model',
  unknown_turn_attribution: 'A tie could not be resolved from turn attribution',
};

export const unpricedText: Record<MetricUnpricedReason, string> = {
  missing_model: 'model not recorded',
  unknown_model: 'model not in the price catalog',
  missing_service_tier: 'service tier not recorded',
  unknown_service_tier: 'service tier not in the price catalog',
  missing_counters: 'token counters not recorded',
  missing_prompt_counters: 'prompt counters not recorded',
  missing_cache_split: '5m/1h cache-write split not recorded',
  inconsistent_cache_split: 'cache-write split is inconsistent',
  missing_rate: 'catalog has no rate for this usage',
  missing_timestamp: 'no time recorded',
};

export const usageGapText: Record<MetricUsageGap, string> = {
  no_selected_usage: 'no recorded usage',
  incomplete_counters: 'incomplete token counters',
  unknown_model: 'unknown model',
};

export const captureGapText: Record<MetricCaptureGap, string> = {
  inventory_unknown: 'session inventory unknown',
  inventory_incomplete: 'session inventory incomplete',
  inventory_stale: 'session inventory stale',
  missing_python: 'Python runtime unavailable',
  store_unavailable: 'host store unavailable',
  missing_native_start: 'native start time missing',
  missing_conversation_identity: 'conversation identity missing',
  unknown_surface: 'surface unknown',
  discovery_incomplete: 'discovery incomplete',
  identity_mismatch: 'identity mismatch',
  surface_mismatch: 'surface mismatch',
  unresolved_surface_denominator: 'surface denominator unresolved',
};

/** Only a fresh, complete inventory may look healthy; unknown is never green. */
export const inventoryState: Record<MetricInventory, { label: string; tone: ControlTone }> = {
  fresh_complete: { label: 'Inventory complete', tone: 'success' },
  unknown: { label: 'Inventory unknown', tone: 'meta' },
  incomplete: { label: 'Inventory incomplete', tone: 'warning' },
  stale: { label: 'Inventory stale', tone: 'warning' },
  missing_python: { label: 'Python unavailable', tone: 'warning' },
  unavailable: { label: 'Inventory unavailable', tone: 'warning' },
};

export const captureText = (row: DashboardCapture) =>
  `${row.captured_sessions} of ${plural(row.observed_sessions, 'observed session')} with a capture`;

export const unavailableReason = (
  unavailable: readonly { key: string; reason: string }[],
  key: string,
  fallback: string,
) => unavailable.find((item) => item.key === key)?.reason ?? fallback;

/**
 * Said when the report hides a tile's change against the previous period:
 * either period has fewer than five samples, or the previous value is zero.
 */
export const DELTA_HIDDEN =
  'No change shown: too little data in this or the last period, or the last period was zero.';

/**
 * The report's fixed "no number" reasons in everyday words. A reason the
 * report adds later and this table does not know is shown as written.
 */
export const plainReasons: Record<string, string> = {
  'Agent time is unmeasured': 'Agent time could not be measured in this range.',
  'Human classification or required typing length is unmeasured':
    'Could not tell which messages a person sent or how long they were.',
  'Human classification is unmeasured': 'Could not tell which messages a person sent.',
  'No positive-duration active spans': 'No measurable agent activity in this range.',
  'No measurable eligible hands-off stretches':
    'No stretch where an agent worked on its own in this range.',
  'Sessions are unmeasured': 'Sessions could not be counted in this range.',
  'Tool count is unmeasured': 'Tool calls could not be counted in this range.',
  'Selected usage counters are absent or incomplete': 'Some token counts are missing.',
  'Selected usage is absent or unpriced; see cost reasons':
    'Some usage has no price; the cost details say why.',
  'Rule-fire data is unavailable': 'Rule fire data is not available.',
  "Human time is unknown: a message's sender is not classified":
    'Could not tell which messages a person sent, so human time is unknown.',
  'You sent no messages to agents in this range': 'You sent no messages to agents in this range.',
};

/** A report reason for a missing number, as one plain sentence. */
export const plainReason = (reason: string) => plainReasons[reason] ?? sentence(reason);

/** Why a tile has no number, in plain words; nothing when it has one. */
export const tileReason = (tile: MetricTile) =>
  tile.value === null && tile.reason ? plainReason(tile.reason) : undefined;

/**
 * The one short note a tile's definition adds after the rule's summary (and,
 * for a tile with no number, after the reason the tile itself shows): the
 * caller's own note, otherwise why no change is shown. The report's longer
 * method notes are not repeated; the rule's summary says what the number means.
 */
export function tileTip(tile: MetricTile, ...extra: (string | null | undefined)[]) {
  const note = extra.find(Boolean);
  if (note) return note;
  if (tile.value !== null && tile.current_n !== null && tile.delta.suppressed) return DELTA_HIDDEN;
  return undefined;
}

/** A report phrase as a sentence: capitalised and closed with a full stop. */
export const sentence = (text: string) => {
  const trimmed = text.trim();
  const capital = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  return /[.!?]$/.test(capital) ? capital : `${capital}.`;
};

/** Which surfaces a hands-off figure leaves out, in one short sentence. */
export function excludedNote(surfaces: readonly MetricExcludedSurface[]) {
  if (surfaces.length === 0) return undefined;
  if (surfaces.length === 1)
    return `Leaves out ${surfaceLabel(surfaces[0].host, surfaces[0].surface)}: its timestamps are too coarse.`;
  return `Leaves out ${surfaces.length} apps whose timestamps are too coarse.`;
}
