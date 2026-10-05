import { Tooltip } from '@base-ui/react/tooltip';
import { useId, type ReactNode } from 'react';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { DetailDialog } from './DetailDialog';

/**
 * A panel's unresolved data, stated as one small triangle at the end of its
 * header rather than a line in its body. It is drawn only when the panel has
 * something unresolved, so it never marks healthy data.
 *
 * The triangle is a real button whose name carries the count, so the fact is
 * one tab stop away without opening anything. Hovering or focusing it reads
 * the gist — the counts and their scope — in a passive tooltip, as the
 * definitions beside card titles do; clicking, tapping or pressing it opens the
 * whole explanation in the Dashboard's detail dialog, which owns Escape,
 * scrolling long content inside the window and returning focus to the
 * triangle. The tooltip holds no control, so nothing in it needs the pointer.
 */
export function IssueFlag({
  label,
  gist,
  title,
  testId,
  children,
}: {
  /** The button's accessible name: what is unresolved, with its count. */
  label: string;
  /** Read on hover and focus: the counts and what they cover. */
  gist: ReactNode;
  /** The dialog's title. */
  title: string;
  testId?: string;
  children: ReactNode;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  return (
    <DetailDialog
      trigger={<IssueTriangle />}
      label={label}
      title={title}
      className="xt-env-modal"
      triggerClassName="xt-issue-flag"
      testId={testId}
      wrapTrigger={(button) => (
        <Tooltip.Root disableHoverablePopup>
          <Tooltip.Trigger render={button} aria-describedby={id} delay={0} />
          <Tooltip.Portal data-theme={theme}>
            <Tooltip.Positioner
              className="xt-rule-positioner"
              positionMethod="fixed"
              side="bottom"
              align="end"
              sideOffset={8}
              collisionPadding={8}
            >
              <Tooltip.Popup
                id={id}
                role="tooltip"
                className="xt-rule-popover xt-issue-gist"
                data-testid={testId && `${testId}-gist`}
              >
                {gist}
              </Tooltip.Popup>
            </Tooltip.Positioner>
          </Tooltip.Portal>
        </Tooltip.Root>
      )}
    >
      {children}
    </DetailDialog>
  );
}

function IssueTriangle() {
  return (
    <svg
      width={13}
      height={13}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M10.3 3.9 1.8 18.4A2 2 0 0 0 3.5 21.4h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
      <path d="M12 9.5v4.5M12 17.5v.01" />
    </svg>
  );
}
