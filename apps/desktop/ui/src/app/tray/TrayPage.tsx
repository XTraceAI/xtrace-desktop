import { useQuery } from '@tanstack/react-query';
import { useEffect, useLayoutEffect, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { TodaySummary } from '../../data/generated/TodaySummary';
import { queryKeys } from '../../data/query-client';
import { calendarDay, clock } from '../../kit/clock';
import { tokens, UNMEASURED } from '../../kit/format';
import { agentTime } from '../agent-duration';
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

export function outputText(summary: TodaySummary) {
  const { output } = summary;
  switch (output.state) {
    case 'recorded':
      return {
        value: tokens(output.output_tokens),
        meta: `${plural(output.selected_responses, 'response')} · ${plural(output.sessions, 'session')}`,
      };
    case 'incomplete':
      return {
        value: UNMEASURED,
        meta: `Some of ${plural(output.selected_responses, 'response')} lack an output count`,
      };
    case 'none_recorded':
      return { value: UNMEASURED, meta: 'No responses recorded yet today' };
  }
}

/** What the visible cost line leaves unsaid: the basis of the estimate, in plain words. */
const COST_NOTE = 'Estimated at public API prices; subscriptions and tool fees are not included.';

export function costText(summary: TodaySummary) {
  const { cost } = summary;
  const coverage = `${cost.priced_observations} of ${plural(cost.selected_observations, 'response')} priced`;
  switch (cost.state) {
    case 'priced':
      return `${usd(cost.total_usd ?? 0)} API-equivalent · ${coverage}`;
    case 'partial':
      return `Partial ${usd(cost.priced_subtotal_usd)}, not a total · ${coverage}`;
    case 'unpriced':
      return `Cost ${UNMEASURED} · ${coverage}`;
    case 'none_recorded':
      return `Cost ${UNMEASURED} · nothing to price`;
  }
}

export function agentText(summary: TodaySummary) {
  const { agent } = summary;
  return {
    value: agentTime(agent.active_ms),
    meta:
      agent.sessions === 0
        ? 'No agent activity recorded today'
        : `across ${plural(agent.sessions, 'session')}`,
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

function Today() {
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
      </>
    );
  if (!data)
    return (
      <>
        <Header />
        <p role="status" className="xt-tray-card xt-tray-message">
          Reading today…
        </p>
      </>
    );
  const output = outputText(data);
  const agent = agentText(data);
  const reason = (key: string) =>
    data.unavailable.find((item) => item.key === key)?.reason ?? 'Unavailable';
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
        <section className="xt-tray-card xt-tray-tile" aria-label="Today’s output tokens">
          <span className="xt-tray-label">Today · output</span>
          <span className="xt-tray-value">{output.value}</span>
          <span className="xt-tray-meta">{output.meta}</span>
          <span className="xt-tray-meta" title={COST_NOTE}>
            {costText(data)}
          </span>
        </section>
        <section className="xt-tray-card xt-tray-tile" aria-label="Today’s agent hours">
          <span className="xt-tray-label">Today · agent</span>
          <span className="xt-tray-value">{agent.value}</span>
          <span className="xt-tray-meta">{agent.meta}</span>
        </section>
      </div>
      <section className="xt-tray-card xt-tray-section" aria-label="Active now">
        <h2 className="xt-tray-heading">Active now</h2>
        <p className="xt-tray-unavailable">{reason('active_now')}</p>
      </section>
      <section className="xt-tray-card xt-tray-section" aria-label="Last rule fire">
        <h2 className="xt-tray-heading">Last rule fire</h2>
        <p className="xt-tray-unavailable">{reason('last_rule_fire')}</p>
      </section>
      <p className="xt-tray-footnote">
        Since local midnight,{' '}
        {data.timezone === 'system-local' ? 'system time zone' : data.timezone}. Agent hours match
        today’s bar on the Dashboard: work that ran past midnight counts from midnight.
      </p>
    </>
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
      {open ? <Today /> : <Header />}
      {/* This popover's own runtime: the main window's Reconnect cannot repair it. */}
      <LiveUpdatesNotice className="xt-tray-stale" buttonClassName="xt-tray-retry" />
      <div className="xt-tray-spacer" />
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
