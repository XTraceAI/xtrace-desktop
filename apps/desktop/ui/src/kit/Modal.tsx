import { useLayoutEffect, useRef, type ComponentProps, type RefObject } from 'react';

export interface ModalProps extends Omit<
  ComponentProps<'dialog'>,
  'open' | 'onClose' | 'onCancel' | 'ref'
> {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnFocusRef?: RefObject<HTMLElement | null>;
}

/** Supply aria-label or aria-labelledby and an explicit close button in children. */
export function Modal({
  open,
  onOpenChange,
  returnFocusRef,
  onKeyDown,
  className = '',
  children,
  ...props
}: ModalProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const notify = useRef(onOpenChange);
  useLayoutEffect(() => {
    notify.current = onOpenChange;
  });
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const closed = () => {
      if (!dialog.open) notify.current(false);
    };
    dialog.addEventListener('close', closed);
    return () => dialog.removeEventListener('close', closed);
  }, []);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!open) {
      if (dialog.open) dialog.close();
      return;
    }
    const previous = returnFocusRef?.current ?? document.activeElement;
    if (!dialog.open) dialog.showModal();
    return () => {
      if (dialog.open) dialog.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus({ preventScroll: true });
    };
  }, [open, returnFocusRef]);
  return (
    <dialog
      {...props}
      ref={ref}
      tabIndex={-1}
      className={`xt-modal ${className}`}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (event.defaultPrevented || event.key !== 'Tab') return;
        // WebKit can send Tab to browser chrome when macOS skips button focus.
        // Cycle locally; showModal still owns background inertness and Escape.
        const items = [
          ...event.currentTarget.querySelectorAll<HTMLElement>(
            'a[href], button, input, select, textarea, [tabindex], [contenteditable="true"]',
          ),
        ]
          .filter(
            (item) =>
              item.tabIndex >= 0 &&
              !item.matches(':disabled') &&
              !item.closest('[inert]') &&
              item.getClientRects().length > 0 &&
              getComputedStyle(item).visibility !== 'hidden',
          )
          .sort(
            (a, b) =>
              (a.tabIndex || Number.MAX_SAFE_INTEGER) - (b.tabIndex || Number.MAX_SAFE_INTEGER),
          );
        event.preventDefault();
        const index = items.indexOf(document.activeElement as HTMLElement);
        const next = event.shiftKey
          ? index <= 0
            ? items.length - 1
            : index - 1
          : (index + 1) % items.length;
        (items[next] ?? event.currentTarget).focus();
      }}
    >
      {children}
    </dialog>
  );
}
