import { Tooltip } from '@base-ui/react/tooltip';
import { useId, type ReactElement } from 'react';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { ruleSummary, type RuleId } from './rules';
import '../styles/metrics.css';

/**
 * Hover waits this long before a definition opens, so passing the pointer
 * over a row of tiles does not flash one popover after another. Keyboard
 * focus still opens it at once: Base UI applies the delay to hover only.
 */
export const RULE_OPEN_DELAY_MS = 450;

/**
 * Passive definition text: the rule's one-line summary, then an optional short
 * note about this value. Base UI owns hover, focus, Escape, and positioning.
 */
export function RulePopover({
  ruleId,
  context,
  text,
  children,
}: {
  ruleId: RuleId;
  context?: string;
  /** Plain words shown instead of the rule's own definition. */
  text?: string;
  children: ReactElement;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={children}
        tabIndex={0}
        aria-describedby={id}
        delay={RULE_OPEN_DELAY_MS}
        closeOnClick={false}
      />
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="bottom"
          align="start"
          sideOffset={8}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover xt-rule-definition">
            <span>{text ?? ruleSummary(ruleId)}</span>
            {context && <span className="xt-rule-context">{context}</span>}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
