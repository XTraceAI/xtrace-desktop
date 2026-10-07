import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { MetricEffortCost } from '../../data/generated/MetricEffortCost';
import type { MetricPrFreshness } from '../../data/generated/MetricPrFreshness';
import type { MetricPrFreshnessSummary } from '../../data/generated/MetricPrFreshnessSummary';
import type { PrAttemptOutcome } from '../../data/generated/PrAttemptOutcome';
import type { PrAutoCheckPause } from '../../data/generated/PrAutoCheckPause';
import type { PrAutoCheckStatus } from '../../data/generated/PrAutoCheckStatus';
import type { PrRefreshErrorCode } from '../../data/generated/PrRefreshErrorCode';
import type { PrRefreshReport } from '../../data/generated/PrRefreshReport';
import type { PrRefreshStatusReport } from '../../data/generated/PrRefreshStatusReport';
import type { PrRow } from '../../data/generated/PrRow';
import type { PrSkipReason } from '../../data/generated/PrSkipReason';
import { clockTime, plural } from './present';

/** Presentation only: every count, subtotal and day value is the Rust report's. */

/** The Effort card's measure: agent hours, human time or cost. */
export type EffortMetric = 'agent' | 'human' | 'dollars';
/** The bar chart's measures; human time is drawn as a timeline instead. */
export type BarMetric = Exclude<EffortMetric, 'human'>;

/** The refresh command's own bound on one selection. */
export const REFRESH_LIMIT = 20;

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

const capital = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/**
 * GitHub answered that the pull request does not exist in a repository it
 * could see. That is an answer, not a failed check: such a number is left out
 * of the pull-request counts and the Pull requests page unless GitHub
 * confirmed it before.
 */
const notFound = (outcome: PrAttemptOutcome) =>
  outcome.outcome === 'failed' && outcome.error === 'not_found';

/** Storage's own word that this number is not a pull request. */
export const notFoundOnGitHub = (row: PrRow) => row.status.status === 'not_found_on_github';

/**
 * One attempt's result, in the same words as the stored status (`checked`).
 * `row` is the stored row as read after the batch: a not-found answer for a
 * pull request GitHub confirmed earlier keeps those facts, and one never
 * confirmed is not counted.
 */
