import { count, UNMEASURED } from '../kit/format';

/**
 * Agent time, the one way the app writes it — the Sessions list, a session's
 * own page and timeline, a pull request's sessions, the Dashboard's effort
 * chart and its activity lanes — as hours and the whole minutes that remain:
 * 192.8 minutes reads `3 h 13 m`, 52 minutes `0 h 52 m`. Prose uses the same
 * value in words (`spoken`).
 *
 * Only the layout changes. The value is the row's own M-05 `agent_ms`, and it
 * is rounded exactly once, to a whole minute; the split into hours happens
 * after that, so 59.6 minutes carries into `1 h 0 m` and never reads
 * `0 h 60 m`.
 *
 * Two small cases keep their meaning: a positive span under half a minute
 * reads `<1 m`, never the zero of an idle window, and a measured zero reads
 * `0 h 0 m`. An unmeasured session never reaches this: the cell renders its
 * reason instead.
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

const unit = (value: string, one: string, many: string) => `${value} ${value === '1' ? one : many}`;

export function agentDuration(agentMs: number): AgentDuration {
  // Not a duration at all: said as unknown, never as `NaN h` or `-1 h -1 m`.
  if (!Number.isFinite(agentMs) || agentMs < 0)
    return { visible: UNMEASURED, spoken: 'not measured', exact: UNMEASURED };
  const exact = `${count(agentMs)} ms`;
  const minutes = Math.round(agentMs / MS_PER_MINUTE);
  if (minutes === 0 && agentMs > 0) return { visible: '<1 m', spoken: 'less than 1 minute', exact };
  const whole = Math.floor(minutes / 60);
  const minuteText = String(minutes % 60);
  const hourText = count(whole);
  return {
    visible: `${hourText} h ${minuteText} m`,
    spoken:
      whole === 0
        ? unit(minuteText, 'minute', 'minutes')
        : `${unit(hourText, 'hour', 'hours')} ${unit(minuteText, 'minute', 'minutes')}`,
    exact,
  };
}

/** Agent time as a compact label: `agentDuration(ms).visible`. */
export const agentTime = (agentMs: number) => agentDuration(agentMs).visible;
