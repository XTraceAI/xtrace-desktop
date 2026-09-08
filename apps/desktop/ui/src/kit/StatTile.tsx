import { MetricCell, type MetricCellProps } from './MetricCell';
import { delta as formatDelta, isMeasured } from './format';
import { MetricIcon, type MetricIconName, type MetricTone } from './metric-icons';
import { RulePopover } from './RulePopover';
import { ruleText, type RuleId } from './rules';

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
    <RulePopover ruleId={ruleId} context={tip} className="xt-stat-anchor">
      {(trigger) => (
        <button
          {...trigger}
          type="button"
          tabIndex={0}
          className="xt-stat-tile"
          title={`${ruleId} · ${ruleText(ruleId)}${tip ? `\n${tip}` : ''}`}
        >
          <MetricIcon name={icon} tone={iconTone} />
          <span className="xt-stat-content">
            <span className="xt-stat-label">
              <span className="xt-stat-label-text">{label}</span>
              <span className="xt-rule-chip" data-size="sm" aria-hidden="true">
                {ruleId}
              </span>
            </span>
            <span className="xt-stat-row">
              <MetricCell value={value} format={format} reason={reason} size={18} />
              {unit && <span className="xt-stat-unit">{unit}</span>}
              {isMeasured(value) && typeof delta === 'number' && Number.isFinite(delta) && (
                <span className="xt-stat-delta" data-tone={deltaTone}>
                  {formatDelta(delta)}
                </span>
              )}
            </span>
          </span>
          {aside && (
            <span className="xt-stat-aside" title={aside}>
              {aside}
            </span>
          )}
        </button>
      )}
    </RulePopover>
  );
}
