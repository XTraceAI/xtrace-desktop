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
  /** Words shown in place of the dash when there is no value to show. */
  placeholder?: string;
  /** Plain words shown instead of the rule's definition in the tile's tip. */
  definition?: string;
}

/** A reason as it reads before a further note: closed with a full stop. */
const closed = (text: string) => (/[.!?…]$/.test(text.trim()) ? text.trim() : `${text.trim()}.`);

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
  placeholder,
  definition,
}: StatTileProps) {
  // Words in place of the dash already say why there is no number.
  const showPlaceholder = placeholder !== undefined && !isMeasured(value);
  // A tile with no number says why in its definition, once, before any note;
  // the dash then needs no native tooltip of its own repeating it.
  const why = isMeasured(value) || !reason || showPlaceholder ? undefined : closed(reason);
  const context =
    [why, tip && reason && closed(tip) === why ? undefined : tip].filter(Boolean).join(' ') ||
    undefined;
  return (
    <RulePopover ruleId={ruleId} context={context} text={definition}>
      <button type="button" className="xt-stat-tile">
        <span className="xt-stat-label-group">
          <MetricIcon name={icon} tone={iconTone} />
          <span className="xt-stat-label">{label}</span>
        </span>
        <span className="xt-stat-row">
          <span className="xt-stat-value">
            {showPlaceholder ? (
              // The cell keeps a number's size, so the smaller words sit on
              // the same baseline as the other tiles' values.
              <span className="xt-metric-cell" style={{ fontSize: 18 }}>
                <span className="xt-stat-placeholder">{placeholder}</span>
              </span>
            ) : (
              <MetricCell
                value={value}
                format={format}
                reason={reason}
                size={18}
                reasonTitle={false}
              />
            )}
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
