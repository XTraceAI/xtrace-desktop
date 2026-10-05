import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { matchRoutes, Outlet, useLocation, useNavigate, useSearchParams } from 'react-router';
import { useData, useLiveUpdates } from '../data/DataProvider';
import { Sidebar, type SidebarKey } from '../kit/Sidebar';
import { TopBar, type TimeRange } from '../kit/TopBar';
import { useTheme } from '../theme/ThemeProvider';
import { useAppInfo, useNativeIndexObservation } from './useAppInfo';
import { LiveUpdatesNotice } from './LiveUpdatesNotice';
import { UpdateNotice } from './UpdateNotice';
import { pluginReceiver, sidebarIndexStatus } from './sidebar-index-status';
import { pages } from './routes';
import { useShellShortcuts } from './useShellShortcuts';
import { INVENTORY_VIEW, PRS_VIEW } from './pr-analytics';
import { parseRange, sessionParams } from './session-search';
import { shortId } from './session-context';
import { useTokensByHost, type ShellOutletContext } from './dashboard/range';
import { queryKeys } from '../data/query-client';
import { accountUsageQueryOptions } from './account-usage-query';
import { windowLabel, zoneLabel } from './dashboard/present';
import { useWindowClock, windowClockKeys } from './window-clock';
import '../styles/shell.css';

