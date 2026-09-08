import '../styles/topbar.css';

const knownHosts = {
  claude: { label: 'Claude', glyph: 'A' },
  codex: { label: 'Codex', glyph: '◎' },
  cursor: { label: 'Cursor', glyph: '▶' },
};

export interface HostGlyphProps {
  /** Canonical known key or an unrecognized discovered host; missing is unknown. */
  host?: string | null;
  size?: 16 | 18 | 20 | 30;
  stacked?: boolean;
}

export function HostGlyph({ host, size = 16, stacked = false }: HostGlyphProps) {
  const known =
    host && Object.hasOwn(knownHosts, host) ? knownHosts[host as keyof typeof knownHosts] : null;
  const label = known?.label ?? (host?.trim() ? `Unknown host: ${host}` : 'Unknown host');
  return (
    <span
      className="xt-host-glyph-mark"
      data-host={known ? host : 'unknown'}
      data-size={size}
      data-stacked={stacked || undefined}
      role="img"
      aria-label={label}
      title={label}
    >
      <span aria-hidden="true">
        {known?.glyph === '◎' ? (
          <svg
            width="1em"
            height="1em"
            viewBox="0 0 12 12"
            fill="none"
            stroke="currentColor"
            strokeWidth={1.25}
            focusable="false"
          >
            <circle cx={6} cy={6} r={4.75} />
            <circle cx={6} cy={6} r={2.25} />
          </svg>
        ) : (
          (known?.glyph ?? '?')
        )}
      </span>
    </span>
  );
}
