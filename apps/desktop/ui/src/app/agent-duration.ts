import { count, hours } from '../kit/format';
import { continuous } from './metric-format';

/**
 * The Sessions list's `agent min` cell, stated as hours and the minutes that
 * remain: 192.8 minutes reads `3h12.8m`, 124 minutes `2h04m`.
 *
 * Only the layout changes. The value is the row's own M-05 `agent_ms`, and it
 * is rounded exactly once, by the same one-decimal scale (`hours`) the column
 * used before, to a whole number of tenths of a minute; the split into hours
 * happens after that, so a value the scale shows as 60 minutes carries into
 * `1h00m` and never reads `0h60m`. The tenth is kept wherever the scale shows
 * one, and dropped only where the scale itself would (`3h12m` is 192.0).
 *
 * The page's two small cases keep their meaning (`metric-format.ts`): a
 * positive span too short for the scale reads `<0.1m`, never the zero of an
 * idle window, and a measured zero reads `0h00m`. An unmeasured session never
 * reaches this: the cell renders its reason instead.
 */
export interface AgentDuration {
  /** What the compact cell draws. */
  visible: string;
  /** The same value in words, for a screen reader. */
  spoken: string;
  /** The measurement itself, never rounded: `11,568,000 ms`. */
  exact: string;
}

const MS_PER_MINUTE = 60_000;
const TENTHS_PER_HOUR = 600;

/** Whole tenths of a minute, as the existing scale rounds them. */
const tenthsOf = (minutes: number) => Math.round(Number(hours(minutes).replace(/,/g, '')) * 10);

const unit = (value: string, one: string, many: string) => `${value} ${value === '1' ? one : many}`;

export function agentDuration(agentMs: number): AgentDuration {
  const exact = `${count(agentMs)} ms`;
  const minutes = agentMs / MS_PER_MINUTE;
  if (continuous(minutes) === '<0.1')
    return { visible: '<0.1m', spoken: 'less than 0.1 minutes', exact };
  const tenths = tenthsOf(minutes);
  const whole = Math.floor(tenths / TENTHS_PER_HOUR);
  const rest = tenths % TENTHS_PER_HOUR;
  const restMinutes = Math.floor(rest / 10);
  const restTenth = rest % 10;
  const minuteText = `${restMinutes}${restTenth ? `.${restTenth}` : ''}`;
  const hourText = count(whole);
  return {
    visible: `${hourText}h${minuteText.padStart(restTenth ? 4 : 2, '0')}m`,
    spoken:
      whole === 0
        ? unit(minuteText, 'minute', 'minutes')
        : `${unit(hourText, 'hour', 'hours')} ${unit(minuteText, 'minute', 'minutes')}`,
    exact,
  };
}
