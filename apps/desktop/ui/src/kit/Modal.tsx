import { Dialog } from '@base-ui/react/dialog';
import type { HTMLAttributes, RefObject } from 'react';
import { useSurfaceTheme } from '../theme/ThemeProvider';

export const ModalClose = Dialog.Close;

export interface ModalProps extends HTMLAttributes<HTMLDivElement> {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnFocusRef?: RefObject<HTMLElement | null>;
}

/** Supply an accessible name and a ModalClose button in children. */
export function Modal({
  open,
  onOpenChange,
  returnFocusRef,
  className = '',
  children,
  ...props
}: ModalProps) {
  const theme = useSurfaceTheme();
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange} disablePointerDismissal>
      <Dialog.Portal data-theme={theme}>
        <Dialog.Backdrop className="xt-modal-backdrop" />
        <Dialog.Popup {...props} finalFocus={returnFocusRef} className={`xt-modal ${className}`}>
          {children}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
