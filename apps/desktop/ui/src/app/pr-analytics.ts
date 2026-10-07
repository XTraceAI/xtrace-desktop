import type { DashboardUsageGate } from '../data/generated/DashboardUsageGate';
import type { DashboardWindow } from '../data/generated/DashboardWindow';
import type { MetricExcludedSurface } from '../data/generated/MetricExcludedSurface';
import type { MetricPrAnalyticsReport } from '../data/generated/MetricPrAnalyticsReport';
import type { MetricPrAnalyticsRow } from '../data/generated/MetricPrAnalyticsRow';
import type { MetricPrConfidence } from '../data/generated/MetricPrConfidence';
import type { MetricPrFreshness } from '../data/generated/MetricPrFreshness';
import type { MetricPrMedian } from '../data/generated/MetricPrMedian';
import type { MetricPrTokenMedian } from '../data/generated/MetricPrTokenMedian';
import { count } from '../kit/format';
import { excludedSurfaceText, plural, surfaceLabel, windowLabel } from './dashboard/present';
import { refreshStatusText } from './dashboard/pr-effort';

/**
 * Words for the PRs page's report. Presentation only: every number is a field
 * of the Rust report as read. Nothing here adds rows, recounts sessions or
 * takes a median; a count shown beside a value is the report's own count.
 */

/** The address parameter naming the PRs page's view; absent is the report. */
export const PRS_VIEW = 'view';
export const INVENTORY_VIEW = 'inventory';

/** The page's standing caveat, used in every label a value could travel with. */
export const OVERLAP = 'Linked-session effort; may overlap other PRs';

/** A pull request's canonical identity, as the report and the drilldown name it. */
export const prIdentity = (row: { repository: string; number: number }) =>
  `${row.repository}#${row.number}`;

/** The report's work type, or unresolved; an unresolved type is not `other`. */
export const workTypeLabel = (workType: string | null) => workType ?? 'unresolved';

/**
 * A pull-request link's evidence in words, the one wording every screen uses:
 * a link found by a commit is `commit`, never the stored `sha`.
 */
export const evidenceWords: Record<MetricPrConfidence, string> = {
  exact: 'exact',
  sha: 'commit',
  inferred: 'inferred',
};

/**
 * One per-PR median as a tile or panel shows it. `complete` is the report's
 * unqualified median, published only when no eligible pull request is
 * unknown. Otherwise a value is the median over measured pull requests only,
 * and it says so; with none measured there is no value, and the reason says
 * why in the report's own counts.
 */
export type MedianView =
  | { kind: 'complete'; value: number }
  | { kind: 'measured'; value: number }
  | { kind: 'none'; reason: string };

export function medianView(median: MetricPrMedian, noSample = 'no sample'): MedianView {
  if (median.median !== null) return { kind: 'complete', value: median.median };
  if (median.measured_median !== null) return { kind: 'measured', value: median.measured_median };
  if (median.eligible_prs === 0)
    return { kind: 'none', reason: 'No pull request is known to have merged in this range' };
  const parts = [
    median.unknown_prs > 0 && `${median.unknown_prs} unknown`,
    median.no_sample_prs > 0 && `${median.no_sample_prs} with ${noSample}`,
  ].filter((part): part is string => typeof part === 'string');
  return {
    kind: 'none',
    reason: `No measured pull request: of ${plural(median.eligible_prs, 'merged PR')}, ${parts.join(' and ')}`,
  };
}

/** What a median left out, as one short note; nothing when every PR gave a value. */
export function medianGap(median: MetricPrMedian, noSample = 'no sample') {
  const rest = [
    median.unknown_prs > 0 && `${median.unknown_prs} unknown`,
    median.no_sample_prs > 0 && `${median.no_sample_prs} with ${noSample}`,
  ].filter((part): part is string => typeof part === 'string');
  if (rest.length === 0) return undefined;
  return `Median of ${median.measured_prs} of ${plural(median.eligible_prs, 'merged PR')}; ${rest.join(', ')}.`;
}

/**
 * The compact n beside a value. A median's n is the PRs that gave it a value,
 * `measured_prs`; a PR without a sample or with an unknown value gave none, so
 * the eligible count stands beside it rather than in its place.
 */
export function medianAside(median: MetricPrMedian) {
  if (median.median === null) return `${median.measured_prs} of ${median.eligible_prs} PRs`;
  return median.measured_prs === median.eligible_prs
    ? `n = ${median.eligible_prs} PRs`
    : `n = ${median.measured_prs} of ${median.eligible_prs} PRs`;
}