export function outcomeText(outcome: PrAttemptOutcome, row?: PrRow) {
  if (outcome.outcome === 'skipped') return `Skipped: ${skipText[outcome.reason]}`;
  const notFoundWords = capital(refreshStateLabel.not_found_on_github);
  const ran =
    outcome.outcome === 'succeeded'
      ? capital(refreshStateLabel.refreshed)
      : notFound(outcome)
        ? row === undefined
          ? notFoundWords
          : // "Were there earlier facts to keep?" is the stored success time,
            // not the status: the result can show before the list is read
            // again (or when storage did not keep it), while the row still
            // says "not checked yet".
            row.refreshed_at_ms === null
            ? `${notFoundWords}; not counted as a pull request`
            : `${notFoundWords}; earlier facts kept`
        : `${capital(refreshStateLabel.failed_never_refreshed)}: ${refreshErrorText[outcome.error]}; earlier facts are kept`;
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

/**
 * The words for a pull request's GitHub check status, the one set the app uses
 * — the Merged PRs tile, the refresh dialog, merge markers, the PRs report and
 * the cached inventory. Each also reads after a count (`3 not checked yet`).
 * A not-found answer is GitHub's answer, not a failed check, so it has its own
 * words.
 */
export const refreshStateLabel = {
  never_attempted: 'not checked yet',
  refreshed: 'checked',
  failed_never_refreshed: 'could not be checked',
  failed_after_refresh: 'stale after a failed check',
  not_found_on_github: 'not found on GitHub',
  /** Earlier facts kept after GitHub said the number is not found. */
  not_found_after_refresh: 'stale: not found on GitHub at the last check',
} as const satisfies Record<PrRefreshStatusReport['status'] | 'not_found_after_refresh', string>;

/**
 * A pull request's check status, the one way the app words it: a short label,
 * and the failure's reason when there is one. Callers add only when it
 * happened, never a different word for the status.
 */
export function refreshStatusWords(status: PrRefreshStatusReport | MetricPrFreshness): {
  label: string;
  reason: string | null;
} {
  const state = 'status' in status ? status.status : status.state;
  const error = 'error' in status ? status.error : null;
  if (error === 'not_found')
    return {
      label:
        state === 'failed_after_refresh'
          ? refreshStateLabel.not_found_after_refresh
          : refreshStateLabel.not_found_on_github,
      reason: null,
    };
  return {
    label: refreshStateLabel[state],
    reason: error === null ? null : refreshErrorText[error],
  };
}

/** One manual batch's result in words, in the status words. A not-found answer is not a failure. */
export function refreshReportText(report: PrRefreshReport) {
  const missing = report.rows.filter((row) => notFound(row.outcome)).length;
  const parts = [
    `${report.succeeded} ${refreshStateLabel.refreshed}`,
    `${report.failed - missing} ${refreshStateLabel.failed_never_refreshed}`,
  ];
  if (missing > 0) parts.push(`${missing} ${refreshStateLabel.not_found_on_github}`);
  parts.push(`${report.skipped} skipped`);
  if (report.unrecorded > 0) parts.push(`${report.unrecorded} not stored`);
  return `Requested ${report.requested}: ${parts.join(', ')}.${
    report.cancelled ? ' The batch was cancelled.' : ''
  } ${report.committed ? 'Stored facts changed; the Dashboard reads them again.' : 'No stored facts changed.'}`;
}

/** The status on one line: `stale after a failed check (timed out)`. */
export function refreshStatusText(status: PrRefreshStatusReport | MetricPrFreshness) {
  const { label, reason } = refreshStatusWords(status);
  return reason === null ? label : `${label} (${reason})`;
}

export function rowStatusText(row: PrRow, window: DashboardWindow) {
  const at = (ms: number | null) => (ms === null ? 'an unknown time' : clockTime(ms, window));
  const text = refreshStatusText(row.status);
  switch (row.status.status) {
    case 'refreshed':
      return `${text} ${at(row.refreshed_at_ms)}`;
    case 'failed_after_refresh':
      return `${text}; facts from ${at(row.refreshed_at_ms)}`;
    case 'not_found_on_github':
      return `${text} (checked ${at(row.last_attempted_at_ms)}); not counted as a pull request`;
    default:
      return text;
  }
}

/** Only the counts the report states, in a fixed order, omitting zeros. */
export function prFreshnessText(freshness: MetricPrFreshnessSummary, window: DashboardWindow) {
  const states = [
    'refreshed',
    'failed_after_refresh',
    'failed_never_refreshed',
    'never_attempted',
  ] as const;
  const parts = states
    .filter((state) => freshness[state] > 0)
    .map((state) => `${freshness[state]} ${refreshStateLabel[state]}`);
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
  /** Of `couldNot`, those a manual refresh already failed for too. */
  triedCouldNot: number;
  /** Of `stale`, those a manual refresh already failed for too. */
  triedStale: number;
  /** Checked and merged, but GitHub gave no merge time, so the range cannot tell. */
  noMergeTime: number;
}

export function prCheckCounts(tile: MetricMergedPrs): PrCheckCounts {
  const { freshness } = tile;
  return {
    notChecked: freshness.never_attempted,
    couldNot: freshness.failed_never_refreshed,
    stale: freshness.failed_after_refresh,
    triedCouldNot: freshness.manual_failed_never_refreshed,
    triedStale: freshness.manual_failed_after_refresh,
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
  /** The tile's own counts, so every text says the same numbers. */
  counts: PrCheckCounts;
  /** Never checked, counted only when nothing checks them on its own. */
  unchecked: number;
}

/** Failed pull requests a manual refresh has not met yet: the ones the red ! is for. */
const pendingFailures = (counts: PrCheckCounts) =>
  counts.couldNot - counts.triedCouldNot + (counts.stale - counts.triedStale);

/**
 * The one rule for "do the pull-request checks need the user?". It is the
 * Effort card's red ! and the only gate for every text that points to it.
 * They need the user when a linked pull request's last check failed (with or
 * without earlier facts), when automatic checks paused because of gh, or when
 * automatic checks are off (or absent) and some pull request was never
 * checked. Pull requests not checked yet while automatic checks run do not:
 * XTrace checks them by itself. Nor does a failed one the user already
 * checked again by hand, with that check failing for the pull request itself
 * (it may never be checkable: another account's repository), until a later
 * check succeeds; Rust counts those (`manual_failed_*`), and every text still
 * lists them as failed.
 *
 * It reads the report's counts and the automatic check's status, which reach
 * the screen separately (the status changes on its own, by event), so the two
 * are combined here rather than in one DTO field.
 */
export function prAttention(tile: MetricMergedPrs, auto: AutoCheck): PrAttention | null {
  const counts = prCheckCounts(tile);
  const paused = auto?.paused ?? null;
  const autoOff = auto === null || auto?.enabled === false;
  const unchecked = autoOff ? counts.notChecked : 0;
  return paused !== null || pendingFailures(counts) > 0 || unchecked > 0
    ? { paused, counts, unchecked }
    : null;
}

/**
 * How many of `total` failed ones also failed when the user checked them in
 * the refresh dialog, said after them about the same pull requests.
 */
function triedText(tried: number, total: number) {
  if (tried === 0) return '';
  if (tried === total)
    return total === 1
      ? ' Your own check of it failed too.'
      : total === 2
        ? ' Your own checks of both failed too.'
        : ` Your own checks of all ${total} failed too.`;
  return ` ${tried} of them also failed when you checked ${tried === 1 ? 'it' : 'them'} yourself.`;
}

/**
 * The failed pull requests in plain words, the same sentences on the tile's
 * tip, the red !'s tip and the refresh dialog; `unit` names a number of them.
 */
function failureSentences(counts: PrCheckCounts, unit: (count: number) => string) {
  return [
    counts.couldNot > 0 &&
      `${checkSentence(unit, counts.couldNot, 'failed_never_refreshed')} The last try failed.${triedText(counts.triedCouldNot, counts.couldNot)}`,
    counts.stale > 0 &&
      `${checkSentence(unit, counts.stale, 'failed_after_refresh')} Older facts are shown.${triedText(counts.triedStale, counts.stale)}`,
  ].filter((part): part is string => typeof part === 'string');
}

/**
 * What the red ! asks the user to check, in words that match the sentences
 * before it: "it" or "them", or "the other one" / "the other N" when some of
 * the failed ones also failed when the user checked them; "again" when the ones it
 * names were tried before.
 */
function checkObject({ paused, counts, unchecked }: PrAttention) {
  const pending = pendingFailures(counts);
  const named = pending + unchecked;
  const others = counts.triedCouldNot + counts.triedStale > 0;
  const pronoun = named === 1 ? 'it' : 'them';
  const object =
    named === 0 ? '' : others ? (named === 1 ? 'the other one' : `the other ${named}`) : pronoun;
  return { object, pronoun, again: paused !== null || pending > 0 ? ' again' : '' };
}

const pullRequests = (count: number) => plural(count, 'pull request');

/**
 * A count of pull requests in one check state as a sentence, in the one
 * check-status word set: `3 pull requests are not checked yet.`, `1 PR could
 * not be checked.`.
 */
function checkSentence(
  unit: (count: number) => string,
  count: number,
  state: MetricPrFreshness['state'],
) {
  const label = refreshStateLabel[state];
  // "could not be checked" carries its own verb; the others take "is"/"are".
  const verb = label.startsWith('could ') ? '' : count === 1 ? 'is ' : 'are ';
  return `${unit(count)} ${verb}${label}.`;
}

/**
 * What is wrong, then what to do: on the red !'s tip, click it; inside the
 * refresh dialog it opens, refresh them there.
 */
export function prAttentionText(attention: PrAttention, place: 'tip' | 'dialog' = 'tip') {
  const { paused, counts, unchecked } = attention;
  const { object, pronoun, again } = checkObject(attention);
  const action =
    place === 'tip'
      ? `click to check${object ? ` ${object}` : ''}${again}`
      : object
        ? `choose ${object} below and refresh ${pronoun}${again}`
        : // Nothing in particular is waiting (gh paused): name what to choose.
          'choose pull requests below and refresh them';
  return [
    paused !== null && pausedText[paused],
    ...failureSentences(counts, pullRequests),
    unchecked > 0 &&
      `Automatic checks are off and ${checkSentence(pullRequests, unchecked, 'never_attempted')}`,
    paused !== null ? `${pausedFix[paused]}, then ${action}.` : `${capital(action)}.`,
  ]
    .filter((part): part is string => typeof part === 'string')
    .join(' ');
}

/** Where the Merged PRs tip sends the user when the checks need them. */
export function prAttentionPointer(attention: PrAttention) {
  const { object, again } = checkObject(attention);
  const click = `click the red ! on the Effort card to check${object ? ` ${object}` : ''}${again}.`;
  return attention.paused !== null
    ? `${pausedFix[attention.paused]}, then ${click}`
    : capital(click);
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
        ? `${prs(notChecked)} ${refreshStateLabel.never_attempted}`
        : couldNot > 0
          ? `${prs(couldNot)} ${refreshStateLabel.failed_never_refreshed}`
          : `${prs(noMergeTime)} with no merge time`;
    const aside = checking
      ? 'checking…'
      : paused
        ? paused
        : notChecked > 0 && couldNot > 0
          ? `${couldNot} ${refreshStateLabel.failed_never_refreshed}`
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
        ? `${notChecked} ${refreshStateLabel.never_attempted}`
        : couldNot > 0
          ? `${couldNot} ${refreshStateLabel.failed_never_refreshed}`
          : `${noMergeTime} with no merge time`;
  return { value: tile.known_merged, incomplete: true, aside, freshness };
}

function freshnessLine(
  tile: MetricMergedPrs,
  auto: AutoCheck,
  window: DashboardWindow,
  counts: PrCheckCounts,
) {
  const { notChecked, noMergeTime } = counts;
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
    notChecked > 0 && checkSentence(prs, notChecked, 'never_attempted'),
    ...failureSentences(counts, prs),
    noMergeTime > 0 && `${prs(noMergeTime)} merged with no merge time.`,
    auto?.enabled === false && 'Automatic checks are off.',
    attention !== null && prAttentionPointer(attention),
  ].filter((part): part is string => typeof part === 'string');
  return parts.length > 0 ? parts.join(' ') : 'Every linked pull request has been checked.';
}
