import { useLayoutEffect, useRef, type HTMLAttributes, type RefObject } from 'react';

export interface PopoverProps extends Omit<HTMLAttributes<HTMLDivElement>, 'onToggle'> {
  id: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  anchorRef: RefObject<HTMLElement | null>;
  side?: 'bottom' | 'right';
  align?: 'start' | 'end';
  offset?: number;
}

/** Keep this beside its trigger in the DOM: the native top layer preserves ancestry. */
export function Popover({
  open,
  onOpenChange,
  anchorRef,
  side = 'bottom',
  align = 'start',
  offset = 8,
  className = '',
  children,
  ...props
}: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  const notify = useRef(onOpenChange);
  useLayoutEffect(() => {
    notify.current = onOpenChange;
  });

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    function toggled(event: Event) {
      const visible = (event as ToggleEvent).newState === 'open';
      notify.current(visible);
      if (
        !visible &&
        (document.activeElement === document.body || element?.contains(document.activeElement))
      ) {
        anchorRef.current?.focus({ preventScroll: true });
      }
    }
    element.addEventListener('toggle', toggled);
    return () => element.removeEventListener('toggle', toggled);
  }, [anchorRef]);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    // Refs can detach without changing their identity or the controlled open prop.
    if (!open || !anchorRef.current?.isConnected) {
      if (element.matches(':popover-open')) element.hidePopover();
      if (open) notify.current(false);
    }
  });

  useLayoutEffect(() => {
    const element = ref.current;
    const anchor = anchorRef.current;
    if (!open || !element || !anchor?.isConnected) return;
    function position() {
      if (!element || !anchor) return;
      if (!anchor.isConnected) {
        if (element.matches(':popover-open')) element.hidePopover();
        return;
      }
      const rect = anchor.getBoundingClientRect();
      const x =
        side === 'right'
          ? rect.right + offset
          : align === 'end'
            ? rect.right - element.offsetWidth
            : rect.left;
      const y =
        side === 'bottom'
          ? rect.bottom + offset
          : align === 'end'
            ? rect.bottom - element.offsetHeight
            : rect.top;
      element.style.left = `${Math.max(8, Math.min(x, innerWidth - element.offsetWidth - 8))}px`;
      element.style.top = `${Math.max(8, Math.min(y, innerHeight - element.offsetHeight - 8))}px`;
    }
    if (!element.matches(':popover-open')) element.showPopover();
    position();
    const observer = new ResizeObserver(position);
    observer.observe(anchor);
    observer.observe(element);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  }, [open, anchorRef, side, align, offset]);

  return (
    <div {...props} ref={ref} popover="auto" className={`xt-popover ${className}`}>
      {children}
    </div>
  );
}
