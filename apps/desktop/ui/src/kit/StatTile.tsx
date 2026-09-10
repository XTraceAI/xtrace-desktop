import { MetricCell, type MetricCellProps } from './MetricCell';
import { delta as formatDelta, isMeasured } from './format';
import { MetricIcon, type MetricIconName, type MetricTone } from './metric-icons';
import { RulePopover } from './RulePopover';
import type { RuleId } from './rules';

export interface StatTileProps extends Pick<MetricCellProps, 'value' | 'format' | 'reason'> {
  label: string;
  ruleId: RuleId;
  icon: MetricIconName;
  iconTone?: MetricTone;
  unit?: string;
  delta?: number | null;
  deltaTone?: 'good' | 'bad';
  aside?: string;
  tip?: string;
}

export function StatTile({
  label,
  ruleId,
  icon,
  iconTone,
  value,
  format,
  reason,
  unit,
  delta,
  deltaTone = 'good',
  aside,
  tip,
}: StatTileProps) {
  return (
    <RulePopover ruleId={ruleId} context={tip}>
      <button type="button" className="xt-stat-tile">
        <span className="xt-stat-label-group">
          <MetricIcon name={icon} tone={iconTone} />
          <span className="xt-stat-label">{label}</span>
        </span>
        <span className="xt-stat-row">
          <span className="xt-stat-value">
            <MetricCell value={value} format={format} reason={reason} size={18} />
            {unit && <span className="xt-stat-unit">{unit}</span>}
            {isMeasured(value) && typeof delta === 'number' && Number.isFinite(delta) && (
              <span className="xt-stat-delta" data-tone={deltaTone}>
                {formatDelta(delta)}
              </span>
            )}
          </span>
          {aside && (
            <span className="xt-stat-aside" title={aside}>
              {aside}
            </span>
          )}
        </span>
      </button>
    </RulePopover>
  );
}
