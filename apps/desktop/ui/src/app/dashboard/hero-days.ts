import type { DashboardDay } from '../../data/generated/DashboardDay';

/** How one day's two measurements are drawn: a height, or nothing to draw. */
export type BarState = 'measured' | 'unknown';

export interface HeroBar {
  state: BarState;
  /** `null` when unknown; a measured zero stays `0`. */
  value: number | null;
  /** Share of the shared peak, 0 when there is no positive peak to share. */
  ratio: number;
}
export interface HeroDay {
  date: string;
  agent: HeroBar;
  human: HeroBar;
}
export interface HeroSeries {
  days: HeroDay[];
  /** The one peak both series are drawn against; `null` when nothing positive. */
  peak: number | null;
  /** False when every day's human estimate is unknown, as M-07 reports it. */
  humanMeasured: boolean;
}

const bar = (value: number | null, peak: number | null): HeroBar =>
  value === null || !Number.isFinite(value)
    ? { state: 'unknown', value: null, ratio: 0 }
    : { state: 'measured', value, ratio: peak !== null && peak > 0 ? value / peak : 0 };

/**
 * Agent and estimated human hours share one scale, because the pair is the
 * comparison: drawing each against its own peak would make an hour of human
 * time look like an hour of agent time. The peak is the largest measured value
 * in either series, so every bar is a real fraction of a day that happened.
 *
 * Human time is unknown for a whole range or none of it — M-07 cannot classify
 * part of a window — so an unmeasured human series is stated once rather than
 * drawn thirty times as an empty bar that could be read as zero.
 */
export function heroSeries(days: readonly DashboardDay[]): HeroSeries {
  const measured = (value: number | null) =>
    value !== null && Number.isFinite(value) ? value : null;
  const peak = days.reduce<number | null>((top, day) => {
    const values = [measured(day.agent_hours), measured(day.human_hours_est)];
    return values.reduce<number | null>(
      (best, v) => (v === null ? best : Math.max(best ?? 0, v)),
      top,
    );
  }, null);
  return {
    peak,
    humanMeasured: days.some((day) => measured(day.human_hours_est) !== null),
    days: days.map((day) => ({
      date: day.date,
      agent: bar(measured(day.agent_hours), peak),
      human: bar(measured(day.human_hours_est), peak),
    })),
  };
}
