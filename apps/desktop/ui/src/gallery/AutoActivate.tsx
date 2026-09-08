import { useEffect, useRef, type ReactNode } from 'react';

/** Exercise the real invoker for components whose open state is internal. */
export function AutoActivate({ selector, children }: { selector: string; children: ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const frame = requestAnimationFrame(() =>
      root.current?.querySelector<HTMLElement>(selector)?.click(),
    );
    return () => cancelAnimationFrame(frame);
  }, [selector]);
  return (
    <div ref={root} style={{ height: '100%' }}>
      {children}
    </div>
  );
}
