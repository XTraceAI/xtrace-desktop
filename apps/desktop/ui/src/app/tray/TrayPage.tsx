import { useQuery } from '@tanstack/react-query';
import { useEffect, useLayoutEffect, useMemo, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { TodaySummary } from '../../data/generated/TodaySummary';
import { queryKeys } from '../../data/query-client';
import { calendarDay, clock } from '../../kit/clock';
import { UNMEASURED } from '../../kit/format';
import { agentTime } from '../agent-duration';
import { AccountUsageWidget } from '../../kit/AccountUsageWidget';
import { accountUsageQueryOptions } from '../account-usage-query';
import { isLiveSessionHost, useLiveSessionStatus } from '../live-session-status';
import { LiveSessionBadge, LiveSessionHost } from '../LiveSessionBadge';
import { displayTitle, repoName, shortId } from '../session-context';
import { listedLanes, notListed, sessionKey } from '../dashboard/lanes';
import { verifiedParent } from '../session-parent';
import { useSessionTitles } from '../session-titles';
import '../../styles/sidebar.css';
import { plural, usd } from '../dashboard/present';
import { LiveUpdatesNotice } from '../LiveUpdatesNotice';
import '../../styles/tray.css';

/** setTimeout's delay is a signed 32-bit millisecond count. */
const MAX_DELAY = 2 ** 31 - 1;

/** The report zone is an IANA name, or `system-local` when the OS names none. */
const zoneOf = (summary: TodaySummary) => {
  if (summary.timezone === 'system-local') return undefined;
  try {
    new Intl.DateTimeFormat(undefined, { timeZone: summary.timezone });
    return summary.timezone;
  } catch {
    return undefined;
  }
};
const observedAt = (summary: TodaySummary) =>
  clock(summary.observed_ms, { timeZone: zoneOf(summary) });
/** The report's own local date, never re-derived from this machine's clock. */
const dateLabel = (summary: TodaySummary) => calendarDay(summary.date, 'weekday-day');

/** The price is an API estimate, not a subscription or tool bill. */
const COST_NOTE = 'Estimated at public API prices; subscriptions and tool fees are not included.';

export function costText(summary: TodaySummary) {
  const { cost } = summary;
  const coverage = `${cost.priced_observations} of ${plural(cost.selected_observations, 'response')} priced`;
  switch (cost.state) {
    case 'priced':
      return {
        value: cost.total_usd === null ? UNMEASURED : usd(cost.total_usd),
        title: `${COST_NOTE} ${coverage}.${cost.total_usd === null ? ' Today’s total is unavailable.' : ''}`,
      };
    case 'partial':
      return {
        value: UNMEASURED,
        title: `${COST_NOTE} Today’s total is unavailable. Partial ${usd(cost.priced_subtotal_usd)}, not a total · ${coverage}.`,
      };
    case 'unpriced':
      return {
        value: UNMEASURED,
        title: `${COST_NOTE} Today’s total is unavailable · ${coverage}.`,
      };
    case 'none_recorded':
      return {
        value: UNMEASURED,
        title: `${COST_NOTE} No responses recorded yet today; nothing to price.`,
      };
  }
}

export function agentText(summary: TodaySummary) {
  const { agent } = summary;
  return {
    value: agentTime(agent.active_ms),
    meta: agent.sessions === 0 ? 'No activity' : plural(agent.sessions, 'session'),
  };
}

function Header({ children }: { children?: ReactNode }) {
  return (
    <header className="xt-tray-header">
      <span className="xt-tray-brand">
        <img src="/sidebar-mark.png" width={20} height={20} alt="" />
        XTrace
      </span>
      {children}
    </header>
  );
}

function Today({ children }: { children: ReactNode }) {
  const { source } = useData();
  const query = useQuery({
    queryKey: queryKeys.today,
    queryFn: () => source.today(),
    // Every open reads again; a hidden popover reads nothing.
    refetchOnMount: 'always',
  });
  const { data, refetch } = query;
  const nextMidnight = data?.next_midnight_ms;
  // One timer while visible: the day this summary describes ends at its next
  // local midnight, and the figures then belong to a new date.
  useEffect(() => {
    if (nextMidnight === undefined) return;
    const timer = setTimeout(
      () => void refetch(),
      Math.min(MAX_DELAY, Math.max(0, nextMidnight - Date.now())),
    );
    return () => clearTimeout(timer);
  }, [nextMidnight, refetch]);

  if (query.isError && !data)
    return (
      <>
        <Header />
        <div role="alert" className="xt-tray-card xt-tray-message">
          <span>Today’s figures could not be read.</span>
          <button type="button" className="xt-tray-retry" onClick={() => void refetch()}>
            Retry
          </button>
        </div>
        {children}
      </>
    );
  if (!data)
    return (
      <>
        <Header />
        <p role="status" className="xt-tray-card xt-tray-message">
          Reading today…
        </p>
        {children}
      </>
    );
  const cost = costText(data);
  const humanNote = `${data.human.active_ms === null ? 'Who sent some messages is unknown' : 'Estimated from your messages'}. ${data.human.break_minutes} min break length.`;
  const agent = agentText(data);
  return (
    <>
      <Header>
        <p
          className="xt-tray-status"
          data-testid="tray-observed"
          title={data.timezone === 'system-local' ? 'System time zone' : data.timezone}
        >
          <time dateTime={data.date}>{dateLabel(data)}</time> · as of{' '}
          <time dateTime={new Date(data.observed_ms).toISOString()}>{observedAt(data)}</time>
        </p>
      </Header>
      {query.isError && (
        <div role="alert" className="xt-tray-stale">
          <span>Could not refresh; showing the figures above.</span>
          <button type="button" className="xt-tray-retry" onClick={() => void refetch()}>
            Retry
          </button>
        </div>
      )}
      <div className="xt-tray-tiles">
        <section
          className="xt-tray-card xt-tray-tile"
          aria-label="Today’s agent hours"
          title={
            data.agent.sessions === 0 ? 'No agent activity recorded today' : `Across ${agent.meta}`
          }
        >
          <span className="xt-tray-label">Today · agent</span>
          <span className="xt-tray-value">{agent.value}</span>
          <span className="xt-tray-meta">{agent.meta}</span>
        </section>
        <section
          className="xt-tray-card xt-tray-tile"
          aria-label="Today’s human hours"
          title={humanNote}
          aria-description={humanNote}
        >
          <span className="xt-tray-label">Today · human</span>
          <span className="xt-tray-value">
            {data.human.active_ms === null ? UNMEASURED : agentTime(data.human.active_ms)}
          </span>
          <span className="xt-tray-meta">
            {data.human.active_ms === null ? 'Unknown' : 'Estimate'}
          </span>
        </section>
        <section
          className="xt-tray-card xt-tray-tile"
          aria-label="Today’s cost"
          title={cost.title}
          aria-description={cost.title}
        >
          <span className="xt-tray-label">Today · cost</span>
          <span className="xt-tray-value">{cost.value}</span>
        </section>
      </div>
      {children}
    </>
  );
}

/** Only mounted while the popover is visible, including the widget's reset timer. */
function Usage() {
  const { source } = useData();
  const query = useQuery(accountUsageQueryOptions(source));
  return (
    <AccountUsageWidget
      usage={query.isError ? undefined : query.data}
      failed={source.kind === 'preview' || query.isError}
      refreshing={query.isFetching && query.data === undefined}
      refreshEnabled={false}
      failureMessage={
        source.kind !== 'preview'
          ? 'Account usage could not be read. Open XTrace Desktop to refresh.'
          : undefined
      }
      onRefresh={() => {}}
    />
  );
}

const RECENT_LIMIT = 3;
const RECENT_KEY = ['sessions', 'tray-recent', 7] as const;
function RecentSessions() {
  const { source } = useData();
  const query = useQuery({
    queryKey: RECENT_KEY,
    queryFn: () =>
      source.sessionsList(
        { sort: 'recently_active', search: '', hosts: null, withPrs: false },
        null,
        7,
      ),
    enabled: source.kind !== 'preview',
    refetchOnMount: 'always',
  });
  // First indexed page only. Reuse the list's decision about which sessions
  // may be shown, then keep main sessions before limiting the visible rows.
  // No scan for every session currently running is performed.
  const rows = useMemo(() => {
    if (query.isError || !query.data) return [];
    const returned = query.data.rows;
    const byKey = new Map(returned.map((row) => [sessionKey(row.host, row.id), row]));
    const context = new Map(
      (query.data.referenced_parents ?? []).map((row) => [
        sessionKey(row.host, row.session_id),
        row,
      ]),
    );
    return listedLanes(
      returned.map((row) => ({
        key: sessionKey(row.host, row.id),
        sessionId: row.id,
        host: row.host,
        row,
      })),
      (lane) => verifiedParent(lane.row),
      ({ session_id, host }) => {
        const key = sessionKey(host, session_id);
        const found = byKey.get(key) ?? context.get(key);
        return notListed(
          found,
          host,
          verifiedParent({ id: session_id, parent: found?.parent }) !== null,
        );
      },
    )
      .filter((lane) => verifiedParent(lane.row) === null)
      .slice(0, RECENT_LIMIT)
      .map((lane) => lane.row);
  }, [query.data, query.isError]);
  const readTitle = useSessionTitles(
    'tray-recent',
    rows.map((row) => ({ id: row.id, version: row.record_count })),
  );
  const liveStatus = useLiveSessionStatus(
    'tray-recent',
    rows.filter((row) => isLiveSessionHost(row.host)).map((row) => row.id),
  );
  return (
    <section className="xt-tray-card xt-tray-section" aria-label="Recent sessions">
      <h2 className="xt-tray-heading">Recent sessions</h2>
      <p className="xt-tray-meta">Up to {RECENT_LIMIT} recently active main sessions</p>
      {query.isError ? (
        <p className="xt-tray-unavailable">Recent sessions could not be read.</p>
      ) : query.isPending ? (
        <p className="xt-tray-unavailable">Reading sessions…</p>
      ) : rows.length === 0 ? (
        <p className="xt-tray-unavailable">No recent sessions to show.</p>
      ) : (
        <ul className="xt-tray-sessions">
          {rows.map((row) => {
            const status = liveStatus(row.id);
            const name =
              displayTitle(
                readTitle(row.id, row.record_count) ?? row.title,
                row.automated_review,
              ) ?? `Session ${shortId(row.id)}`;
            return (
              <li key={row.id} className="xt-tray-session">
                <div className="xt-tray-session-top">
                  <span className="xt-tray-session-name" title={`${name} · ${row.id}`}>
                    {name}
                  </span>
                  {isLiveSessionHost(row.host) && status !== 'running' && (
                    <LiveSessionBadge host={row.host} status={status} />
                  )}
                </div>
                <span className="xt-tray-meta xt-tray-session-meta">
                  <LiveSessionHost host={row.host} status={status} />
                  <span>{repoName(row.repo) ?? 'Unknown repository'}</span>
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/**
 * The menu-bar popover: outside the Shell, with no sidebar or top bar. In the
 * native popover window it reads only while shown; elsewhere (fixture preview)
 * it is always shown.
 */
export function TrayPage() {
  const { source } = useData();
  const navigate = useNavigate();
  const tray = source.tray;
  const [open, setOpen] = useState(tray === undefined);

  useLayoutEffect(() => {
    document.documentElement.dataset.surface = 'tray';
    return () => {
      delete document.documentElement.dataset.surface;
    };
  }, []);

  useEffect(() => {
    if (!tray) return;
    let active = true;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => {
      if (active) stops.push(stop);
      else stop();
    };
    // Each show or hide heard bumps the generation. Events heard before the
    // snapshot is requested are older than its answer; only one heard after
    // the request outranks the reply.
    let generation = 0;
    const set = (value: boolean) => {
      generation += 1;
      if (active) setOpen(value);
    };
    // The snapshot is always taken, and only once both listeners are
    // registered: an event lost while either was registering is then covered
    // by the snapshot, whichever listener heard what before it.
    void Promise.allSettled([
      tray.onShown(() => set(true)).then(keep),
      tray.onHidden(() => set(false)).then(keep),
    ]).then(() => {
      if (!active) return;
      const requested = generation;
      void tray.visible().then(
        (visible) => active && generation === requested && setOpen(visible),
        () => {},
      );
    });
    return () => {
      active = false;
      stops.forEach((stop) => stop());
    };
  }, [tray]);

  useEffect(() => {
    if (!tray || !open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void tray.hide().catch(() => {});
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [tray, open]);

  return (
    <main className="xt-tray" aria-label="XTrace today">
      <div className="xt-tray-content">
        {open ? (
          <Today>
            <Usage />
            <RecentSessions />
          </Today>
        ) : (
          <Header />
        )}
      </div>
      {/* This popover's own runtime: the main window's Reconnect cannot repair it. */}
      <LiveUpdatesNotice className="xt-tray-stale" buttonClassName="xt-tray-retry" />
      <button
        type="button"
        className="xt-tray-open"
        onClick={() => {
          if (tray) void tray.openMain().catch(() => {});
          else void navigate('/dashboard');
        }}
      >
        Open XTrace Desktop
      </button>
    </main>
  );
}
