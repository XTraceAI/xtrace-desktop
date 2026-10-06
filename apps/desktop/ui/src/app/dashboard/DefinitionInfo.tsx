import { RulePopover } from '../../kit/RulePopover';
import type { RuleId } from '../../kit/rules';
import '../../styles/definition.css';

/**
 * A card's definition, one small info control beside its title. Hover, focus
 * and Escape are Base UI's. The button is one tab stop away from the title.
 * A card whose rows need more explanation states it in `context`.
 */
export function DefinitionInfo({
  ruleId,
  name,
  context,
  text,
}: {
  ruleId: RuleId;
  name: string;
  context?: string;
  /** Plain words shown instead of the rule's own definition. */
  text?: string;
}) {
  return (
    <RulePopover ruleId={ruleId} context={context} text={text}>
      <button type="button" className="xt-dash-definition" aria-label={`${name} definition`}>
        <svg
          width={12}
          height={12}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeLinecap="round"
          aria-hidden="true"
          focusable="false"
        >
          <circle cx="12" cy="12" r="9" />
          <path d="M12 11v6M12 7.5v.01" />
        </svg>
      </button>
    </RulePopover>
  );
}
