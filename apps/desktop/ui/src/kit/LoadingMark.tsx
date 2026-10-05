import { useState, type CSSProperties } from 'react';
import '../styles/startup.css';

/** One lap of the mark's four cutouts, in milliseconds. Matches startup.css. */
const CYCLE_MS = 1600;

/**
 * The XTrace mark with its four cutouts fading in turn, shown while the app
 * waits on something. Every copy is timed from page load rather than from
 * mount, so one that replaces another (opening, then the first Dashboard
 * read) continues the same lap instead of restarting it.
 */
export function LoadingMark({ size = 56 }: { size?: 32 | 56 }) {
  const [style] = useState(
    () =>
      ({
        '--xt-startup-phase': `${-(performance.now() % CYCLE_MS)}ms`,
        width: size,
        height: size,
        borderRadius: size * 0.16,
      }) as CSSProperties,
  );
  return (
    <span className="xt-startup-mark" style={style} aria-hidden="true">
      <svg viewBox="0 0 96 96">
        {/* Clockwise from the top, so the lap reads as one motion around the mark. */}
        <path className="xt-startup-cut" d="M32 32V0a32 32 0 0 1 32 32z" />
        <path className="xt-startup-cut" d="M64 64V32a32 32 0 0 1 32 32z" />
        <path className="xt-startup-cut" d="M64 64H32a32 32 0 0 0 32 32z" />
        <path className="xt-startup-cut" d="M32 32H0a32 32 0 0 0 32 32z" />
      </svg>
    </span>
  );
}
