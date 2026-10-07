import { HOST_NAMES } from './hosts';
import '../styles/topbar.css';

const knownHosts = {
  claude: { label: HOST_NAMES.claude, image: 'claude.svg' },
  codex: { label: HOST_NAMES.codex, image: 'codex.webp' },
  cursor: { label: HOST_NAMES.cursor, image: 'cursor.png' },
};

export interface HostGlyphProps {
  /** Canonical known key or an unrecognized discovered host; missing is unknown. */
  host?: string | null;
  size?: 16 | 18 | 20 | 30;
  stacked?: boolean;
  /**
   * The name to give the glyph where the text beside it names the host
   * differently on purpose (the usage widget's "Claude" account).
   */
  label?: string;
}

export function HostGlyph({ host, size = 16, stacked = false, label: named }: HostGlyphProps) {
  const known =
    host && Object.hasOwn(knownHosts, host) ? knownHosts[host as keyof typeof knownHosts] : null;
  const label = named ?? known?.label ?? (host?.trim() ? `Unknown host: ${host}` : 'Unknown host');
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
