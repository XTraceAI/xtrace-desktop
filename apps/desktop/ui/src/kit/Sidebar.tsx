import { useId, useState, type ReactNode } from 'react';
import type { AccountUsage } from '../data/generated/AccountUsage';
import type { LocalUpdateControls } from '../data/DataSource';
import { AccountUsageWidget } from './AccountUsageWidget';
import { BrandMark } from './BrandMark';
import { HubPopover } from './HubPopover';
import { LocalUpdatesPopover } from './LocalUpdatesPopover';
import { createPopoverHandle, Popover, PopoverTrigger } from './Popover';
import { sidebarIcons } from './sidebar-icons';
import type { Theme } from '../theme/ThemeProvider';
import '../styles/sidebar.css';

export type SidebarKey = 'dashboard' | 'sessions' | 'prs' | 'rulebook' | 'leaderboard' | 'team';
export type SidebarIcon = SidebarKey | 'hub' | 'theme' | 'settings';
export interface SurfaceStatus {
  host: string;
  /** Raw discovered surface identifier; null means the source did not report it. */
  surface: string | null;
  status: 'capturing' | 'not-capturing' | 'unknown';
  reason?: string;
}
/** One host's last scan, worded by the caller. It is never plugin capture coverage. */
export interface LocalIndexHost {
  /** Host identifier as reported, such as `claude`. */
  host: string;
  state: string;
  /** A last scan the caller wants noticed; nothing is inferred from `state`. */
  attention?: boolean;
  reason?: string;
}
/**
 * The local index as the caller describes it. Every word and tone is supplied:
 * nothing here decides what is healthy, and no status is derived from the
 * listener or the capture surfaces.
 */
export interface LocalIndexStatus {
  /** Short state word for the compact trigger, shown after “index ·”. */
  label: string;
  /** Full state name for the panel and the trigger's accessible name. */
  title: string;
  /** `live` draws the green dot, `attention` the warning dot, `idle` the plain one. */
  tone: 'live' | 'attention' | 'idle';
  summary: string;
  /** Qualifications the short label cannot carry; every one is shown in full. */
  notes?: string[];
  hosts: LocalIndexHost[];
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
  accountUsage?: AccountUsage;
  accountUsageFailed?: boolean;
  accountUsageRefreshing?: boolean;
  accountUsageRefreshEnabled?: boolean;
  onRefreshAccountUsage?: () => void;
  surfaces?: SurfaceStatus[];
  /** The optional plugin receiver. A port is shown only when the caller knows one. */
  listener: { status: 'listening'; port?: number } | { status: 'off' | 'unknown' };
  /**
   * When supplied, the status row describes the local index and its panel keeps
   * the plugin receiver and capture coverage as separate lines. Omitted, the row
   * stays the plugin listener and its panel the capture surfaces.
   */
  localIndex?: LocalIndexStatus;
  version: string;
  updateLabel?: string;
  localUpdates?: LocalUpdateControls;
  /** Caller-owned release status/actions beside the version. */
  versionExtra?: ReactNode;
  theme: Theme;
  onToggleTheme: () => void;
  onSettings?: () => void;
  /** Total top padding. Native shells can reserve their traffic-light region. */
  topInset?: number;
  nativeChrome?: boolean;
  icons?: Partial<Record<SidebarIcon, ReactNode>>;
}

const surfaceLabels = {
  capturing: 'Capturing',
  'not-capturing': 'Not capturing',
  unknown: 'Unknown',
};
const receiverLabels = { listening: 'Listening', off: 'Off', unknown: 'Unknown' };