/** One median's own sample in words: its n, then what the rest of its PRs were. */
export function medianSample(median: MetricPrMedian, noSample = 'no sample') {
  const rest = [
    median.unknown_prs > 0 && `${median.unknown_prs} unknown`,
    median.no_sample_prs > 0 && `${median.no_sample_prs} with ${noSample}`,
  ].filter((part): part is string => typeof part === 'string');
  const n =
    median.measured_prs === median.eligible_prs
      ? `n = ${plural(median.eligible_prs, 'merged PR')}`
      : `n = ${median.measured_prs} of ${plural(median.eligible_prs, 'merged PR')}`;
  return rest.length > 0 ? `${n}: ${rest.join(', ')}` : n;
}

/**
 * The gate's coverage as shown: one decimal, rounded down, never up. Rust
 * decides whether the gate passes; rounding up could make a failing 89.99%
 * read as `90%` beside "need at least 90%", so the shown value never exceeds
 * the measured one.
 */
export const gatePercentText = (pct: number) => `${Math.floor(pct * 10 + 1e-9) / 10}%`;

/** Rust's pass/fail for the gate, in words. */
export const gateVerdict = (passes: boolean | null) =>
  passes === null ? 'gate unknown' : passes ? 'meets the 90% gate' : 'below the 90% gate';

/** The fixed trailing-14-day token+model coverage behind token medians. */
export function gateText(gate: DashboardUsageGate, window: DashboardWindow) {
  const span = windowLabel({
    ...window,
    start_ms: gate.window_start_ms,
    end_ms: gate.window_end_ms,
  });
  const coverage =
    gate.pct === null
      ? 'no session with work in those days'
      : `${count(gate.measured_sessions)} of ${plural(gate.eligible_sessions, 'session')} measured with a model (${gatePercentText(gate.pct)}, ${gateVerdict(gate.passes)})`;
  const excluded =
    gate.excluded_surfaces.length > 0
      ? ` · not counted, structurally unmeasured: ${gate.excluded_surfaces
          .map((surface) => surfaceLabel(surface.host, surface.surface))
          .join(', ')}`
      : '';
  return `Token+model coverage over the fixed 14 days ${span}: ${coverage}; token medians need at least 90%${excluded}`;
}

/** Why token medians are absent even though their counts are shown. */
export function tokenMedianView(
  tokens: MetricPrTokenMedian,
  gate: DashboardUsageGate,
  window: DashboardWindow,
): MedianView {
  // With no merged PR there is nothing to withhold; say that first.
  if (tokens.median.eligible_prs === 0) return medianView(tokens.median);
  if (tokens.withheld === 'gate_failed')
    return { kind: 'none', reason: `Withheld: ${gateText(gate, window)}` };
  if (tokens.withheld === 'gate_unknown')
    return { kind: 'none', reason: `Withheld, coverage unknown: ${gateText(gate, window)}` };
  return medianView(tokens.median);
}

/** How fresh a row's cached merge facts are, without an age policy. */
export function freshnessText(freshness: MetricPrFreshness) {
  const text = refreshStatusText(freshness);
  return freshness.state === 'failed_after_refresh' ? `${text}; earlier facts kept` : text;
}

const sessionsText = (value: number) => plural(value, 'linked session');

/** Why a row's four-counter token total is absent. */
export function rowTokensReason(row: MetricPrAnalyticsRow) {
  if (row.linked_sessions === 0) return 'No indexed session is linked, so nothing was measured';
  const parts = [
    row.tokens.no_selected_usage_sessions > 0 &&
      `${row.tokens.no_selected_usage_sessions} of ${sessionsText(row.linked_sessions)} ${row.tokens.no_selected_usage_sessions === 1 ? 'has' : 'have'} no selected usage in this range`,
    row.tokens.incomplete_sessions > 0 &&
      `${row.tokens.incomplete_sessions} ${row.tokens.incomplete_sessions === 1 ? 'has' : 'have'} usage missing a counter`,
  ].filter((part): part is string => typeof part === 'string');
  return `Unknown: ${parts.length > 0 ? parts.join('; ') : 'a counter is missing'}`;
}

const counter = (value: number | null) => (value === null ? 'unknown' : count(value));

/** The four counters behind a row's total, exactly. */
export function rowTokensTitle(row: MetricPrAnalyticsRow) {
  const { counters } = row.tokens;
  return [
    `total ${counter(counters.total_tokens)}`,
    `input ${counter(counters.input_tokens)}`,
    `output ${counter(counters.output_tokens)}`,
    `cache read ${counter(counters.cache_read_tokens)}`,
    `cache write ${counter(counters.cache_creation_tokens)}`,
    `${row.tokens.measured_sessions} of ${sessionsText(row.linked_sessions)} measured`,
  ].join(' · ');
}

