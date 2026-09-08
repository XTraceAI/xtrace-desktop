import { RulePopover } from './RulePopover';
import { ruleText, type RuleId } from './rules';

export function RuleChip({ ruleId, size = 'normal' }: { ruleId: RuleId; size?: 'normal' | 'sm' }) {
  return (
    <RulePopover ruleId={ruleId}>
      {(trigger) => (
        <button
          {...trigger}
          type="button"
          tabIndex={0}
          className="xt-rule-chip"
          data-size={size}
          title={`${ruleId} · ${ruleText(ruleId)}`}
          aria-label={`Definition ${ruleId}`}
        >
          {ruleId}
        </button>
      )}
    </RulePopover>
  );
}
