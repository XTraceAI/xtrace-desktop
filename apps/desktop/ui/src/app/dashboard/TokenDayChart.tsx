import type { CSSProperties } from 'react';
import type { DashboardDay } from '../../data/generated/DashboardDay';
import type { MetricTokenCounters } from '../../data/generated/MetricTokenCounters';
import type { ControlTone } from '../../kit/control-tone';
import { tokens as formatTokens } from '../../kit/format';

type Counter = keyof MetricTokenCounters;
const series: { key: Counter; label: string; tone: ControlTone }[] = [
  { key: 'input_tokens', label: 'Fresh input', tone: 'info' },
  { key: 'output_tokens', label: 'Output', tone: 'accent' },
  { key: 'cache_read_tokens', label: 'Cache read', tone: 'success' },
];
const tableColumns: { key: Counter; label: string }[] = [
  ...series,
  { key: 'cache_creation_tokens', label: 'Cache write' },
  { key: 'total_tokens', label: 'Total' },
];

/** A day with no selected responses has nothing to measure; otherwise null is unknown. */
const cell = (day: DashboardDay, key: Counter) => {
  const value = day.tokens.counters[key];
  if (typeof value === 'number') return { state: 'measured' as const, value };
  return day.tokens.selected_responses === 0
    ? { state: 'none' as const, value: null }
    : { state: 'unknown' as const, value: null };
};
const cellText = (day: DashboardDay, key: Counter) => {
  const { state, value } = cell(day, key);
  return state === 'measured'
    ? value.toLocaleString('en-US')
    : state === 'none'
      ? 'no recorded usage'
      : 'unmeasured';
};
const shortDate = (date: string) => {
  const [, month, day] = date.split('-');
  return `${month}-${day}`;
};

/**
 * Small multiples, one row per counter on its own labelled linear scale: fresh input and
 * cache reads are usually orders of magnitude larger than output, which a shared scale hides.
 */
export function TokenDayChart({ days }: { days: readonly DashboardDay[] }) {
  if (days.length === 0) return <p className="xt-dash-empty">No days in this range.</p>;
  const ticks = [days[0], days[Math.floor((days.length - 1) / 2)], days.at(-1)!];
  return (
    <div className="xt-token-chart">
      {series.map(({ key, label, tone }) => {
        // null until a numeric counter exists: no measurement is not a measured zero peak.
        const max = days.reduce<number | null>((top, day) => {
          const { state, value } = cell(day, key);
          return state === 'measured' ? Math.max(top ?? 0, value) : top;
        }, null);
        return (
          <div className="xt-token-series" key={key}>
            <div className="xt-token-series-label">
              <span aria-hidden="true" className="xt-control-tone" data-tone={tone} />
              <span>{label}</span>
              <small>
                {max === null ? 'no recorded measurement' : `measured peak ${formatTokens(max)}`}
              </small>
            </div>
            <div
              className="xt-token-bars"
              role="group"
              aria-label={`${label} tokens per day`}
              style={{ '--days': days.length } as CSSProperties}
            >
              {days.map((day) => {
                const { state, value } = cell(day, key);
                const ratio = state === 'measured' && max !== null && max > 0 ? value / max : 0;
                const text = `${day.date}, ${label.toLowerCase()} tokens: ${cellText(day, key)}`;
                return (
                  <span key={day.date} className="xt-token-slot">
                    <span
                      role="img"
                      aria-label={text}
                      title={text}
                      className="xt-token-bar xt-control-tone"
                      data-tone={tone}
                      data-state={state}
                      data-zero={(state === 'measured' && value === 0) || undefined}
                      style={{ height: state === 'unknown' ? '100%' : `${ratio * 100}%` }}
                    />
                  </span>
                );
              })}
            </div>
          </div>
        );
      })}
      <div className="xt-token-axis" aria-hidden="true">
        <span />
        <span className="xt-token-ticks">
          {ticks.map((day, index) => (
            <span key={index}>{shortDate(day.date)}</span>
          ))}
        </span>
      </div>
      <p className="xt-dash-note">
        Each row uses its own scale; dashed bars are days with unmeasured counters.
      </p>
      <details className="xt-dash-details">
        <summary>Daily token values</summary>
        <div className="xt-dash-table-scroll">
          <table className="xt-dash-table">
            <caption className="sr-only">Recorded tokens per day</caption>
            <thead>
              <tr>
                <th scope="col">Day</th>
                {tableColumns.map((column) => (
                  <th scope="col" key={column.key}>
                    {column.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {days.map((day) => (
                <tr key={day.date}>
                  <th scope="row">{day.date}</th>
                  {tableColumns.map((column) => (
                    <td key={column.key}>{cellText(day, column.key)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}
