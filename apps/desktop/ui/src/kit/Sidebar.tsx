import { useId, useState, type ReactNode } from 'react';
import { HostGlyph } from './HostGlyph';
import { BrandMark } from './BrandMark';
import { HubPopover } from './HubPopover';
import { createPopoverHandle, Popover, PopoverTrigger } from './Popover';
import { sidebarIcons } from './sidebar-icons';
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

const hostLabels = { claude: 'Claude Code', codex: 'Codex', cursor: 'Cursor' };
const compactTokens = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
});
const millionTokens = new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 });
const formatTokens = (tokens: number) =>
  tokens >= 100000 ? `${millionTokens.format(tokens / 1000000)}M` : compactTokens.format(tokens);
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
  const [hubHandle] = useState(createPopoverHandle);
  const [hubPosition, setHubPosition] = useState<HTMLSpanElement | null>(null);
  const [hubOpen, setHubOpen] = useState(false);
  const captureId = useId();
  const [captureHandle] = useState(createPopoverHandle);
  const [captureOpen, setCaptureOpen] = useState(false);
  const listening = listener.status === 'listening';
  const icon = (key: SidebarIcon) => (
    <span className="xt-sidebar-icon" aria-hidden="true">
      {icons[key] ?? sidebarIcons[key]}
    </span>
  );
  const navItem = (key: SidebarKey, label: string, disabled = false) => (
    <button
      type="button"
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
        <BrandMark size={22} />
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
            <h2>Usages</h2>
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
                  {row.glyph ?? <HostGlyph host={row.host} />}
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
                  {measured ? formatTokens(row.tokens!) : '—'}
                </span>
              </div>
            );
          })}
        </section>
        <PopoverTrigger
          handle={hubHandle}
          type="button"
          className="xt-hub-button"
          aria-label="XTrace Hub"
        >
          {icon('hub')}
          <span>Cloud and Team</span>
        </PopoverTrigger>
        <div className="xt-sidebar-status">
          <span ref={setHubPosition} className="xt-hub-position" aria-hidden="true" />
          <div className="xt-version-status">
            <PopoverTrigger
              handle={captureHandle}
              type="button"
              className="xt-capture-trigger"
              aria-label="Capture status"
              title="View capture status by surface"
            >
              <i className={listening ? 'xt-status-live' : ''} aria-hidden="true" />
              <span>
                {listening ? `plugin · :${listener.port}` : `plugin · ${listener.status}`}
              </span>
            </PopoverTrigger>
            <span title={`v${version}${updateLabel ? ` · ${updateLabel}` : ''}`}>
              <i aria-hidden="true" />
              <span>
                v{version}
                {updateLabel ? ` · ${updateLabel}` : ''}
              </span>
            </span>
          </div>
          <div className="xt-sidebar-actions">
            <button
              type="button"
              className="xt-icon-button"
              aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} appearance`}
              onClick={onToggleTheme}
            >
              {icon('theme')}
            </button>
            <button
              type="button"
              className="xt-icon-button"
              aria-label="Settings"
              disabled={!onSettings}
              onClick={onSettings}
            >
              {icon('settings')}
            </button>
          </div>
        </div>
      </div>
      <Popover
        id={captureId}
        open={captureOpen}
        onOpenChange={(open) => {
          setCaptureOpen(open);
          if (open) setHubOpen(false);
        }}
        handle={captureHandle}
        positionAnchor={hubPosition}
        side="right"
        align="end"
        offset={14}
        className="xt-capture-popover"
        role="dialog"
        aria-label="Capture by surface"
      >
        <section className="xt-surface-status">
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
      </Popover>
      <HubPopover
        id={hubId}
        open={hubOpen}
        onOpenChange={(open) => {
          setHubOpen(open);
          if (open) setCaptureOpen(false);
        }}
        handle={hubHandle}
        positionAnchor={hubPosition}
        connected={hubConnected}
        teamLabel={teamLabel}
        onConnect={onConnectHub}
      />
    </aside>
  );
}