export function Shell() {
  const location = useLocation();
  const navigate = useNavigate();
  const { source } = useData();
  const queryClient = useQueryClient();
  const { theme, toggle } = useTheme();
  const info = useAppInfo();
  // The sidebar's status row describes the local index from the status the
  // pages already read, observed passively: no timer, listener or report of
  // its own. While this renderer does not hear status events the cached state
  // is shown as last known. The browser preview reads no index at all, so it
  // keeps the plain plugin row.
  const index = useNativeIndexObservation();
  const live = useLiveUpdates();
  const localIndex =
    source.kind === 'preview' ? undefined : sidebarIndexStatus({ ...index, live: live.state });
  const page = matchRoutes(pages, location)?.at(-1);
  const current = page?.route;
  const nativeMac = source.kind === 'native' && navigator.platform.startsWith('Mac');
  // One selected range for the Dashboard, its TopBar presets and the sidebar
  // token totals. A link into a windowed page may name the range it was
  // showing, and that named range wins while it is in the address: the value
  // is derived here rather than copied into state, so back and forward restore
  // the window a page was measured over. An absent or unsupported value falls
  // back to the last range this session chose, which leaves an ordinary
  // `/sessions` link and a mistyped one behaving the same way.
  const [params, setParams] = useSearchParams();
  const [chosen, setChosen] = useState<TimeRange>('7d');
  const addressed = parseRange(params.get(sessionParams.range));
  const range = addressed ?? chosen;
  // A range arrived at through a link becomes this window's chosen range too,
  // so leaving for a page whose address carries none keeps the same window
  // rather than silently reverting to the default.
  useEffect(() => {
    if (addressed) setChosen(addressed);
  }, [addressed]);
  const setRange = (next: TimeRange) => {
    setChosen(next);
    // Only a URL that already carries a range keeps carrying it, so choosing a
    // range never adds a parameter to a plain page address. Replacing the
    // entry keeps range changes out of the back stack, as they were before.
    if (params.has(sessionParams.range)) {
      setParams(
        (previous) => {
          const updated = new URLSearchParams(previous);
          updated.set(sessionParams.range, next);
          return updated;
        },
        { replace: true },
      );
    }
  };
  const tokens = useTokensByHost(range);
  const accountUsage = useQuery(accountUsageQueryOptions(source));
  const [claudeRefreshing, setClaudeRefreshing] = useState(false);
  async function refreshClaudeUsage() {
    if (claudeRefreshing) return;
    setClaudeRefreshing(true);
    // An older passive response must not replace a newer explicit read.
    try {
      await queryClient.cancelQueries({ queryKey: queryKeys.accountUsage, exact: true });
      await queryClient.invalidateQueries({
        queryKey: queryKeys.accountUsage,
        exact: true,
        refetchType: 'none',
      });
      await queryClient.fetchQuery({
        queryKey: queryKeys.accountUsage,
        queryFn: () => source.refreshClaudeUsage(),
        staleTime: 0,
        retry: false,
      });
    } catch {
      // The query records the failure. The widget hides its previous values.
    } finally {
      setClaudeRefreshing(false);
    }
  }
  const outlet: ShellOutletContext = { range };
  // Pages whose numbers are taken over the selected event window own the range
  // control and show which window produced them. One session's page is one of
  // them: its row and its hands-off stretches are both measured over the window,
  // and changing it re-reads those and never the session's text. So is the
  // pull requests report, but not that page's cached inventory view, whose
  // lifetime list the range does not filter.
  const windowed =
    current?.path === '/dashboard' ||
    current?.path === '/sessions' ||
    current?.path === '/sessions/:sessionId' ||
    (current?.path === '/prs' && params.get(PRS_VIEW) !== INVENTORY_VIEW);
  // A mounted screen keeps the window its reports were read over, and no data
  // event re-reads them when only the clock moves, so the visible route's
  // selected-window reads are read again on a coarse clock. The Sessions list
  // follows this clock with only the pages the user has already loaded.
  useWindowClock(
    windowClockKeys({
      path: current?.path,
      sessionId: page?.params.sessionId,
      inventory: params.get(PRS_VIEW) === INVENTORY_VIEW,
      range,
    }),
  );
  // The report window for the selected range; absent until that range has loaded.
  const period = windowed ? tokens.data?.window : undefined;
  useShellShortcuts();
  function onNavigate(key: SidebarKey) {
    void navigate(`/${key}`);
  }
  return (
    <div className={`xt-app-shell${nativeMac ? ' native-mac' : ''}`}>
      <Sidebar
        activeKey={current?.active}
        onNavigate={onNavigate}
        // Leaderboard keeps the kit's disabled "soon" row: this local build has
        // no ranking to show, and /leaderboard is only a named placeholder.
        // A background re-read keeps the values on screen; only the first
        // read and an explicit Refresh show the reading state.
        accountUsage={accountUsage.isError || claudeRefreshing ? undefined : accountUsage.data}
        accountUsageFailed={source.kind === 'preview' || accountUsage.isError}
        accountUsageRefreshing={
          claudeRefreshing || (accountUsage.isFetching && accountUsage.data === undefined)
        }
        accountUsageRefreshEnabled={source.kind !== 'preview'}
        onRefreshAccountUsage={() => void refreshClaudeUsage()}
        // Plugin delivery coverage is not read here, so it stays unknown: host
        // scans are never turned into capture surfaces, and no surface is listed.
        surfaces={[]}
        listener={pluginReceiver(info.data?.listening)}
        localIndex={localIndex}
        version={info.data?.version ?? '…'}
        versionExtra={source.kind === 'native' ? <UpdateNotice /> : undefined}
        theme={theme}
        onToggleTheme={toggle}
        onSettings={() => void navigate('/settings')}
        topInset={nativeMac ? 74 : 16}
        nativeChrome={nativeMac}
      />
      <div className="xt-shell-main">
        <TopBar
          crumb={current?.crumb ?? 'not-found'}
          subcrumb={
            location.pathname === '/rulebook/fires'
              ? 'fires'
              : // No rule record is read, so an address's rule ID is never
                // shown in the chrome as if it named a rule that exists.
                page?.params.ruleId
                ? 'rule'
                : // Enough of the identity to say which session is open, in the
                  // same short form the lists show.
                  page?.params.sessionId && shortId(page.params.sessionId)
          }
          {...(windowed ? { range, onRange: setRange } : { showRange: false as const })}
          nativeDrag={nativeMac}
          right={
            <>
              {period && (
                <span
                  className="xt-topbar-period"
                  data-testid="report-period"
                  title={`${windowLabel(period)} · ${zoneLabel(period)}`}
                >
                  {windowLabel(period)}
                  <span className="sr-only">, time zone {zoneLabel(period)}</span>
                </span>
              )}
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
          <LiveUpdatesNotice className="xt-shell-notice" buttonClassName="refresh-button" />
          <Outlet context={outlet} />
        </main>
      </div>
    </div>
  );
}
