import { useId, useRef, useState, type ReactNode } from 'react';
import { Popover } from './Popover';
import { ruleText, type RuleId } from './rules';
import '../styles/metrics.css';

export function RulePopover({
  ruleId,
  context,
  className = '',
  children,
}: {
  ruleId: RuleId;
  context?: string;
  className?: string;
  children: (props: { 'aria-describedby': string; onClick: () => void }) => ReactNode;
}) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const hovering = useRef(false);
  const [open, setOpen] = useState(false);
  return (
    <span
      ref={anchor}
      className={`xt-rule-anchor ${className}`}
      onPointerEnter={() => {
        hovering.current = true;
        setOpen(true);
      }}
      onPointerLeave={(event) => {
        hovering.current = false;
        if (!event.currentTarget.contains(document.activeElement)) setOpen(false);
      }}
      onFocus={() => setOpen(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget) && !hovering.current) setOpen(false);
      }}
    >
      {children({ 'aria-describedby': id, onClick: () => setOpen(true) })}
      <Popover
        id={id}
        open={open}
        onOpenChange={setOpen}
        anchorRef={anchor}
        restoreFocus={false}
        role="tooltip"
        className="xt-rule-popover"
      >
        <span>
          {ruleId} · {ruleText(ruleId)}
        </span>
        {context && <span className="xt-rule-context">{context}</span>}
      </Popover>
    </span>
  );
}
