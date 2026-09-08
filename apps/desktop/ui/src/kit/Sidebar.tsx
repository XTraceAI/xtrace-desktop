import { useId, useRef, useState, type ReactNode } from 'react';
import { BrandMark } from './BrandMark';
import { HubPopover } from './HubPopover';
import type { Theme } from '../theme/ThemeProvider';
import '../styles/sidebar.css';

export type SidebarKey = 'dashboard' | 'sessions' | 'prs' | 'rulebook' | 'leaderboard' | 'team';
export type SidebarIcon = SidebarKey | 'hub' | 'theme' | 'settings';
export interface HostTokens {
  host: 'claude' | 'codex' | 'cursor';
  tokens: number | null;
  /** Display-only fill supplied by the caller; no metric is calculated here. */
  fillPercent: number;
  glyph?: ReactNode;
}
export interface SurfaceStatus {
  host: string;
  /** Raw discovered surface identifier; null means the source did not report it. */
  surface: string | null;
  status: 'capturing' | 'not-capturing' | 'unknown';
  reason?: string;
}
export interface SidebarProps {
  activeKey?: SidebarKey;
  onNavigate: (key: SidebarKey) => void;
  rulebookCount?: number;
  leaderboardEnabled?: boolean;
  showTeam?: boolean;
  hubConnected?: boolean;
  teamLabel?: string;
  onConnectHub?: () => void;
  hosts: HostTokens[];
  tokensCaption?: string;
  surfaces?: SurfaceStatus[];
  listener: { status: 'listening'; port: number } | { status: 'off' | 'unknown' };
  version: string;
  updateLabel?: string;
  theme: Theme;
  onToggleTheme: () => void;
  onSettings?: () => void;
  /** Total top padding. Native shells can reserve their traffic-light region. */
  topInset?: number;
  nativeChrome?: boolean;
  icons?: Partial<Record<SidebarIcon, ReactNode>>;
}

const hostLabels = { claude: 'Claude', codex: 'Codex', cursor: 'Cursor' };
const fallbackIcons: Record<SidebarIcon, string> = {
  dashboard: '⌂',
  sessions: '◷',
  prs: '↗',
  rulebook: '◇',
  leaderboard: '☷',
  team: '◎',
  hub: '☁',
  theme: '◐',
  settings: '⚙',
};
const formatTokens = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
});
const surfaceLabels = {
  capturing: 'Capturing',
  'not-capturing': 'Not capturing',
  unknown: 'Unknown',
};

