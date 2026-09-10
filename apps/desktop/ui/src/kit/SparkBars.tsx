import type { CSSProperties } from 'react';
import '../styles/tables.css';

export interface SparkDay {
  label: string;
  agent: number | null;
  human?: number | null;
}
export function SparkBars({
  days,
  max,
  height = 36,
  single = false,
  label = 'Daily activity',
}: {
  days: readonly SparkDay[];
  max: number;
  height?: 36 | 44;
  single?: boolean;
  label?: string;
}) {
  const ceiling = Number.isFinite(max) && max > 0 ? max : 0;
  const bar = (value: number | null | undefined, series: string, day: string) => {
    const measured = typeof value === 'number' && Number.isFinite(value) && value >= 0;
    const ratio = measured && ceiling > 0 ? Math.min(value / ceiling, 1) : 0;
    const text = measured ? String(value) : 'unmeasured';
    return (
      <span
        role="img"
        aria-label={`${day}, ${series}: ${text}`}
        title={`${day}, ${series}: ${text}`}
        className="xt-spark-bar"
        data-series={series}
        data-zero={(measured && value === 0) || undefined}
        data-unknown={!measured || undefined}
        style={{ height: Math.max(2, ratio * height) }}
      />
    );
  };
  return (
    <div
      className="xt-spark-bars"
      role="group"
      aria-label={label}
      data-single={single || undefined}
      style={{ height, '--bar-width': height === 44 || single ? '9px' : '6px' } as CSSProperties}
    >
      {days.map((day, index) => (
        <span className="xt-spark-day" key={index}>
          {bar(day.agent, single ? 'fires' : 'agent', day.label)}
          {!single && bar(day.human, 'human', day.label)}
        </span>
      ))}
    </div>
  );
}