export function rowHumanReason(row: MetricPrAnalyticsRow) {
  if (row.linked_sessions === 0) return 'No indexed session is linked, so nothing was measured';
  return `Unknown: ${plural(row.human_unknown_sessions, 'linked session')} ${row.human_unknown_sessions === 1 ? 'has' : 'have'} a message whose human or agent origin is unknown`;
}

const excludedText = (surfaces: MetricExcludedSurface[]) =>
  surfaces.map(excludedSurfaceText).join('; ');

/**
 * A row's pooled hands-off: the value's sample, or why there is none. A row
 * without a median is a known absence only when no member was excluded: an
 * excluded member's stretches exist but cannot be measured, so the core
 * counts such a row as unknown, and so does this.
 */
export function rowHandsOff(row: MetricPrAnalyticsRow): { value: number | null; text: string } {
  const handsOff = row.hands_off;
  const surfaces = excludedText(handsOff.excluded_surfaces);
  const excluded =
    handsOff.excluded_sessions > 0
      ? `${plural(handsOff.excluded_sessions, 'session')} on an excluded surface not counted (${surfaces})`
      : null;
  if (row.linked_sessions === 0)
    return { value: null, text: 'No indexed session is linked, so nothing was measured' };
  if (handsOff.n === null)
    return {
      value: null,
      text: `Unknown: ${plural(handsOff.unknown_sessions, 'linked session')} ${handsOff.unknown_sessions === 1 ? 'leaves' : 'leave'} a stretch boundary unknown${excluded ? `; ${excluded}` : ''}`,
    };
  if (handsOff.median_min === null) {
    if (handsOff.excluded_sessions === 0)
      return { value: null, text: 'No hands-off stretch in this range, a known absence' };
    return {
      value: null,
      text:
        handsOff.measured_sessions === 0
          ? `Unknown: every linked session is on an excluded surface (${surfaces})`
          : `Unknown: no stretch among ${plural(handsOff.measured_sessions, 'measured linked session')}, and ${plural(handsOff.excluded_sessions, 'session')} on an excluded surface ${handsOff.excluded_sessions === 1 ? 'is' : 'are'} not counted (${surfaces})`,
    };
  }
  return {
    value: handsOff.median_min,
    text: `Median of ${plural(handsOff.n, 'pooled stretch', 'pooled stretches')} from ${plural(handsOff.measured_sessions, 'linked session')}, in minutes${excluded ? `; ${excluded}` : ''}`,
  };
}

/**
 * Why a successfully read report has nothing to group by type; unknown cached
 * facts are said, never read as nothing merged.
 */
export function emptyTypesText(report: MetricPrAnalyticsReport) {
  const unknown = report.eligibility.unknown_facts;
  return unknown > 0
    ? `No known merged PR to group; ${plural(unknown, 'linked PR')} ${unknown === 1 ? 'has' : 'have'} unknown cached facts.`
    : 'No merged pull request to group.';
}

/** A companion surface's words while the report is not shown. */
export const REPORT_PENDING = 'Reading the report…';
export const REPORT_FAILED = 'Not available: the report could not be read';

/** Each linked session's own evidence, strongest first, zeros left out. */
export function evidenceMix(row: MetricPrAnalyticsRow) {
  return (['exact', 'sha', 'inferred'] as const)
    .filter((kind) => row.evidence[kind] > 0)
    .map((kind) => `${row.evidence[kind]} ${evidenceWords[kind]}`)
    .join(' · ');
}

/** Whether the row's sessions carry more than one kind of evidence. */
export const mixedEvidence = (row: MetricPrAnalyticsRow) =>
  (['exact', 'sha', 'inferred'] as const).filter((kind) => row.evidence[kind] > 0).length > 1;

/**
 * Why the report has no rows, in its own eligibility counts. Unknown cached
 * facts are never read as "nothing merged".
 */
export function emptyReportText(report: MetricPrAnalyticsReport, confirmedOnly: boolean) {
  const { outside, unknown_facts: unknown } = report.eligibility;
  if (outside + unknown === 0)
    return confirmedOnly
      ? 'No session links a pull request with exact or commit evidence.'
      : 'No session links a pull request yet.';
  const parts = [
    unknown > 0 &&
      `${plural(unknown, 'linked pull request')} ${unknown === 1 ? 'has' : 'have'} unknown cached facts, so ${unknown === 1 ? 'it' : 'they'} may have merged here`,
    outside > 0 && `${outside} ${outside === 1 ? 'is' : 'are'} open, closed or merged outside it`,
  ].filter((part): part is string => typeof part === 'string');
  return `No merged pull request is known in this range. ${parts.join('; ')}.`;
}
