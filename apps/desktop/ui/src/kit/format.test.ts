import { expect, it } from 'vitest';
import { count, delta, hours, minutes, percent, tokens } from './format';

it('formats the declared token, duration, count and fraction examples deterministically', () => {
  expect(tokens(15_100_000)).toBe('15.1M');
  expect(tokens(3_720_000_000)).toBe('3.72B');
  expect(tokens(400_000)).toBe('0.4M');
  expect(tokens(1_250)).toBe('1.25K');
  expect(hours(62.34)).toBe('62.3');
  expect(minutes(1.75)).toBe('1.8');
  expect(count(4639)).toBe('4,639');
  expect(percent(0.18)).toBe('18%');
  expect(delta(0.18)).toBe('▲18%');
  expect(delta(-0.09)).toBe('▼9%');
});

it('keeps missing/nonfinite input distinct from measured zero in every formatter', () => {
  for (const format of [tokens, hours, minutes, count, percent, delta]) {
    for (const value of [null, undefined, NaN, Infinity, -Infinity])
      expect(format(value)).toBe('—');
    expect(format(0)).toBe(format(-0));
    expect(format(0)).not.toBe('—');
  }
  expect(tokens(0)).toBe('0');
  expect(delta(0)).toBe('0%');
  expect(percent(Number.MAX_VALUE)).not.toMatch(/∞|Infinity|NaN/);
});
