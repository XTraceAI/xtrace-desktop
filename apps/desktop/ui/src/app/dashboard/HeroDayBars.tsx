import type { CSSProperties } from 'react';
import type { DashboardDay } from '../../data/generated/DashboardDay';
import { continuous } from '../metric-format';
import { DetailDialog } from './DetailDialog';
import { heroSeries, type HeroBar } from './hero-days';

const UNMEASURED = 'unmeasured';
const shortDate = (date: string) => {
  const [, month, day] = date.split('-');
  return `${month}-${day}`;
};
const barText = (bar: HeroBar) =>
  bar.state === 'measured' && bar.value !== null ? `${continuous(bar.value)} h` : UNMEASURED;

/**
 * One reported day of the selected range, agent beside estimated human, on the
 * scale both share. The bars are the same M-05 spans and M-07 character estimate the hero's
 * two numbers are made of, split at local midnight, so the row reads as the
 * shape of those totals over time rather than as a separate measurement.
 *
 * It sits on the ratio's line, where the design places it, and every reported
 * day is drawn — including the ones with nothing on them, because a range's
 * empty days are part of what happened in it. A measured zero sits flat on the
 * axis and an unmeasured estimate is dashed, so the two never look alike; the
 * dates and values are stated in full by `HeroDayTable`.
 */
export function HeroDayChart({ days }: { days: readonly DashboardDay[] }) {
  const { days: series, peak, humanMeasured } = heroSeries(days);
  if (series.length === 0) return null;
  return (
    <div className="xt-hero-chart" style={{ '--days': series.length } as CSSProperties}>
      <p className="xt-hero-legend">
        <span>
          <span aria-hidden="true" className="xt-control-tone" data-tone="accent" />
          agent
        </span>
        <span>
          <span aria-hidden="true" className="xt-control-tone" data-tone="warning" />
          human estimated
        </span>
        <small>
          {peak === null
            ? 'no measured day'
            : humanMeasured
              ? `peak ${continuous(peak)} h`
              : `peak ${continuous(peak)} h · estimate ${UNMEASURED}`}
        </small>
      </p>
      <div
        className="xt-hero-bars"
        role="group"
        aria-label="Agent and estimated human hours per day, on one shared scale"
      >
        {series.map((day) => {
          const text = `${day.date}: agent ${barText(day.agent)}, human ${barText(day.human)} estimated`;
          return (
            <span key={day.date} className="xt-hero-day" role="img" aria-label={text} title={text}>
              {(
                [
                  ['accent', day.agent],
                  ['warning', day.human],
                ] as const
              ).map(([tone, bar]) => (
                <span
                  key={tone}
                  aria-hidden="true"
                  className="xt-hero-bar xt-control-tone"
                  data-tone={tone}
                  data-state={bar.state}
                  data-zero={(bar.state === 'measured' && bar.value === 0) || undefined}
                  style={{ height: bar.state === 'unknown' ? '100%' : `${bar.ratio * 100}%` }}
                />
              ))}
            </span>
          );
        })}
      </div>
    </div>
  );
}

/**
 * The same series as dates and values, reachable by keyboard and read in full
 * by a screen reader. It opens over the page from the card's header, so the
 * card keeps its height; the dialog's title carries the range the bars cover.
 */
export function HeroDayTable({ days }: { days: readonly DashboardDay[] }) {
  const { days: series } = heroSeries(days);
  if (series.length === 0) return null;
  return (
    <DetailDialog
      trigger="Daily values"
      title="Daily agent and human hours"
      meta={
        <span className="xt-hero-range">
          {shortDate(series[0].date)} – {shortDate(series[series.length - 1].date)}
        </span>
      }
      className="xt-hero-table"
      testId="hero-day-table"
    >
      {/* The rows scroll inside the dialog; the region takes focus so the
          keyboard reaches every day, in WebKit as in Chromium. */}
      <div
        className="xt-dash-table-scroll"
        role="region"
        aria-label="Daily agent and human hours scroll area"
        tabIndex={0}
      >
        <table className="xt-dash-table">
          <caption className="sr-only">
            Measured agent and estimated human hours for each day of the selected range
          </caption>
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col">Agent</th>
              <th scope="col">Human est.</th>
            </tr>
          </thead>
          <tbody>
            {series.map((day) => (
              <tr key={day.date}>
                <th scope="row">{day.date}</th>
                <td>{barText(day.agent)}</td>
                <td>{barText(day.human)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </DetailDialog>
  );
}
