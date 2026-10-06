// @vitest-environment node
import { createHash } from 'node:crypto';
import { expect, it } from 'vitest';
import report from '../../../../../docs/acceptance/metrics/rule-coverage.json';
import { rules, ruleSummaries, ruleSummary, ruleText, SUMMARY_MAX, type RuleId } from './rules';

it('covers all current metric IDs, PR variants, and referenced privacy/capture definitions', () => {
  const expected = [
    ...Array.from({ length: 19 }, (_, index) => `M-${String(index + 1).padStart(2, '0')}`),
    'M-11a',
    'M-12a',
    'R-05',
    'R-08',
    'P-01',
    'P-02',
    'U-03',
    'U-08',
    'O-11',
    'O-12',
    'C-08',
  ].sort();
  expect(Object.keys(rules).sort()).toEqual(expected);
  expect(report.matched).toBe(expected.length);
  expect(report.rules.map((entry) => entry.id).sort()).toEqual(expected);
  for (const entry of report.rules)
    expect(
      createHash('sha256')
        .update(ruleText(entry.id as RuleId))
        .digest('hex'),
    ).toBe(entry.textSha256);
  expect(ruleText('toString' as RuleId)).toBe('Definition unavailable');
});

it('retains amended coverage and privacy rules', () => {
  expect(rules['M-04']).toContain('missing counters are not implicit zero');
  expect(rules['M-11']).toContain('Empty eligible denominator → —');
  expect(rules['M-18']).toContain('it is not complete measurement verification');
  expect(rules['M-19']).toContain('first unions their distinct session IDs');
  expect(rules['P-02']).toContain('Original host files are never modified');
  expect(rules['C-08']).toContain('it must never inherit backfill-only fields');
});

it('states confirmed automated inputs as neutral without proving unmatched inputs human', () => {
  expect(rules['M-02']).toContain('never a human message, and contributes no characters');
  expect(rules['M-02']).toContain('which does not prove that a person submitted it');
  for (const id of ['M-03', 'M-07', 'M-09'] as const)
    expect(rules[id]).toContain('A confirmed automated input (M-02)');
  expect(rules['M-07']).toContain(
    'counted input characters divided by the configured typing speed',
  );
  expect(rules['M-09']).toContain('It still counts as a user record for timestamp health');
});

it('defines leverage as agent hours divided by your hours, not typing time', () => {
  expect(rules['M-08']).toContain('Leverage = agent hours (M-05) ÷ your hours');
  expect(rules['M-08']).toContain(
    'Agent hours and your hours both cover the same whole local days',
  );
  expect(rules['M-08']).not.toContain('M-07');
  expect(ruleSummary('M-08')).toContain('Agent hours divided by your hours');
});

it('gives every rule one short plain-language summary without contract codes or jargon', () => {
  expect(Object.keys(ruleSummaries).sort()).toEqual(Object.keys(rules).sort());
  for (const id of Object.keys(rules) as RuleId[]) {
    const summary = ruleSummary(id);
    expect(summary.length, id).toBeGreaterThan(0);
    expect(summary.length, id).toBeLessThanOrEqual(SUMMARY_MAX);
    expect(summary, id).not.toMatch(/\b(?:[A-Z]-\d{2}[a-z]?|F\d{2})\b/);
    expect(summary, id).not.toMatch(
      /\b(?:disjoint|union|SHA|event window|gh pr view|enrichment|denominator|canonical)\b/i,
    );
  }
  expect(ruleSummary('M-19')).toBe(
    'Linked pull requests merged in this range, and their effort. A session linked to several PRs is counted once.',
  );
  expect(ruleSummary('toString' as RuleId)).toBe('Definition unavailable');
});
