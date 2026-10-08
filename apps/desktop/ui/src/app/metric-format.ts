import { hours, UNMEASURED } from '../kit/format';

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

/**
 * Hands-off time (M-09), the one way it is written wherever it is shown — a
 * stretch's length on a session's timeline, a session's or a pull request's
 * median, the Dashboard's median, p90 and daily line: minutes on the shared
 * scale, `3.2 min`. It is its own quantity, so it is never written as agent
 * time (`0 h 3 m`). Not measured reads `—`.
 */
export const handsOffTime = (minutes: number): string =>
  Number.isFinite(minutes) && minutes >= 0 ? `${continuous(minutes)} min` : UNMEASURED;

/** The same value read aloud: `3.2 minutes`, `1 minute`. */
export const handsOffSpoken = (minutes: number): string => {
  if (!(Number.isFinite(minutes) && minutes >= 0)) return 'not measured';
  const shown = continuous(minutes);
  return shown === '<0.1'
    ? 'less than 0.1 minutes'
    : `${shown} ${shown === '1' ? 'minute' : 'minutes'}`;
};