export function Sidebar({
  activeKey,
  onNavigate,
  rulebookCount = 0,
  leaderboardEnabled = false,
  showTeam = false,
  hubConnected = false,
  teamLabel,
  onConnectHub,
  accountUsage,
  accountUsageFailed = false,
  accountUsageRefreshing = false,
  accountUsageRefreshEnabled = true,
  onRefreshAccountUsage = () => {},
  surfaces = [],
  listener,
  localIndex,
  version,
  updateLabel,
  localUpdates,
  versionExtra,
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
  const updatesId = useId();
  const [updatesHandle] = useState(createPopoverHandle);
  const [updatesOpen, setUpdatesOpen] = useState(false);
  const listening = listener.status === 'listening';
  // A port is printed only when the caller supplied one; none is ever assumed.
  const port = listener.status === 'listening' ? listener.port : undefined;
  // The one status row: the local index when the caller describes it, and
  // otherwise the plugin listener exactly as before.
  const statusRow = localIndex
    ? {
        name: `Local index: ${localIndex.title}`,
        hint: 'View local index and plugin receiver status',
        dot: `xt-status-${localIndex.tone}`,
        text: `index · ${localIndex.label}`,
      }
    : {
        name: 'Capture status',
        hint: 'View capture status by surface',
        dot: listening ? 'xt-status-live' : '',
        text: port !== undefined ? `plugin · :${port}` : `plugin · ${listener.status}`,
      };
  const surfaceRows = surfaces.map((surface) => (
    <div className="xt-surface-row" key={JSON.stringify([surface.host, surface.surface])}>
      <span>
        {surface.host} · {surface.surface ?? 'Unknown surface'}
      </span>
      <span className={`xt-capture-${surface.status}`}>{surfaceLabels[surface.status]}</span>
      {surface.reason && <small>{surface.reason}</small>}
    </div>
  ));
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
        <AccountUsageWidget
          usage={accountUsage}
          failed={accountUsageFailed}
          refreshing={accountUsageRefreshing}
          refreshEnabled={accountUsageRefreshEnabled}
          onRefresh={onRefreshAccountUsage}
        />
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
              aria-label={statusRow.name}
              title={statusRow.hint}
            >
              <i className={statusRow.dot} aria-hidden="true" />
              <span>{statusRow.text}</span>
            </PopoverTrigger>
            <div className="xt-sidebar-version">
              <span
                className="xt-version-row"
                title={`v${version}${updateLabel ? ` · ${updateLabel}` : ''}`}
              >
                <i aria-hidden="true" />
                <span>
                  v{version}
                  {updateLabel ? ` · ${updateLabel}` : ''}
                </span>
                {localUpdates && (
                  <PopoverTrigger
                    handle={updatesHandle}
                    type="button"
                    className="xt-updates-trigger"
                  >
                    Updates
                  </PopoverTrigger>
                )}
              </span>
              <div className="xt-sidebar-version-extra">{versionExtra}</div>
            </div>
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
          if (open) {
            setHubOpen(false);
            setUpdatesOpen(false);
          }
        }}
        handle={captureHandle}
        positionAnchor={hubPosition}
        side="right"
        align="end"
        offset={14}
        className="xt-capture-popover"
        role="dialog"
        aria-label={localIndex ? 'Local index and plugin status' : 'Capture by surface'}
      >
        {localIndex ? (
          <div className="xt-index-content">
            <section className="xt-surface-status xt-index-status">
              <h2>Local index</h2>
              <p className={`xt-index-state xt-index-${localIndex.tone}`}>{localIndex.title}</p>
              <p>{localIndex.summary}</p>
              {localIndex.notes?.map((note, index) => (
                <p className="xt-index-note" key={index}>
                  {note}
                </p>
              ))}
            </section>
            <section className="xt-surface-status">
              <h2>Last scan by host</h2>
              {localIndex.hosts.length === 0 && (
                <p className="xt-sidebar-empty">No host scan reported</p>
              )}
              {localIndex.hosts.map((row, index) => (
                <div className="xt-surface-row" key={`${row.host}:${index}`}>
                  <span>{row.host}</span>
                  <span className={row.attention ? 'xt-index-attention' : undefined}>
                    {row.state}
                  </span>
                  {row.reason && <small>{row.reason}</small>}
                </div>
              ))}
              {localIndex.hosts.length > 0 && (
                <small className="xt-index-footnote">
                  A complete scan read the history it found. It is not plugin capture coverage.
                </small>
              )}
            </section>
            {/* The receiver and what plugins delivered are separate facts from
                the index above, and neither is inferred from the other. */}
            <section className="xt-surface-status">
              <h2>Plugin · optional</h2>
              <div className="xt-surface-row">
                <span>Plugin receiver</span>
                <span className={listening ? 'xt-index-live' : 'xt-capture-unknown'}>
                  {port !== undefined ? `Listening · :${port}` : receiverLabels[listener.status]}
                </span>
              </div>
              {surfaces.length === 0 ? (
                <div className="xt-surface-row">
                  <span>Plugin delivery</span>
                  <span className="xt-capture-unknown">Unknown</span>
                </div>
              ) : (
                surfaceRows
              )}
              <small className="xt-index-footnote">
                Local history is indexed without the plugin.
                {listening && ' A listening receiver is not proof that a plugin delivers to it.'}
              </small>
            </section>
            {onSettings && (
              <button
                type="button"
                tabIndex={0}
                className="xt-index-settings"
                onClick={() => {
                  setCaptureOpen(false);
                  onSettings();
                }}
              >
                Index details in Settings
              </button>
            )}
          </div>
        ) : (
          <section className="xt-surface-status">
            <h2>Capture by surface</h2>
            {surfaces.length === 0 && <p className="xt-sidebar-empty">Capture status unknown</p>}
            {surfaceRows}
          </section>
        )}
      </Popover>
      <HubPopover
        id={hubId}
        open={hubOpen}
        onOpenChange={(open) => {
          setHubOpen(open);
          if (open) {
            setCaptureOpen(false);
            setUpdatesOpen(false);
          }
        }}
        handle={hubHandle}
        positionAnchor={hubPosition}
        connected={hubConnected}
        teamLabel={teamLabel}
        onConnect={onConnectHub}
      />
      {localUpdates && (
        <LocalUpdatesPopover
          id={updatesId}
          open={updatesOpen}
          onOpenChange={(open) => {
            setUpdatesOpen(open);
            if (open) {
              setHubOpen(false);
              setCaptureOpen(false);
            }
          }}
          handle={updatesHandle}
          positionAnchor={hubPosition}
          controls={localUpdates}
        />
      )}
    </aside>
  );
}
