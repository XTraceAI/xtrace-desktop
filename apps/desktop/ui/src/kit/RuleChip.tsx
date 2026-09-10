import { RulePopover } from './RulePopover';
import type { RuleId } from './rules';

export function RuleChip({ ruleId, size = 'normal' }: { ruleId: RuleId; size?: 'normal' | 'sm' }) {
  return (
    <RulePopover ruleId={ruleId}>
      <button
        type="button"
        className="xt-rule-chip"
        data-size={size}
        aria-label={`Definition ${ruleId}`}
      >
        {ruleId}
      </button>
    </RulePopover>
  );
}
