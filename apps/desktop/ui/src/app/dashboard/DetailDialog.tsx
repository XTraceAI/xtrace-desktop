import { useId, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { Modal, ModalClose } from '../../kit/Modal';
import type { RuleId } from '../../kit/rules';
import { DefinitionInfo } from './DefinitionInfo';

/**
 * A secondary detail of the Dashboard, one step away over the page: the
 * trigger is a compact text control that keeps the page's height, and the
 * detail opens in the kit Modal, which owns focus containment, Escape and the
 * return of focus to the trigger. The page's geometry is the same before,
 * during and after, whatever the detail holds.
 *
 * `open` and `onOpenChange` make the dialog controlled by its owner, for a
 * detail that must survive its trigger unmounting while another range loads;
 * otherwise the dialog keeps its own state. `wrapTrigger` hands the trigger to
 * a wrapper that renders it — a passive tooltip that reads the detail's gist
 * on hover and focus — without changing what the trigger opens.
 */
export function DetailDialog({
  trigger,
  label,
  title,
  rule,
  meta,
  className = '',
  triggerClassName = '',
  open: controlled,
  onOpenChange,
  testId,
  wrapTrigger = (button) => button,
  children,
}: {
  trigger: ReactNode;
  /** The trigger's accessible name when its visible text is shorter. */
  label?: string;
  title: string;
  /** The rule the dialog's definition quotes; the control sits beside its title. */
  rule?: RuleId;
  /** Scope or version, read beside the title. */
  meta?: ReactNode;
  className?: string;
  triggerClassName?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  testId?: string;
  wrapTrigger?: (button: ReactElement) => ReactNode;
  children: ReactNode;
}) {
  const [own, setOwn] = useState(false);
  const open = controlled ?? own;
  const setOpen = onOpenChange ?? setOwn;
  const button = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  return (
    <>
      {wrapTrigger(
        <button
          ref={button}
          type="button"
          // Explicit, as the kit's buttons are: WebKit tabs past a button without it.
          tabIndex={0}
          className={`xt-dash-link-button ${triggerClassName}`}
          aria-label={label}
          aria-haspopup="dialog"
          aria-expanded={open}
          data-testid={testId}
          onClick={() => setOpen(true)}
        >
          {trigger}
        </button>,
      )}
      <Modal
        open={open}
        onOpenChange={setOpen}
        returnFocusRef={button}
        aria-labelledby={titleId}
        className={`xt-dash-modal ${className}`}
      >
        {/* Close is the first control in the dialog, so opening it by keyboard
            lands there rather than on the definition, whose tooltip would
            otherwise open at once; it is drawn at the header's end. */}
        <header className="xt-dash-modal-header">
          <h2 id={titleId}>{title}</h2>
          <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
            Close
          </ModalClose>
          {rule && <DefinitionInfo ruleId={rule} name={title} />}
          {meta && <span className="xt-dash-modal-meta">{meta}</span>}
        </header>
        {children}
      </Modal>
    </>
  );
}
