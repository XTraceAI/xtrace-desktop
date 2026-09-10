import { isMeasured, type MetricValue } from './format';
import '../styles/metrics.css';

export function Unmeasured({ reason = 'This value was not measured' }: { reason?: string }) {
  return (
    <span className="xt-unmeasured" title={reason}>
      <span aria-hidden="true">—</span>
      <span className="sr-only">Unmeasured: {reason}</span>
    </span>
  );
}

export interface MetricCellProps {
  value?: MetricValue;
  format?: (value: number) => string;
  reason?: string;
  size?: 10.5 | 11 | 18 | 24 | 30;
  align?: 'left' | 'right';
}

export function MetricCell({
  value,
  format = String,
  reason,
  size = 11,
  align = 'left',
}: MetricCellProps) {
  return (
    <span
      className="xt-metric-cell"
      style={{ fontSize: size, textAlign: align, width: align === 'right' ? '100%' : undefined }}
    >
      {isMeasured(value) ? (
        typeof value === 'number' ? (
          format(value)
        ) : (
          value
        )
      ) : (
        <Unmeasured reason={reason} />
      )}
    </span>
  );
}
