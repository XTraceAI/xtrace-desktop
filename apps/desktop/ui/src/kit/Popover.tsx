import { useLayoutEffect, useRef, type HTMLAttributes, type RefObject } from 'react';

export interface PopoverProps extends Omit<HTMLAttributes<HTMLDivElement>, 'onToggle'> {
  id: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  anchorRef: RefObject<HTMLElement | null>;
  /** Optional geometry target; the invoker remains the focus-return target. */
  positionRef?: RefObject<HTMLElement | null>;
  side?: 'bottom' | 'right';
  align?: 'start' | 'end';
  offset?: number;
}

/** Keep this beside its trigger in the DOM: the native top layer preserves ancestry. */
export function Popover({
  open,
  onOpenChange,
  anchorRef,
  positionRef,
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
    if (
      !open ||
      !anchorRef.current?.isConnected ||
      (positionRef && !positionRef.current?.isConnected)
    ) {
      if (element.matches(':popover-open')) element.hidePopover();
      if (open) notify.current(false);
    }
  });

  useLayoutEffect(() => {
    const element = ref.current;
    if (
      open &&
      element &&
      anchorRef.current?.isConnected &&
      (!positionRef || positionRef.current?.isConnected) &&
      !element.matches(':popover-open')
    ) {
      element.showPopover();
    }
    // Native dismissal precedes its queued toggle event. Unrelated renders
    // must not reopen the panel while the controlled prop catches up.
  }, [open, anchorRef, positionRef]);

  useLayoutEffect(() => {
    // Stable ref objects can point to new DOM nodes after any commit.
    // Rebind geometry and observers to the current targets before paint.
    const element = ref.current;
    const invoker = anchorRef.current;
    const anchor = positionRef ? positionRef.current : invoker;
    if (!open || !element || !invoker?.isConnected || !anchor?.isConnected) return;
    function position() {
      if (!element || !anchor) return;
      if (!anchor.isConnected || !invoker?.isConnected) {
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
    position();
    const observer = new ResizeObserver(position);
    observer.observe(anchor);
    if (invoker !== anchor) observer.observe(invoker);
    observer.observe(element);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  });

  return (
    <div {...props} ref={ref} popover="auto" className={`xt-popover ${className}`}>
      {children}
    </div>
  );
}
