import '../styles/topbar.css';

const knownHosts = {
  claude: { label: 'Claude', image: 'claude.svg' },
  codex: { label: 'Codex', image: 'codex.webp' },
  cursor: { label: 'Cursor', image: 'cursor.png' },
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
      {known ? <img src={`/hosts/${known.image}`} alt="" /> : <span aria-hidden="true">?</span>}
    </span>
  );
}
