import { describe, expect, it } from 'vitest';
import type { MetricPrMedian } from '../data/generated/MetricPrMedian';
import {
  emptyReportText,
  evidenceMix,
  medianAside,
  medianGap,
  medianView,
  mixedEvidence,
  rowHandsOff,
  rowTokensReason,
  tokenMedianView,
  workTypeLabel,
} from './pr-analytics';
import { scenarios, syntheticReport } from './pr-analytics.synthetic';

const median = (overrides: Partial<MetricPrMedian>): MetricPrMedian => ({
  eligible_prs: 4,
  measured_prs: 4,
  unknown_prs: 0,
  no_sample_prs: 0,
  measured_median: 3,
  median: 3,
  ...overrides,
});

describe('per-PR medians', () => {
  it('publishes the complete median only as the report does, else the measured one, labelled', () => {
    expect(medianView(median({}))).toEqual({ kind: 'complete', value: 3 });
    expect(medianAside(median({}))).toBe('n = 4 PRs');
    const partial = median({ measured_prs: 3, unknown_prs: 1, median: null });
    expect(medianView(partial)).toEqual({ kind: 'measured', value: 3 });
    expect(medianAside(partial)).toBe('3 of 4 PRs');
    expect(medianGap(partial)).toBe('Median of 3 of 4 merged PRs; 1 unknown.');
    expect(medianGap(median({}))).toBeUndefined();
  });

  it('counts a published median by the PRs that gave it a value, not every eligible PR', () => {
    const sparse = median({ eligible_prs: 3, measured_prs: 2, no_sample_prs: 1, median: 5 });
    expect(medianView(sparse)).toEqual({ kind: 'complete', value: 5 });
    expect(medianAside(sparse)).toBe('n = 2 of 3 PRs');
    expect(medianGap(sparse, 'no stretch')).toBe('Median of 2 of 3 merged PRs; 1 with no stretch.');
  });

  it('keeps a measured zero a value and gives every absence its counted reason', () => {
    expect(medianView(median({ measured_median: 0, median: 0 }))).toEqual({
      kind: 'complete',
      value: 0,
    });
    expect(
      medianView(
        median({
          eligible_prs: 3,
          measured_prs: 0,
          unknown_prs: 1,
          no_sample_prs: 2,
          measured_median: null,
          median: null,
        }),
        'no stretch',
      ),
    ).toEqual({
      kind: 'none',
      reason: 'No measured pull request: of 3 merged PRs, 1 unknown and 2 with no stretch',
    });
    expect(
      medianView(median({ eligible_prs: 0, measured_prs: 0, measured_median: null, median: null })),
    ).toEqual({ kind: 'none', reason: 'No pull request is known to have merged in this range' });
  });

  it('names the gate when token medians are withheld, over the report’s own counts', () => {
    const page = syntheticReport(scenarios.sparse, 30, false);
    const view = tokenMedianView(page.report.summary.tokens, page.report.token_gate, page.window);
    expect(view.kind).toBe('none');
    expect(view.kind === 'none' && view.reason).toMatch(/^Withheld: Token\+model coverage/);
    // Nothing merged: that is said instead of a gate that withholds nothing.
    const empty = syntheticReport(scenarios.sparse, 7, false);
    expect(
      tokenMedianView(empty.report.summary.tokens, empty.report.token_gate, empty.window),
    ).toEqual({ kind: 'none', reason: 'No pull request is known to have merged in this range' });
  });
});

describe('rows', () => {
  const rows = syntheticReport(scenarios.measured, 7, false).report.rows;
  const row = (repository: string, number: number) =>
    rows.find((item) => item.repository === repository && item.number === number)!;

  it('explains unknown tokens with the member counts the report carries', () => {
    expect(rowTokensReason(row('example/atlas', 102))).toBe(
      'Unknown: 1 of 2 linked sessions has no selected usage in this range',
    );
  });

  it('keeps pooled hands-off, no stretch, unknown and excluded apart', () => {
    expect(rowHandsOff(row('example/atlas', 101)).text).toMatch(
      /^Median of 6 pooled stretches from 3 linked sessions, in minutes$/,
    );
    expect(rowHandsOff(row('example/harbor', 7))).toEqual({
      value: null,
      text: 'No hands-off stretch in this range, a known absence',
    });
    expect(rowHandsOff(row('example/harbor', 9)).text).toMatch(/^Unknown: 1 linked session leaves/);
    expect(rowHandsOff(row('example/atlas', 103)).text).toMatch(
      /1 session on an excluded surface not counted \(Claude Code · desktop: 2 of 3 qualifying sessions/,
    );
  });

  it('never calls a row with an excluded member a known absence', () => {
    // One excluded member beside one measured member without stretches.
    const mixed = rowHandsOff(row('example/atlas', 109));
    expect(mixed.value).toBeNull();
    expect(mixed.text).not.toMatch(/known absence/);
    expect(mixed.text).toBe(
      'Unknown: no stretch among 1 measured linked session, and 1 session on an excluded surface is not counted (Claude Code · desktop: 2 of 3 qualifying sessions have batch-stamped timestamps)',
    );
    // Without exclusions the same absence is known.
    expect(rowHandsOff(row('example/harbor', 7)).text).toBe(
      'No hands-off stretch in this range, a known absence',
    );
    // An unknown classification keeps the excluded surfaces' names.
    const desk = row('example/atlas', 103);
    const unknown = rowHandsOff({
      ...desk,
      hands_off: { ...desk.hands_off, n: null, median_min: null, unknown_sessions: 1 },
    });
    expect(unknown.text).toBe(
      'Unknown: 1 linked session leaves a stretch boundary unknown; 1 session on an excluded surface not counted (Claude Code · desktop: 2 of 3 qualifying sessions have batch-stamped timestamps)',
    );
  });

  it('discloses mixed evidence and an unresolved type in words', () => {
    expect(mixedEvidence(row('example/atlas', 101))).toBe(true);
    expect(evidenceMix(row('example/atlas', 101))).toBe('2 exact · 1 commit');
    expect(mixedEvidence(row('example/harbor', 7))).toBe(false);
    expect(workTypeLabel(row('example/harbor', 9).work_type)).toBe('unresolved');
  });

  it('never reads an empty report as nothing merged when facts are unknown', () => {
    const sparse = syntheticReport(scenarios.sparse, 7, true).report;
    expect(emptyReportText(sparse, true)).toBe(
      'No merged pull request is known in this range. 1 linked pull request has unknown cached facts, so it may have merged here; 2 are open, closed or merged outside it.',
    );
    const none = {
      ...sparse,
      eligibility: { ...sparse.eligibility, outside: 0, unknown_facts: 0 },
    };
    expect(emptyReportText(none, true)).toBe(
      'No session links a pull request with exact or commit evidence.',
    );
  });
});
