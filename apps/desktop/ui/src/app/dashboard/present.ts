import type { DashboardCapture } from '../../data/generated/DashboardCapture';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricCaptureGap } from '../../data/generated/MetricCaptureGap';
import type { MetricExcludedSurface } from '../../data/generated/MetricExcludedSurface';
import type { MetricFavoriteUnknown } from '../../data/generated/MetricFavoriteUnknown';
import type { MetricInventory } from '../../data/generated/MetricInventory';
import type { MetricTile } from '../../data/generated/MetricTile';
import type { MetricUnpricedReason } from '../../data/generated/MetricUnpricedReason';
import type { MetricUsageGap } from '../../data/generated/MetricUsageGap';
import type { ControlTone } from '../../kit/control-tone';
import { rules, type RuleId } from '../../kit/rules';
import type { TimeRange } from '../../kit/TopBar';

/** Presentation only: every value here was calculated by the Rust report. */

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

const usdFormat = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});
export const usd = (value: number) =>
  value > 0 && value < 0.005 ? '<$0.01' : usdFormat.format(Object.is(value, -0) ? 0 : value);

export const plural = (count: number, one: string, many = `${one}s`) =>
  `${count.toLocaleString('en-US')} ${count === 1 ? one : many}`;

/** The report zone is an IANA name, or `system-local` when the OS reports none. */
const zoneOf = (window: DashboardWindow) => {
  if (window.timezone === 'system-local') return undefined;
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: window.timezone });
    return window.timezone;
  } catch {
    return undefined;
  }
};
export function windowLabel(window: DashboardWindow) {
  const timeZone = zoneOf(window);
  const date = new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    timeZone,
  });
  // end_ms is exclusive: the last included instant is one millisecond earlier.
  return date.formatRange(window.start_ms, Math.max(window.start_ms, window.end_ms - 1));
}
export const zoneLabel = (window: DashboardWindow) =>
  window.timezone === 'system-local' ? 'system time zone' : window.timezone;

export function clockTime(ms: number, window: DashboardWindow, withDate = true) {
  return new Intl.DateTimeFormat('en-US', {
    month: withDate ? 'short' : undefined,
    day: withDate ? 'numeric' : undefined,
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
    timeZone: zoneOf(window),
  }).format(ms);
}

export const surfaceLabel = (host: string, surface: string | null) =>
  `${host} · ${surface ?? 'unknown surface'}`;

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
