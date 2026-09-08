import { matchRoutes, Outlet, useLocation, useNavigate } from 'react-router';
import { useData } from '../data/DataProvider';
import { Sidebar, type SidebarIcon, type SidebarKey } from '../kit/Sidebar';
import { TopBar } from '../kit/TopBar';
import { Icon, type IconName } from '../kit/icons';
import { useTheme, type ThemePreference } from '../theme/ThemeProvider';
import { useAppInfo, useDbCounts } from './useAppInfo';
import { pages } from './routes';
import { useShellShortcuts } from './useShellShortcuts';
import '../styles/shell.css';

const iconNames: Partial<Record<SidebarIcon, IconName>> = {
  dashboard: 'dashboard',
  sessions: 'sessions',
  prs: 'prs',
  rulebook: 'rulebook',
  leaderboard: 'leaderboard',
  hub: 'cloud',
  theme: 'moon',
  settings: 'gear',
};
const icons = Object.fromEntries(
  Object.entries(iconNames).map(([key, name]) => [key, <Icon key={key} name={name} />]),
);
export function Shell() {
  const location = useLocation();
  const navigate = useNavigate();
  const { source, eventError } = useData();
  const { theme, toggle, preference, setPreference } = useTheme();
  const info = useAppInfo();
  const counts = useDbCounts();
  const page = matchRoutes(pages, location)?.at(-1);
  const current = page?.route;
  const nativeMac = source.kind === 'native' && navigator.platform.startsWith('Mac');
  useShellShortcuts();
  function onNavigate(key: SidebarKey) {
    void navigate(`/${key}`);
  }
  return (
    <div className={`xt-app-shell${nativeMac ? ' native-mac' : ''}`}>
      <Sidebar
        activeKey={current?.active}
        onNavigate={onNavigate}
        leaderboardEnabled
        hosts={[]}
        surfaces={[]}
        listener={{ status: info.data?.listening === false ? 'off' : 'unknown' }}
        version={info.data?.version ?? '…'}
        theme={theme}
        onToggleTheme={toggle}
        onSettings={() => void navigate('/settings')}
        topInset={nativeMac ? 74 : 16}
        nativeChrome={nativeMac}
        icons={icons}
      />
      <div className="xt-shell-main">
        <TopBar
          crumb={current?.crumb ?? 'not-found'}
          subcrumb={location.pathname === '/rulebook/fires' ? 'fires' : page?.params.ruleId}
          showRange={false}
          nativeDrag={nativeMac}
          right={
            <>
              {info.data?.fixture && (
                <span className="xt-fixture-badge" role="status">
                  fixture {info.data.fixture}
                </span>
              )}
              <label className="xt-appearance">
                Appearance{' '}
                <select
                  aria-label="Appearance"
                  value={preference}
                  onChange={(event) => setPreference(event.target.value as ThemePreference)}
                >
                  <option value="system">System</option>
                  <option value="dark">Dark</option>
                  <option value="light">Light</option>
                </select>
              </label>
              {source.kind !== 'preview' && (
                <button
                  type="button"
                  tabIndex={0}
                  className="refresh-button"
                  data-tauri-drag-region="false"
                  disabled={info.isFetching || counts.isFetching}
                  onClick={() => {
                    void info.refetch();
                    void counts.refetch();
                  }}
                >
                  Refresh
                </button>
              )}
            </>
          }
        />
        <main className="xt-shell-outlet">
          {source.kind === 'preview' ? (
            <p className="xt-shell-notice">
              Browser preview · Open the desktop app to read local data.
            </p>
          ) : info.isPending ? (
            <p role="status" className="xt-shell-notice">
              Reading app data…
            </p>
          ) : info.isError ? (
            <p role="alert" className="xt-shell-notice">
              App data could not be loaded. Use Refresh to try again.
            </p>
          ) : null}
          {eventError && (
            <p role="alert" className="xt-shell-notice">
              Live updates are unavailable.
            </p>
          )}
          <Outlet />
        </main>
      </div>
    </div>
  );
}
