import { useQuery } from '@tanstack/react-query';
import { usePurgeEpoch } from '../../data/content-purges';
import { Link } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DashboardDaySession } from '../../data/generated/DashboardDaySession';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import { queryKeys } from '../../data/query-client';
import type { TimeRange } from '../../kit/TopBar';
import { agentDuration, agentTime } from '../agent-duration';
import { displayTitle, shortId } from '../session-context';
import { listAddress, sessionHref } from '../session-search';
import { useSessionTitles } from '../session-titles';
import { plural } from './present';

/** The version a listed session's host title is read by; see `last_activity_at_ms`. */
const titleVersion = (session: DashboardDaySession) => session.last_activity_at_ms ?? 'unknown';

/**
 * A "human + agent" day's sessions box, under a heading that names the day:
 * every session with agent time that day, the most first, each with its time
 * that day and a link that opens it, then how many more there were and their
 * time when the report capped the list. The list and every number are the
 * native side's answer for this day of this report, so they add up to the
 * day's agent time.
 *
 * Read when the box first opens (it is only mounted while it is open) and
 * kept for the same report: opening the same day again reads nothing,
 * a new report is a new read, and committed data re-reads it. While a new
 * report's answer is on its way the box keeps the last one, unless stored
 * content was deleted since, which drops every kept answer. A session is
 * named as the lanes name it: its host title when that has been read, else its
 * saved title or reviewer mark, else its short identifier.
 */
export function DaySessions({
  window,
  date,
  label,
  range,
  listRef,
}: {
  window: DashboardWindow;
  date: string;
  /** The day as its row writes it, for the heading. */
  label: string;
  /** The Dashboard's range, carried by each link as the lanes' links carry it. */
  range: TimeRange;
  /** Given the list of links once the sessions have been read. */
  listRef?: (list: HTMLUListElement | null) => void;
}) {
  const { source } = useData();
  const available = Boolean(source.daySessions);
  // Keyed by the purge epoch, so no saved title read before "Delete stored
  // content" is shown after it, not even while the next answer is on its way.
  const purges = usePurgeEpoch();
  const query = useQuery({
    queryKey: queryKeys.daySessions(purges, window.days, window.end_ms, window.timezone, date),
    queryFn: () => source.daySessions!(window.days, window.end_ms, window.timezone, date),
    enabled: available,
    staleTime: Infinity,
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === purges ? previous : undefined,
  });
  const sessions = query.data?.sessions ?? [];
  const hostTitle = useSessionTitles(
    JSON.stringify(['effort-day', date]),
    // The session's latest activity, on any day, as Sessions shows it: every
    // new event reads its title again, even one after this day or one that
    // adds no time. A session with none known is read once.
    sessions.map((session) => ({ id: session.session_id, version: titleVersion(session) })),
    { keepResolved: true },
  );
  if (!available) return null;
  const body = query.isError ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      Sessions could not be read
    </span>
  ) : !query.data ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      Reading sessions…
    </span>
  ) : sessions.length === 0 ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      No sessions
    </span>
  ) : (
    <ul ref={listRef} className="xt-effort-tip-sessions" role="list" aria-label="Sessions this day">
      {sessions.map((session) => {
        const title = displayTitle(
          hostTitle(session.session_id, titleVersion(session)) ?? session.title,
          session.automated_review,
        );
        const name = title ?? `Session ${shortId(session.session_id)}`;
        return (
          <li key={`${session.host}\u0000${session.session_id}`}>
            <Link
              className="xt-effort-tip-session"
              data-testid="day-session"
              to={sessionHref(
                session.session_id,
                listAddress({ sessionId: session.session_id, host: session.host }, range),
              )}
              // The visible name can be cut to fit, so the link names the
              // session in full, as a lane's link does.
              aria-label={`Open session ${title ? `${title}, ` : ''}${session.session_id}, ${agentDuration(session.agent_ms).spoken} this day`}
              title={`${name} · ${session.session_id}`}
            >
              <span>{name}</span>
              <b>{agentTime(session.agent_ms)}</b>
            </Link>
          </li>
        );
      })}
      {query.data.more_sessions > 0 && (
        <li className="xt-effort-tip-row" data-testid="day-sessions-more">
          <span>and {plural(query.data.more_sessions, 'more session')}</span>
          <b>{agentTime(query.data.more_ms)}</b>
        </li>
      )}
    </ul>
  );
  return (
    <>
      <span className="xt-effort-tip-head" data-testid="day-sessions-head">
        <span>Sessions on {label}</span>
      </span>
      {body}
    </>
  );
}
