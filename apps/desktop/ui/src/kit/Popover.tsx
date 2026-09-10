import { Popover as Primitive } from '@base-ui/react/popover';
import type { HTMLAttributes } from 'react';
import { useSurfaceTheme } from '../theme/ThemeProvider';

export const createPopoverHandle = Primitive.createHandle;
export const PopoverTrigger = Primitive.Trigger;
export const PopoverClose = Primitive.Close;

export interface PopoverProps extends HTMLAttributes<HTMLDivElement> {
  id: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  handle: Primitive.Handle<unknown>;
  /** Optional geometry target; the registered trigger still owns focus return. */
  positionAnchor?: HTMLElement | null;
  side?: 'bottom' | 'right';
  align?: 'start' | 'end';
  offset?: number;
}

export function Popover({
  open,
  onOpenChange,
  handle,
  positionAnchor,
  side = 'bottom',
  align = 'start',
  offset = 8,
  className = '',
  children,
  ...props
}: PopoverProps) {
  const theme = useSurfaceTheme();
  return (
    <Primitive.Root handle={handle} open={open} onOpenChange={onOpenChange}>
      <Primitive.Portal data-theme={theme}>
        <Primitive.Positioner
          className="xt-popover-positioner"
          anchor={positionAnchor}
          positionMethod="fixed"
          side={side}
          align={align}
          sideOffset={offset}
          collisionPadding={8}
        >
          <Primitive.Popup {...props} className={`xt-popover ${className}`}>
            {children}
          </Primitive.Popup>
        </Primitive.Positioner>
      </Primitive.Portal>
    </Primitive.Root>
  );
}
