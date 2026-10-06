import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { MetricEffortCost } from '../../data/generated/MetricEffortCost';
import type { MetricPrFreshness } from '../../data/generated/MetricPrFreshness';
import type { MetricPrFreshnessSummary } from '../../data/generated/MetricPrFreshnessSummary';
import type { PrAttemptOutcome } from '../../data/generated/PrAttemptOutcome';
import type { PrAutoCheckPause } from '../../data/generated/PrAutoCheckPause';
import type { PrAutoCheckStatus } from '../../data/generated/PrAutoCheckStatus';
import type { PrRefreshErrorCode } from '../../data/generated/PrRefreshErrorCode';
import type { PrRefreshStatusReport } from '../../data/generated/PrRefreshStatusReport';
import type { PrRow } from '../../data/generated/PrRow';
import type { PrSkipReason } from '../../data/generated/PrSkipReason';
import { continuous } from '../metric-format';
import { clockTime, plural } from './present';

/** Presentation only: every count, subtotal and day value is the Rust report's. */

/** The Effort card's measure: agent hours, your hours or cost. */
export type EffortMetric = 'agent' | 'human' | 'dollars';
/** The bar chart's measures; your hours are drawn as a timeline instead. */
export type BarMetric = Exclude<EffortMetric, 'human'>;

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

/** The Merged PRs tile's definition, in place of the rule's technical text. */
export const MERGED_PRS_DEFINITION =
  'Pull requests your agent sessions opened or pushed to, and how many of them merged on GitHub in this range. XTrace checks GitHub using your gh sign-in; it only reads.';

const prs = (count: number) => plural(count, 'PR');

const pausedAside: Record<PrAutoCheckPause, string> = {
  gh_missing: 'gh not found',
  gh_signed_out: 'gh not signed in',
};

/** Why automatic checks stopped, in plain words; what to do follows it. */
const pausedText: Record<PrAutoCheckPause, string> = {
  gh_missing: 'XTrace could not run the GitHub CLI (gh), so it stopped checking.',
  gh_signed_out: 'The GitHub CLI (gh) is not signed in, so XTrace stopped checking.',
};

/** What to do about a pause before checking again. */
const pausedFix: Record<PrAutoCheckPause, string> = {
  gh_missing: 'Install gh',
  gh_signed_out: 'Run gh auth login',
};

/**
 * The automatic check as the Dashboard knows it: its status, `null` where the
 * data source never checks on its own (a fixture or test source), and
 * `undefined` while the status is not known yet.
 */
export type AutoCheck = PrAutoCheckStatus | null | undefined;

/**
 * How many linked pull requests are in each check state. Each count answers
 * one question and is the only place it is worked out: the Merged PRs tile and
 * the red ! say the same words for the same number.
 */
export interface PrCheckCounts {
  /** Never checked. */
  notChecked: number;
  /** Checked, but the check failed and no facts were ever stored. */
  couldNot: number;
  /** The last check failed; facts from an earlier check are shown. */
  stale: number;
  /** Checked and merged, but GitHub gave no merge time, so the range cannot tell. */
  noMergeTime: number;
}

export function prCheckCounts(tile: MetricMergedPrs): PrCheckCounts {
  const { freshness } = tile;
  return {
    notChecked: freshness.never_attempted,
    couldNot: freshness.failed_never_refreshed,
    stale: freshness.failed_after_refresh,
    // The rest of the undecided ones: storage holds facts, but no merge time.
    noMergeTime: Math.max(
      0,
      tile.unknown_facts - freshness.never_attempted - freshness.failed_never_refreshed,
    ),
  };
}

/** Why the pull-request checks need the user; see {@link prAttention}. */
export interface PrAttention {
  /** Automatic checks stopped because of the GitHub CLI. */
  paused: PrAutoCheckPause | null;
  /** {@link PrCheckCounts.couldNot}. */
  couldNot: number;
  /** {@link PrCheckCounts.stale}. */
  stale: number;
  /** Never checked, counted only when nothing checks them on its own. */
  unchecked: number;
}

/**
 * The one rule for "do the pull-request checks need the user?". It is the
 * Effort card's red ! and the only gate for every text that points to it.
 * They need the user when a linked pull request's last check failed (with or
 * without earlier facts), when automatic checks paused because of gh, or when
 * automatic checks are off (or absent) and some pull request was never
 * checked. Pull requests not checked yet while automatic checks run do not:
 * XTrace checks them by itself.
 *
 * It reads the report's counts and the automatic check's status, which reach
 * the screen separately (the status changes on its own, by event), so the two
 * are combined here rather than in one DTO field.
 */
export function prAttention(tile: MetricMergedPrs, auto: AutoCheck): PrAttention | null {
  const { notChecked, couldNot, stale } = prCheckCounts(tile);
  const paused = auto?.paused ?? null;
  const autoOff = auto === null || auto?.enabled === false;
  const unchecked = autoOff ? notChecked : 0;
  return paused !== null || couldNot > 0 || stale > 0 || unchecked > 0
    ? { paused, couldNot, stale, unchecked }
    : null;
}

/** "it" or "them" for the pull requests an attention names, and "again" when they were tried. */
function checkObject({ paused, couldNot, stale, unchecked }: PrAttention) {
  const named = couldNot + stale + unchecked;
  const again = paused !== null || couldNot + stale > 0;
  return { pronoun: named === 1 ? 'it' : 'them', named, again: again ? ' again' : '' };
}

const pullRequests = (count: number) => plural(count, 'pull request');

