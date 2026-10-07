export type MetricValue = number | string | null | undefined;
type NumericValue = number | null | undefined;
export const UNMEASURED = '—';
const decimal = new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 });
const compact = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 });
const percentage = new Intl.NumberFormat('en-US', { style: 'percent', maximumFractionDigits: 1 });
const integer = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });
const finite = (value: NumericValue): value is number =>
  typeof value === 'number' && Number.isFinite(value);
const zero = (value: number) => (Object.is(value, -0) ? 0 : value);

export function isMeasured(value: MetricValue): value is number | string {
  return typeof value === 'string' ? value.trim().length > 0 : finite(value);
}

export function tokens(value: NumericValue): string {
  if (!finite(value)) return UNMEASURED;
  const magnitude = Math.abs(value);
  const [divisor, suffix]: [number, string] =
    magnitude >= 1e9
      ? [1e9, 'B']
      : magnitude >= 1e5
        ? [1e6, 'M']
        : magnitude >= 1e3
          ? [1e3, 'K']
          : [1, ''];
  return `${compact.format(zero(value / divisor))}${suffix}`;
}
export function hours(value: NumericValue): string {
  return finite(value) ? decimal.format(zero(value)) : UNMEASURED;
}
export const minutes = hours;
export function count(value: NumericValue): string {
  return finite(value) ? integer.format(zero(value)) : UNMEASURED;
}
export function percent(value: NumericValue): string {
  return finite(value) ? percentage.format(zero(value)) : UNMEASURED;
}
export function delta(value: NumericValue): string {
  if (!finite(value)) return UNMEASURED;
  return `${value > 0 ? '▲' : value < 0 ? '▼' : ''}${percent(Math.abs(value))}`;
}

const cents = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});
const wholeDollars = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 0,
});
/** Where cents stop: a value that rounds to $100.00 or more is whole dollars. */
const WHOLE_FROM = 99.995;

/**
 * Money, the one way the app writes it: whole dollars from $100 (`$250`,
 * `$1,235`), cents below (`$12.40`), and `<$0.01` for a positive amount too
 * small for a cent, so it never reads as nothing. A measured zero is `$0.00`.
 */
export function usd(value: number): string {
  const amount = Object.is(value, -0) ? 0 : value;
  if (amount > 0 && amount < 0.005) return '<$0.01';
  return Math.abs(amount) >= WHOLE_FROM ? wholeDollars.format(amount) : cents.format(amount);
}
