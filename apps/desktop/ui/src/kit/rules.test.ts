// @vitest-environment node
import { createHash } from 'node:crypto';
import { expect, it } from 'vitest';
import report from '../../../../../docs/acceptance/metrics/rule-coverage.json';
import { rules, ruleText, type RuleId } from './rules';

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
