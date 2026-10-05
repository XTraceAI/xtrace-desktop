import { expect, it } from 'vitest';
import { hours } from '../kit/format';
import { continuous } from './metric-format';

it('separates a below-scale positive from the measured zero, and changes nothing else', () => {
  // The only values that move: a positive the one-decimal scale would print as
  // "0", which is the string these pages reserve for a measured zero.
  expect(continuous(0.001)).toBe('<0.1');
  expect(continuous(0.03)).toBe('<0.1');
  expect(continuous(0.049)).toBe('<0.1');
  // A measured zero stays "0", including the negative zero the scale folds in.
  expect(continuous(0)).toBe('0');
  expect(continuous(-0)).toBe('0');
  // From the first value the scale itself shows, this is the scale, unchanged.
  for (const value of [0.05, 0.1, 1.24, 2.75, 1234.5]) expect(continuous(value)).toBe(hours(value));
  expect(continuous(0.05)).toBe('0.1');
  expect(continuous(2.75)).toBe('2.8');
  expect(continuous(1234.5)).toBe('1,234.5');
});
