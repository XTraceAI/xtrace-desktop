import { hours } from '../kit/format';

/**
 * How this app states a continuous measurement — an hour count, a ratio, a
 * mean per day, a span in minutes — wherever one is shown: the Dashboard's
 * tiles, and the Sessions summary tiles and list rows alike.
 *
 * The shared one-decimal scale rounds anything under half a tenth to "0",
 * which would read as "nothing happened" for a session or a range that did a
 * little: one session across 30 day buckets is 0.03/day, and a two-second span
 * is 0.03 minutes. Neither is nothing, and these pages use "0" for exactly one
 * thing — a measured zero. So a positive the scale cannot show says that it is
 * below the scale instead.
 *
 * This is not a new precision policy: the scale, its one decimal and every
 * value reaching it are unchanged, and the same shape already reads a small
 * cost as `<$0.01`. An unmeasured value never arrives here at all, because
 * `MetricCell` renders its reason instead of calling a formatter.
 */
const SHOWN_SCALE = 0.05;

export const continuous = (value: number): string =>
  value > 0 && value < SHOWN_SCALE ? '<0.1' : hours(value);
