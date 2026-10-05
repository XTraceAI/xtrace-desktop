import { useState, type CSSProperties, type ReactNode } from 'react';
import { LoadingMark } from './LoadingMark';
import '../styles/startup.css';

/** A start this quick shows nothing, so a fast launch does not flash. */
const REVEAL_AFTER_MS = 180;

/**
 * The window's content while the app is still opening: the animated mark
 * with what is being waited for underneath. Opening and connecting are two
 * separate mounts; the reveal is timed from page load, so the second does not
 * fade in again.
 */
export function StartupSplash({ children }: { children: ReactNode }) {
  const [style] = useState(
    () => ({ '--xt-startup-reveal': `${REVEAL_AFTER_MS - performance.now()}ms` }) as CSSProperties,
  );
  return (
    <div className="xt-startup" style={style} data-tauri-drag-region>
      <LoadingMark />
      <p role="status" className="xt-startup-text">
        {children}
      </p>
    </div>
  );
}