/** The red !'s sentences of what is wrong, in the tile's words for the same counts. */
function attentionProblems({ paused, couldNot, stale, unchecked }: PrAttention) {
  const unit = pullRequests;
  return [
    paused !== null && pausedText[paused],
    couldNot > 0 && `${unit(couldNot)} could not be checked.`,
    stale > 0 && `The last check of ${unit(stale)} failed; older facts are shown.`,
    unchecked > 0 &&
      `Automatic checks are off and ${unit(unchecked)} ${unchecked === 1 ? 'was' : 'were'} never checked.`,
  ].filter((part): part is string => typeof part === 'string');
}

/**
 * What is wrong, then what to do: on the red !'s tip, click it; inside the
 * refresh dialog it opens, refresh them there.
 */
export function prAttentionText(attention: PrAttention, place: 'tip' | 'dialog' = 'tip') {
  const { pronoun, named, again } = checkObject(attention);
  const action =
    place === 'tip'
      ? `click to check${named === 0 ? '' : ` ${pronoun}`}${again}`
      : `choose ${pronoun} below and refresh ${pronoun}${again}`;
  const { paused } = attention;
  return [
    ...attentionProblems(attention),
    paused !== null
      ? `${pausedFix[paused]}, then ${action}.`
      : `${action.charAt(0).toUpperCase()}${action.slice(1)}.`,
  ].join(' ');
}

/** Where the Merged PRs tip sends the user when the checks need them. */
export function prAttentionPointer(attention: PrAttention) {
  const { pronoun, named, again } = checkObject(attention);
  const click = `click the red ! on the Effort card to check${named === 0 ? '' : ` ${pronoun}`}${again}.`;
  return attention.paused !== null
    ? `${pausedFix[attention.paused]}, then ${click}`
    : `${click.charAt(0).toUpperCase()}${click.slice(1)}`;
}

/** What the Merged PRs tile shows, in plain words. */
export interface MergedTileView {
  /** The number shown; null when no merged count can be shown yet. */
  value: number | null;
  /** Shown in place of the number while there is none. */
  placeholder?: string;
  /** The number is the known part of an unfinished count ("so far"). */
  incomplete?: boolean;
  aside?: string;
  /** One plain line on how fresh the facts are, for the tile's tip. */
  freshness: string;
}

/**
 * Pull requests the tile cannot decide yet are not checked at all, checked
 * without an answer (the last try failed), or merged without a merge time.
 * Until GitHub has answered for at least one linked pull request, the tile
 * says so in words instead of a number; after that, the known merged count
 * (even zero) is shown as "so far", with how many are still undecided, or why
 * checks paused, under it. Every count is {@link prCheckCounts}'.
 */
export function mergedTileView(
  tile: MetricMergedPrs,
  auto: AutoCheck,
  window: DashboardWindow,
): MergedTileView {
  const counts = prCheckCounts(tile);
  const { notChecked, couldNot, noMergeTime } = counts;
  const checking = auto?.checking === true;
  const paused = auto?.paused ? pausedAside[auto.paused] : undefined;
  const freshness = freshnessLine(tile, auto, window, counts);
  if (tile.complete)
    return {
      value: tile.merged,
      aside: tile.unresolved_type > 0 ? `${tile.unresolved_type} untyped` : undefined,
      freshness,
    };
  // A zero is only shown once GitHub has answered for at least one linked
  // pull request; before that it would be a guess, so the words stand in.
  const anyChecked = tile.freshness.refreshed + tile.freshness.failed_after_refresh > 0;
  if (tile.known_merged === 0 && !anyChecked) {
    const placeholder =
      notChecked > 0
        ? `${prs(notChecked)} not checked yet`
        : couldNot > 0
          ? `${prs(couldNot)} could not be checked`
          : `${prs(noMergeTime)} with no merge time`;
    const aside = checking
      ? 'checking…'
      : paused
        ? paused
        : notChecked > 0 && couldNot > 0
          ? `${couldNot} could not be checked`
          : undefined;
    return { value: null, placeholder, aside, freshness };
  }
  // The known count is shown, marked incomplete ("so far" beside the number);
  // the line under it says what is missing.
  const aside = checking
    ? 'checking…'
    : paused
      ? paused
      : notChecked > 0
        ? `${notChecked} not checked yet`
        : couldNot > 0
          ? `${couldNot} could not be checked`
          : `${noMergeTime} with no merge time`;
  return { value: tile.known_merged, incomplete: true, aside, freshness };
}

function freshnessLine(
  tile: MetricMergedPrs,
  auto: AutoCheck,
  window: DashboardWindow,
  { notChecked, couldNot, stale, noMergeTime }: PrCheckCounts,
) {
  const { freshness } = tile;
  const linked =
    freshness.never_attempted +
    freshness.refreshed +
    freshness.failed_never_refreshed +
    freshness.failed_after_refresh;
  if (linked === 0) return 'No agent session is linked to a pull request yet.';
  // The pointer to the red ! is said exactly when the red ! is shown.
  const attention = prAttention(tile, auto);
  const parts = [
    auto?.checking ? 'Checking GitHub now…' : auto?.paused ? pausedText[auto.paused] : false,
    !auto?.checking &&
      freshness.newest_attempted_at !== null &&
      `Last checked ${clockTime(freshness.newest_attempted_at, window)}.`,
    notChecked > 0 && `${prs(notChecked)} not checked yet.`,
    couldNot > 0 && `${prs(couldNot)} could not be checked; the last try failed.`,
    stale > 0 && `The last check of ${prs(stale)} failed; older facts are shown.`,
    noMergeTime > 0 && `${prs(noMergeTime)} merged with no merge time.`,
    auto?.enabled === false && 'Automatic checks are off.',
    attention !== null && prAttentionPointer(attention),
  ].filter((part): part is string => typeof part === 'string');
  return parts.length > 0 ? parts.join(' ') : 'Every linked pull request has been checked.';
}
