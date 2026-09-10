import { Tooltip } from '@base-ui/react/tooltip';
import { useId, type ReactElement } from 'react';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { ruleText, type RuleId } from './rules';
import '../styles/metrics.css';

/** Passive definition text. Base UI owns hover, focus, Escape, and positioning. */
export function RulePopover({
  ruleId,
  context,
  children,
}: {
  ruleId: RuleId;
  context?: string;
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
        delay={0}
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
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover">
            <span>
              {ruleId} · {ruleText(ruleId)}
            </span>
            {context && <span className="xt-rule-context">{context}</span>}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
