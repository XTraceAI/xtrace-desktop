import { useEffect, useRef, type ReactNode } from 'react';

/** Exercise the real invoker for components whose open state is internal. */
export function AutoActivate({
  selector,
  children,
  hover = false,
}: {
  selector: string;
  children: ReactNode;
  hover?: boolean;
}) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const target = root.current?.querySelector<HTMLElement>(selector);
      // Each document previews its own tooltip without taking keyboard focus
      // away from the other theme or the story index.
      if (hover) target?.dispatchEvent(new MouseEvent('mouseenter'));
      else target?.click();
    });
    return () => cancelAnimationFrame(frame);
  }, [selector, hover]);
  return (
    <div ref={root} style={{ height: '100%' }}>
      {children}
    </div>
  );
}
