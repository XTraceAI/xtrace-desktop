import { expect, it } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { delta, percent } from '../../kit/format';
import { fromPoints, ruleId, tileDelta, usd, windowLabel, zoneLabel } from './present';

const report = (fixture as FixtureExport).dashboards[0];

it('converts report percentage points to formatter fractions exactly once', () => {
  expect(percent(fromPoints(50))).toBe('50%');
  expect(percent(fromPoints(0))).toBe('0%');
  expect(percent(fromPoints(66.7))).toBe('66.7%');
  expect(fromPoints(null)).toBeNull();
  expect(fromPoints(Number.NaN)).toBeNull();
  const tile = { ...report.tiles.agent_hours, delta: { previous: 1, pct: 50, suppressed: false } };
  expect(delta(tileDelta(tile))).toBe('▲50%');
  expect(tileDelta({ ...tile, delta: { ...tile.delta, suppressed: true } })).toBeUndefined();
});

it('formats API-equivalent dollars without inventing precision', () => {
  expect(usd(0)).toBe('$0.00');
  expect(usd(0.001)).toBe('<$0.01');
  expect(usd(1234.567)).toBe('$1,234.57');
});

it('falls back to a known rule definition and system zone labels', () => {
  expect(ruleId('M-09', 'M-05')).toBe('M-09');
  expect(ruleId('not-a-rule', 'M-05')).toBe('M-05');
  const local = { ...report.window, timezone: 'system-local' };
  expect(zoneLabel(local)).toBe('system time zone');
  expect(windowLabel(local)).toMatch(/2026/);
  expect(windowLabel({ ...report.window, timezone: 'Not/AZone' })).toMatch(/2026/);
});