export function Sidebar({
  activeKey,
  onNavigate,
  rulebookCount = 0,
  leaderboardEnabled = false,
  showTeam = false,
  hubConnected = false,
  teamLabel,
  onConnectHub,
  hosts,
  tokensCaption,
  surfaces = [],
  listener,
  version,
  updateLabel,
  theme,
  onToggleTheme,
  onSettings,
  topInset = 16,
  nativeChrome = false,
  icons = {},
}: SidebarProps) {
  const hubId = useId();
  const hubTrigger = useRef<HTMLButtonElement>(null);
  const hubPosition = useRef<HTMLSpanElement>(null);
  const [hubOpen, setHubOpen] = useState(false);
  const listening = listener.status === 'listening';
  const icon = (key: SidebarIcon) => (
    <span className="xt-sidebar-icon" aria-hidden="true">
      {icons[key] ?? fallbackIcons[key]}
    </span>
  );
  const navItem = (key: SidebarKey, label: string, disabled = false) => (
    <button
      type="button"
      tabIndex={0}
      data-tauri-drag-region="false"
      className="xt-nav-item"
      key={key}
      disabled={disabled}
      aria-current={activeKey === key ? 'page' : undefined}
      onClick={() => onNavigate(key)}
    >
      {icon(key)}
      <span>{label}</span>
      {key === 'rulebook' && Number.isFinite(rulebookCount) && rulebookCount > 0 && (
        <span className="xt-rulebook-count" aria-label={`${rulebookCount} rulebook items`}>
          {rulebookCount}
        </span>
      )}
      {key === 'leaderboard' && !leaderboardEnabled && <span className="xt-soon">soon</span>}
    </button>
  );

  return (
    <aside
      className="xt-sidebar"
      aria-label="Workspace"
      style={{ paddingTop: Number.isFinite(topInset) ? Math.max(16, topInset) : 16 }}
    >
      {nativeChrome && (
        <div className="xt-window-chrome" data-tauri-drag-region="deep" aria-hidden="true" />
      )}
      <div className="xt-sidebar-brand" data-tauri-drag-region={nativeChrome ? 'deep' : undefined}>
        <BrandMark />
      </div>
      <nav aria-label="Main navigation">
        <div className="xt-nav-group">
          <h2>Observe</h2>
          {navItem('dashboard', 'Dashboard')}
          {navItem('sessions', 'Sessions')}
          {navItem('prs', 'Pull requests')}
        </div>
        <div className="xt-nav-group">
          <h2>Govern</h2>
          {navItem('rulebook', 'Rulebook')}
        </div>
        <div className="xt-nav-group">
          <h2>Community</h2>
          {navItem('leaderboard', 'Leaderboard', !leaderboardEnabled)}
          {showTeam && navItem('team', teamLabel ?? 'Team', !hubConnected)}
        </div>
      </nav>
      <div className="xt-sidebar-footer">
        <section className="xt-host-tokens" aria-label="Tokens by host">
          <div className="xt-token-heading">
            <h2>Tokens by host</h2>
            {tokensCaption && <span>{tokensCaption}</span>}
          </div>
          {hosts.length === 0 && <p className="xt-sidebar-empty">No host measurements</p>}
          {hosts.map((row) => {
            const measured = row.tokens !== null && Number.isFinite(row.tokens) && row.tokens >= 0;
            const fill =
              measured && row.tokens !== 0 && Number.isFinite(row.fillPercent)
                ? Math.min(100, Math.max(0, row.fillPercent))
                : 0;
            return (
              <div className={`xt-host-row xt-host-${row.host}`} key={row.host}>
                <span className="xt-host-glyph" aria-hidden="true">
                  {row.glyph ?? <span className="xt-host-dot" />}
                </span>
                <span className="xt-host-name">
                  {hostLabels[row.host]}
                  <span className="xt-host-track" aria-hidden="true">
                    <span style={{ width: `${fill}%` }} />
                  </span>
                </span>
                <span
                  className="xt-host-value"
                  aria-label={`${hostLabels[row.host]} tokens: ${measured ? row.tokens : 'unmeasured'}`}
                >
                  {measured ? formatTokens.format(row.tokens!) : '—'}
                </span>
              </div>
            );
          })}
        </section>
        <section className="xt-surface-status" aria-label="Capture by surface">
          <h2>Capture by surface</h2>
          {surfaces.length === 0 && <p className="xt-sidebar-empty">Capture status unknown</p>}
          {surfaces.map((surface) => (
            <div className="xt-surface-row" key={JSON.stringify([surface.host, surface.surface])}>
              <span>
                {surface.host} · {surface.surface ?? 'Unknown surface'}
              </span>
              <span className={`xt-capture-${surface.status}`}>
                {surfaceLabels[surface.status]}
              </span>
              {surface.reason && <small>{surface.reason}</small>}
            </div>
          ))}
        </section>
        <div className="xt-sidebar-status">
          <span ref={hubPosition} className="xt-hub-position" aria-hidden="true" />
          <button
            ref={hubTrigger}
            type="button"
            tabIndex={0}
            data-tauri-drag-region="false"
            className="xt-icon-button"
            aria-label="XTrace Hub"
            aria-expanded={hubOpen}
            popoverTarget={hubId}
          >
            {icon('hub')}
          </button>
          <div className="xt-version-status">
            <span>
              <i className={listening ? 'xt-status-live' : ''} aria-hidden="true" />
              {listening ? `plugin · :${listener.port}` : `plugin · ${listener.status}`}
            </span>
            <span>
              <i aria-hidden="true" />v{version}
              {updateLabel ? ` · ${updateLabel}` : ''}
            </span>
          </div>
        </div>
        <div className="xt-sidebar-actions">
          <span>{hubConnected ? (teamLabel ?? 'Hub connected') : 'Local workspace'}</span>
          <button
            type="button"
            tabIndex={0}
            data-tauri-drag-region="false"
            className="xt-icon-button"
            aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} appearance`}
            onClick={onToggleTheme}
          >
            {icon('theme')}
          </button>
          <button
            type="button"
            tabIndex={0}
            data-tauri-drag-region="false"
            className="xt-icon-button"
            aria-label="Settings"
            disabled={!onSettings}
            onClick={onSettings}
          >
            {icon('settings')}
          </button>
        </div>
      </div>
      <HubPopover
        id={hubId}
        open={hubOpen}
        onOpenChange={setHubOpen}
        anchorRef={hubTrigger}
        positionRef={hubPosition}
        connected={hubConnected}
        teamLabel={teamLabel}
        onConnect={onConnectHub}
      />
    </aside>
  );
}
