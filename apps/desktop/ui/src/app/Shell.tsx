import { matchRoutes, Outlet, useLocation, useNavigate } from 'react-router';
import { useData } from '../data/DataProvider';
import { Sidebar, type SidebarKey } from '../kit/Sidebar';
import { TopBar } from '../kit/TopBar';
import { useTheme } from '../theme/ThemeProvider';
import { useAppInfo } from './useAppInfo';
import { pages } from './routes';
import { useShellShortcuts } from './useShellShortcuts';
import '../styles/shell.css';

export function Shell() {
  const location = useLocation();
  const navigate = useNavigate();
  const { source, eventError } = useData();
  const { theme, toggle } = useTheme();
  const info = useAppInfo();
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
              App data could not be loaded.{' '}
              <button type="button" className="refresh-button" onClick={() => void info.refetch()}>
                Refresh
              </button>
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
