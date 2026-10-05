import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { MetricEffortCost } from '../../data/generated/MetricEffortCost';
import type { MetricPrFreshness } from '../../data/generated/MetricPrFreshness';
import type { MetricPrFreshnessSummary } from '../../data/generated/MetricPrFreshnessSummary';
import type { PrAttemptOutcome } from '../../data/generated/PrAttemptOutcome';
import type { PrRefreshErrorCode } from '../../data/generated/PrRefreshErrorCode';
import type { PrRefreshStatusReport } from '../../data/generated/PrRefreshStatusReport';
import type { PrRow } from '../../data/generated/PrRow';
import type { PrSkipReason } from '../../data/generated/PrSkipReason';
import { continuous } from '../metric-format';
import { clockTime } from './present';

/** Presentation only: every count, subtotal and day value is the Rust report's. */

export type EffortMetric = 'agent' | 'dollars';

/** The refresh command's own bound on one selection. */
export const REFRESH_LIMIT = 20;

export const agentHours = (agentMs: number) => `${continuous(agentMs / 3_600_000)} h`;

export type CostCell =
  | { state: 'measured'; value: number }
  | { state: 'none'; value: null }
  | { state: 'unknown'; value: null };

/**
 * No selected response has nothing to price. Otherwise the plotted value is
 * the priced subtotal, and unknown only when not one response could be
 * priced; a partial subtotal is stated as partial wherever it is shown.
 */
export const costCell = (cost: MetricEffortCost): CostCell =>
  cost.selected_observations === 0
    ? { state: 'none', value: null }
    : cost.priced_observations === 0
      ? { state: 'unknown', value: null }
      : { state: 'measured', value: cost.priced_subtotal_usd };

export const refreshErrorText: Record<PrRefreshErrorCode, string> = {
  unavailable: 'GitHub CLI unavailable',
  timeout: 'timed out',
  cancelled: 'cancelled',
  output_too_large: 'response too large',
  invalid_response: 'unexpected response',
  execution_failed: 'gh pr view failed',
  not_found: 'not found on GitHub',
  unauthorized: 'not authorized; sign in with gh auth login',
  rate_limited: 'rate limited',
};

export const skipText: Record<PrSkipReason, string> = {
  not_stored: 'not stored any more; nothing ran',
  not_linked: 'no session links it any more; nothing ran',
  cancelled: 'cancelled before it ran',
  budget_exhausted: 'the batch time limit was reached before it ran',
  storage_unavailable: 'storage became unavailable before it ran',
  attempt_refused: 'refused before it ran',
};

export function outcomeText(outcome: PrAttemptOutcome) {
  if (outcome.outcome === 'skipped') return `Skipped: ${skipText[outcome.reason]}`;
  const ran =
    outcome.outcome === 'succeeded'
      ? 'Refreshed'
      : `Failed: ${refreshErrorText[outcome.error]}; earlier facts are kept`;
  const stored = outcome.persistence;
  if (stored.persistence === 'not_recorded')
    return `${ran} · not stored (${
      stored.reason === 'storage_closed'
        ? 'storage closed'
        : stored.reason === 'refused'
          ? 'storage refused the result'
          : 'storage failed'
    })`;
  return `${ran} · ${
    stored.write === 'applied'
      ? 'saved'
      : stored.write === 'unchanged'
        ? 'unchanged'
        : 'older than the stored facts, not applied'
  }`;
}

export function rowStatusText(row: PrRow, window: DashboardWindow) {
  const status: PrRefreshStatusReport = row.status;
  const at = (ms: number | null) => (ms === null ? 'an unknown time' : clockTime(ms, window));
  switch (status.status) {
    case 'never_attempted':
      return 'never refreshed';
    case 'refreshed':
      return `refreshed ${at(row.refreshed_at_ms)}`;
    case 'failed_never_refreshed':
      return `failed (${refreshErrorText[status.error]}); never refreshed`;
    case 'failed_after_refresh':
      return `stale: last refresh failed (${refreshErrorText[status.error]}); facts from ${at(row.refreshed_at_ms)}`;
  }
}

export function markerFreshnessText(freshness: MetricPrFreshness) {
  switch (freshness.state) {
    case 'never_attempted':
      return 'never refreshed';
    case 'refreshed':
      return 'refreshed';
    case 'failed_never_refreshed':
      return `failed (${refreshErrorText[freshness.error]})`;
    case 'failed_after_refresh':
      return `stale: last refresh failed (${refreshErrorText[freshness.error]})`;
  }
}

/** Only the counts the report states, in a fixed order, omitting zeros. */
export function prFreshnessText(freshness: MetricPrFreshnessSummary, window: DashboardWindow) {
  const parts = [
    freshness.refreshed > 0 && `${freshness.refreshed} refreshed`,
    freshness.failed_after_refresh > 0 &&
      `${freshness.failed_after_refresh} stale after a failed refresh`,
    freshness.failed_never_refreshed > 0 &&
      `${freshness.failed_never_refreshed} failed, never refreshed`,
    freshness.never_attempted > 0 && `${freshness.never_attempted} never refreshed`,
  ].filter((part): part is string => typeof part === 'string');
  if (parts.length === 0) return 'No session has a confirmed pull-request link yet.';
  const oldest =
    freshness.oldest_refreshed_at === null
      ? ''
      : ` · oldest facts ${clockTime(freshness.oldest_refreshed_at, window)}`;
  return `Confirmed-linked pull requests: ${parts.join(', ')}${oldest}.`;
}

/**
 * The tile's aside while facts are missing: the known subtotal plus how many
 * pull requests could still add to it, never a zero answer. The tile's reason
 * states the same two counts in full.
 */
export function mergedAside(tile: MetricMergedPrs) {
  if (!tile.complete) return `${tile.known_merged} + ${tile.unknown_facts} unknown`;
  if (tile.unresolved_type > 0) return `${tile.unresolved_type} untyped`;
  return undefined;
}
