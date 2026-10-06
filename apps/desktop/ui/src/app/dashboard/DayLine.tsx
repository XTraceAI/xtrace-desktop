import { Tooltip } from '@base-ui/react/tooltip';
import { useId } from 'react';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { dayLabel } from './effort-chart';
import type { DayValue } from './overview';
import '../../styles/overview.css';

/** The plot's own coordinate box; the SVG stretches it to the tile. */
const W = 100;
const H = 100;
/** Room above the highest value, so the line never touches the top. */
const HEADROOM = 1.12;

/** Date labels counted back from the last day, so today is always named. */
export function lineTicks(count: number) {
  const step = count <= 8 ? 2 : count <= 15 ? 4 : 7;
  const ticks: number[] = [];
  for (let index = count - 1; index >= 0; index -= step) ticks.unshift(index);
  return ticks;
}
const shortDate = (date: string) => date.slice(5);

/**
 * One series, one day per point, on a scale from zero: a 2px line with a
 * light area under it, broken where a day has no value, and a dashed line at
 * the range's own value. Each day is a hover target whose card states its
 * value; the whole chart is one image whose name lists every day.
 */
export function DayLine({
  days,
  average,
  format,
  name,
  averageName,
}: {
  days: readonly DayValue[];
  /** The range's own value, drawn dashed; null draws nothing. */
  average: number | null;
  format: (value: number) => string;
  /** What the series is, e.g. `Leverage by day`. */
  name: string;
  /** What the dashed line is, e.g. `range 5.8×`. */
  averageName: string;
}) {
  const theme = useSurfaceTheme();
  const clip = useId();
  const count = days.length;
  const values = days.map((day) => day.value).filter((v): v is number => v !== null);
  const top = Math.max(...values, average ?? 0, 0) * HEADROOM || 1;
  const x = (index: number) => ((index + 0.5) / count) * W;
  const y = (value: number) => H - (value / top) * H;
  // Unbroken runs of measured days.
  const runs: { index: number; value: number }[][] = [];
  days.forEach((day, index) => {
    if (day.value === null) return;
    const last = runs.at(-1);
    if (last && last.at(-1)!.index === index - 1) last.push({ index, value: day.value });
    else runs.push([{ index, value: day.value }]);
  });
  const label = `${name}, ${days.length > 0 ? `${dayLabel(days[0]!.date)} to ${dayLabel(days.at(-1)!.date)}` : 'no days'}: ${days
    .map((day) => `${dayLabel(day.date)} ${day.value === null ? 'none' : format(day.value)}`)
    .join(', ')}. Dashed line: ${averageName}.`;
  return (
    <div className="xt-dayline" data-testid="day-line">
      <div className="xt-dayline-plot" role="img" aria-label={label}>
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
          <defs>
            <clipPath id={clip}>
              <rect x="0" y="0" width={W} height={H} />
            </clipPath>
          </defs>
          <g clipPath={`url(#${clip})`}>
            {runs
              .filter((run) => run.length > 1)
              .map((run) => (
                <path
                  key={`a${run[0]!.index}`}
                  className="xt-dayline-area"
                  d={`M${x(run[0]!.index)},${H} ${run.map((p) => `L${x(p.index)},${y(p.value)}`).join(' ')} L${x(run.at(-1)!.index)},${H} Z`}
                />
              ))}
            {average !== null && (
              <line
                className="xt-dayline-average"
                x1="0"
                x2={W}
                y1={y(average)}
                y2={y(average)}
                vectorEffect="non-scaling-stroke"
              />
            )}
            {runs
              .filter((run) => run.length > 1)
              .map((run) => (
                <polyline
                  key={`l${run[0]!.index}`}
                  className="xt-dayline-line"
                  points={run.map((p) => `${x(p.index)},${y(p.value)}`).join(' ')}
                  vectorEffect="non-scaling-stroke"
                />
              ))}
          </g>
        </svg>
        {days.map((day, index) => (
          <Tooltip.Root key={day.date} disableHoverablePopup>
            <Tooltip.Trigger
              delay={0}
              closeOnClick={false}
              render={
                <span
                  className="xt-dayline-day"
                  style={{ left: `${(index / count) * 100}%`, width: `${100 / count}%` }}
                />
              }
            >
              {day.value !== null && (
                <span
                  className="xt-dayline-dot"
                  // A day alone between gaps has no line to show it.
                  data-alone={
                    (days[index - 1]?.value ?? null) === null &&
                    (days[index + 1]?.value ?? null) === null
                      ? true
                      : undefined
                  }
                  style={{ top: `${(y(day.value) / H) * 100}%` }}
                />
              )}
            </Tooltip.Trigger>
            <Tooltip.Portal data-theme={theme}>
              <Tooltip.Positioner
                className="xt-rule-positioner"
                positionMethod="fixed"
                side="top"
                sideOffset={4}
                collisionPadding={8}
              >
                <Tooltip.Popup className="xt-rule-popover xt-effort-tip" aria-hidden="true">
                  <span className="xt-effort-tip-head">
                    <span>{dayLabel(day.date)}</span>
                    <b>{day.value === null ? 'none' : format(day.value)}</b>
                  </span>
                </Tooltip.Popup>
              </Tooltip.Positioner>
            </Tooltip.Portal>
          </Tooltip.Root>
        ))}
      </div>
      <div className="xt-dayline-axis" aria-hidden="true">
        {lineTicks(count).map((index) => (
          <span key={index} style={{ left: `${((index + 0.5) / count) * 100}%` }}>
            {shortDate(days[index]!.date)}
          </span>
        ))}
      </div>
    </div>
  );
}
