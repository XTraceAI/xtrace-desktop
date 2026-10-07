import { useState, type CSSProperties, type ReactNode } from 'react';
import { Link } from 'react-router';
import type { DashboardLaneCost } from '../../data/generated/DashboardLaneCost';
import type { DashboardLaneSession } from '../../data/generated/DashboardLaneSession';
import type { DashboardUnpriced } from '../../data/generated/DashboardUnpriced';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { SessionParentLink } from '../../data/generated/SessionParentLink';
import { DataTable, type Column } from '../../kit/DataTable';
import { EvidenceDot } from '../../kit/Badge';
import { HostGlyph } from '../../kit/HostGlyph';
import { hostName } from '../../kit/hosts';
import { Icon } from '../../kit/icons';
import { MetricCell } from '../../kit/MetricCell';
import { RulePopover } from '../../kit/RulePopover';
import type { TimeRange } from '../../kit/TopBar';
import { NO_MODEL, contextLead, contextTitle, displayTitle, shortId } from '../session-context';
import { GroupToggle, ParentMarker, parentName, verifiedParent } from '../session-parent';
import { listAddress, sessionHref } from '../session-search';
import { useSessionTitles, useVisibleIds } from '../session-titles';
import {
  COMPACTION_MEANING,
  CompactionBadge,
  CompactionTick,
  compactionEvents,
  useSessionCompactions,
} from '../session-compactions';
import {
  isLiveSessionHost,
  LIVE_SESSION_HOSTS,
  useLiveSessionStatus,
} from '../live-session-status';
import '../../styles/live-session-status.css';
import { LaneSpan } from './LaneSpan';
import { axisTime, clockTime, plural, recordedTime, unpricedText, usd } from './present';
import {
  groupSessionLanes,
  laneRows,
  listedLanes,
  notListed,
  sessionKey,
  type LaneParent,
  type LaneRow,
  type SessionLane,
} from './lanes';

type Row = LaneRow<SessionParentLink>;

const HOUR = 3_600_000;
/** Said of a row no indexed session owns, so nothing was priced. */
const NO_SESSION = 'Cost unknown for this session';
/** Said of an indexed session with no selected response at all. */
const NO_RESPONSES = 'No responses recorded for this session, so there is nothing to price';
/** Said of a collapsed group none of whose sessions recorded a response. */
const NO_GROUP_RESPONSES = 'No responses recorded for these sessions, so there is nothing to price';
/** Said of a row the report carries no context row for. */
const NO_CONTEXT = "This session hasn't been read yet";
/** Said of an indexed session with no known start. */
const NO_START = 'Start unknown for this session';
/** What the PRs column counts, wherever it is explained. */
export const PR_MEANING =
  'Linked PRs, merged or not. 0 means no link was found; a grey dot means the count includes guesses.';

/**
 * Rows the lane table always shows. The table takes the Sessions card's share
 * of a taller window and shows more; beyond what it shows, it scrolls.
 */
export const MIN_LANE_ROWS = 3;

/** The whole hours of the report's fixed recent axis. */
export const laneHours = (report: DashboardMetrics) =>
  Math.round(Math.max(1, report.lane_end_ms - report.lane_start_ms) / HOUR);

/** Which sessions the card lists, said where the card is named. */
export const laneSubtitle = (report: DashboardMetrics) =>
  `Active in the last ${laneHours(report)} hours`;

/** What the rows are and are not, read from the card's definition. */
export const laneDefinition = (report: DashboardMetrics) =>
  `Each row's numbers cover the whole session; only the Activity bars stop at ${laneHours(report)} hours.`;

/**
 * Ticks at the quarters of the fixed recent axis, on one line after the
 * column's name. The end carries its date; the start, which follows the name,
 * is its time only, so it clears the next tick in a narrow column.
 */
function Axis({ start, span, window }: { start: number; span: number; window: DashboardWindow }) {
  // The column's visible name, as the reference labels it, sits over the tick
  // row inside the same 24px header, so naming it costs no row height.
  return (
    <span className="xt-lanes-activity">
      <span className="xt-lanes-activity-label">Activity</span>
      <span className="xt-lanes-axis">
        <span className="sr-only">: active spans on the fixed recent axis</span>
        {Array.from({ length: 5 }, (_, index) => (
          <span key={index} style={{ left: `${(index / 4) * 100}%` }} aria-hidden="true">
            {axisTime(start + (span * index) / 4, window, index === 4)}
          </span>
        ))}
      </span>
    </span>
  );
}

/**
 * The sessions active in the report's fixed recent window, independent of the
 * selected range, with their returned active spans on that window's axis.
 *
 * One row per session, carrying every span the report returned for it. The
 * window decides which sessions are listed and where their bars fall; every
 * number in a row describes the whole session.
 *
 * A session with a verified parent is listed under that parent, collapsed
 * behind a disclosure on the parent's row — or, when the report returned no
 * span for the parent, on a row that only names it and counts its returned
 * sub-sessions, with no measurement of its own. Opening a group lists each
 * child's own row as it would read ungrouped. A session the report marks as a
 * known sub-session whose creator is not verified is not listed at all, nor
 * is any returned session under it, and nothing is read for them. The
 * Sessions page arranges its lists the same way.
 *
 * A row names its session by its host's own title — read from the original
 * source for the rows in view, never stored — or a title the index saved, else
 * by enough of its identity to tell two rows apart, with the indexed
 * repository and branch beside it, and links that identity — in full — to the session's own page,
 * carrying list state that would find it so the page's Back link returns to a
 * list holding it. Started is the host's recorded start, or a Claude session's
 * earliest imported message (Claude Code records no start), whenever it was; PRs is
 * the count of recorded pull-request links at every evidence level. An unknown
 * fact is said to be unknown; a measured zero stays zero.
 *
 * The cost column is the one measurement here, and it is the whole session's,
 * whenever it was spent, so neither the window nor the display cap reduces
 * it. A collapsed group shows the sum of the rows opening it would list — its
 * own session, if it has a row, and every listed session under it — so the
 * cost cells in view add up to the same amount whether a group is open or
 * not. The column's own definition says so.
 */
export function ActivityLanes({ report, range }: { report: DashboardMetrics; range: TimeRange }) {
  const { lanes, lane_start_ms: start, lane_end_ms: end, window } = report;
  const span = Math.max(1, end - start);
  const hours = laneHours(report);
  // Context is addressed by identity, not by position: the report orders it by
  // identifier while the rows keep the order the spans were returned in. It
  // also holds context-only entries for parents with no span here, which
  // never make a row by themselves, and the sub-sessions with no span here.
  // Keyed by exact host and identity, as the grouping is: one session's
  // context is never another host's.
  const context = new Map<string, DashboardLaneSession>(
    [...report.lane_sessions, ...report.lane_sub_sessions].map((session) => [
      sessionKey(session.host, session.session_id),
      session,
    ]),
  );
  // A session the report named no context row for keeps an honest unknown
  // rather than borrowing another row's repository.
  const found = (lane: SessionLane) => context.get(sessionKey(lane.host, lane.sessionId)) ?? null;
  const linkOf = (lane: SessionLane) =>
    verifiedParent({ id: lane.sessionId, parent: found(lane)?.parent });
  // A session still being checked, or a known sub-session whose creator is
  // not verified, is not listed here at all, nor anything returned under it;
  // nor does the Sessions page. A report with no context for a session, or
  // without its display check, leaves it out: it is not known to be shown.
  const unresolved = ({ session_id, host }: LaneParent) => {
    const session = context.get(sessionKey(host, session_id));
    return notListed(
      session,
      host,
      verifiedParent({ id: session_id, parent: session?.parent }) !== null,
    );
  };
  const returned = groupSessionLanes(lanes);
  // The listed sessions' sub-sessions with no span here, whenever they ran,
  // so a group lists every sub-session its total adds up. Each is placed only
  // under its verified parent — a returned session or another of these — so
  // none becomes a row of its own, and it has no span to draw. They follow the
  // returned sessions, so a group still sits where its newest returned member
  // does. Placement does not depend on the report's order: passes repeat
  // until no more can be placed, and only one whose parent never appears is
  // left out.
  const placed = new Set(returned.map((lane) => lane.key));
  const older: SessionLane[] = [];
  let waiting = report.lane_sub_sessions;
  for (let progress = true; progress && waiting.length > 0;) {
    progress = false;
    const next: typeof waiting = [];
    for (const session of waiting) {
      const key = sessionKey(session.host, session.session_id);
      const parent = verifiedParent({ id: session.session_id, parent: session.parent });
      if (!parent || placed.has(key)) continue;
      if (!placed.has(sessionKey(parent.host, parent.session_id))) {
        next.push(session);
        continue;
      }
      placed.add(key);
      progress = true;
      older.push({
        key,
        sessionId: session.session_id,
        host: session.host,
        spans: [],
        firstMs: 0,
        lastMs: 0,
      });
    }
    waiting = next;
  }
  const sessions = listedLanes([...returned, ...older], linkOf, unresolved);
  // Which groups are open: collapsed until asked, and kept by key while the
  // report refreshes, so a group open before a live update stays open after.
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const toggle = (key: string) =>
    setOpen((previous) => {
      const next = new Set(previous);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  const position = (ms: number) => (Math.min(end, Math.max(start, ms)) - start) / span;
  // Host titles are read only for the rows inside the table's own scroll area,
  // and again for a row whose latest span has moved, which keeps its last
  // title until that read answers; scrolling brings more rows into view, and
  // a new range starts again.
  const { observe, visible } = useVisibleIds();
  const liveStatus = useLiveSessionStatus(
    JSON.stringify(['dashboard', range]),
    sessions
      .filter((lane) => isLiveSessionHost(lane.host) && visible.has(lane.sessionId))
      .map((lane) => lane.sessionId),
    { keepResolved: true },
  );
  const hostTitle = useSessionTitles(
    JSON.stringify(['dashboard', range]),
    sessions
      .filter((lane) => visible.has(lane.sessionId))
      .map((lane) => ({ id: lane.sessionId, version: lane.lastMs })),
    { keepResolved: true },
  );
  const titleOf = (lane: SessionLane) =>
    displayTitle(
      hostTitle(lane.sessionId, lane.lastMs) ?? found(lane)?.title ?? null,
      found(lane)?.automated_review ?? false,
    );
  // A verified parent is named by its own row when it has one here, as that
  // row reads now; a parent with no lane in this report keeps the name the
  // report sent, and no title is read for it. It never gains a lane.
  const rowsById = new Map(sessions.map((lane) => [lane.sessionId, lane]));
  const compaction = useSessionCompactions(
    JSON.stringify(['dashboard', range]),
    sessions
      .filter((lane) => visible.has(lane.sessionId) && !linkOf(lane))
      .map((lane) => lane.sessionId),
    { retryTransient: true },
  );
  const parentOf = (lane: SessionLane) => {
    const parent = linkOf(lane);
    if (!parent) return null;
    const shown = rowsById.get(parent.session_id);
    return {
      parent,
      name: parentName(parent, shown?.host === parent.host ? titleOf(shown) : undefined),
    };
  };

  const spanText = (lane: SessionLane) =>
    lane.spans
      .map(
        (item) =>
          `${clockTime(item.start_ms, window)} – ${clockTime(item.end_ms, window)}${
            item.start_ms === item.end_ms ? ' (single event)' : ''
          }`,
      )
      .join(', ');
  const costText = (lane: SessionLane) => shownCostText(shownCost([found(lane)]));
  const activityText = (lane: SessionLane) =>
    lane.spans.length === 0
      ? `no activity in the last ${hours} hours`
      : `${plural(lane.spans.length, 'active span')} in the last ${hours} hours — ${spanText(lane)}`;
  const startedText = (lane: SessionLane) => {
    const session = found(lane);
    if (!session) return "start unknown: this session hasn't been read yet";
    return session.started_at_ms === null
      ? 'start unknown'
      : `started ${recordedTime(session.started_at_ms, window)}`;
  };
  const prText = (lane: SessionLane) => {
    const session = found(lane);
    if (!session || session.pr_links === null) return 'recorded pull request links unknown';
    const inferred = session.inferred_pr_links ?? 0;
    return session.pr_links === 0
      ? 'no recorded pull request link'
      : `${plural(session.pr_links, 'recorded pull request link')}${
          inferred > 0 ? `, ${inferred.toLocaleString('en-US')} inferred` : ''
        }`;
  };
  const rowText = (lane: SessionLane) => {
    const session = found(lane);
    const title = titleOf(lane);
    const created = parentOf(lane);
    return `${hostName(lane.host)} session ${lane.sessionId}, repository ${
      session?.repo ?? 'unknown'
    }, branch ${session?.branch ?? 'unknown'}${title ? `, titled ${title}` : ''}${
      created ? `, sub-session of ${created.name} (${created.parent.session_id})` : ''
    }: ${activityText(lane)}. ${startedText(lane)}; ${prText(lane)}. Whole session: ${costText(
      lane,
    )}.`;
  };

  // Every listed session is placed once: under its parent's row, else under
  // a row that names an absent parent. The table draws the rows
  // this union holds, so a collapsed child is unmounted, not hidden.
  const { rows } = laneRows(sessions, linkOf, open);
  const nameOf = (lane: SessionLane) => titleOf(lane) ?? `Session ${shortId(lane.sessionId)}`;
  /**
   * The cost a row shows: its own, or a collapsed group's total of every row
   * opening it lists, with any sub-sessions the report left out counted.
   */
  const rowCost = (row: Row): ShownCost | null => {
    if (row.children > 0 && !row.expanded) {
      const group = [...(row.kind === 'session' ? [row.lane] : []), ...row.members].map(found);
      const notShown = group.reduce(
        (sum, session) => sum + (session?.sub_sessions_not_shown ?? 0),
        0,
      );
      const cutShort = group.some((session) => session?.sub_sessions_cut_short === true);
      return shownCost(group, { total: true, notShown, cutShort });
    }
    return row.kind === 'session' ? shownCost([found(row.lane)]) : null;
  };
  /**
   * Sub-sessions the report left out under a row that shows no total of
   * them — one with none listed, or an open group — said beside its own cost.
   */
  const leftOutNote = (row: Row) => {
    if (row.kind !== 'session' || (row.children > 0 && !row.expanded)) return '';
    const session = found(row.lane);
    return hiddenNote(
      session?.sub_sessions_not_shown ?? 0,
      session?.sub_sessions_cut_short === true,
    );
  };
  const groupText = (row: Row) => {
    if (row.children === 0) return '';
    if (row.expanded) return ` ${plural(row.children, 'sub-session')} listed below.`;
    const cost = rowCost(row);
    return ` ${plural(row.children, 'sub-session')} collapsed under this row. Total${
      row.kind === 'session' ? ' with its sub-sessions' : ''
    }: ${cost ? shownCostText(cost) : 'cost unknown'}.`;
  };
  const absentText = (row: Extract<Row, { kind: 'absent' }>) =>
    `Sub-sessions of ${hostName(row.parent.host)} session ${row.parent.session_id}, which is not listed here: it is only named, with no numbers of its own. Its sub-sessions active in the last ${hours} hours are listed, with their own sub-sessions.${groupText(
      row,
    )}`;
  /** A cell that belongs to a returned session; an absent parent's is blank. */
  const onLane =
    (render: (lane: SessionLane) => ReactNode) =>
    (row: Row): ReactNode =>
      row.kind === 'session' ? render(row.lane) : null;

  const columns: Column<Row>[] = [
    {
      key: 'host',
      header: <span className="sr-only">Host</span>,
      width: '20px',
      render: (row) => {
        const host = row.kind === 'session' ? row.lane.host : row.parent.host;
        const running =
          row.kind === 'session' &&
          isLiveSessionHost(host) &&
          liveStatus(row.lane.sessionId) === 'running';
        const glyph = <HostGlyph host={host} size={18} />;
        return running && isLiveSessionHost(host) ? (
          <span
            className="xt-lane-live-host"
            data-live-status="running"
            role="img"
            aria-label={`${LIVE_SESSION_HOSTS[host].label} · Running`}
            title={`${LIVE_SESSION_HOSTS[host].source} · Running`}
          >
            <span aria-hidden="true">{glyph}</span>
          </span>
        ) : (
          glyph
        );
      },
    },
    {
      key: 'session',
      header: 'session',
      width: 'minmax(0, 1fr)',
      render: (row) => {
        if (row.kind === 'absent') {
          const name = parentName(row.parent);
          return (
            <span className="xt-lane-name" data-group="absent">
              <GroupToggle row={row} name={name} noun="sub-session" onToggle={toggle} />
              <span className="xt-lane-group-words" aria-hidden="true">
                Sub-sessions of{' '}
              </span>
              <Link
                className="xt-lane-find"
                to={sessionHref(
                  row.parent.session_id,
                  listAddress({ sessionId: row.parent.session_id, host: row.parent.host }, range),
                )}
                aria-label={`Open parent session ${name}, ${row.parent.session_id}`}
                title={`Sub-sessions of ${name} · ${row.parent.session_id}`}
              >
                {name}
              </Link>
              <span className="sr-only"> — {absentText(row)}</span>
            </span>
          );
        }
        const { lane } = row;
        const session = found(lane);
        const title = titleOf(lane);
        const created = parentOf(lane);
        return (
          <span
            className="xt-lane-name"
            ref={observe}
            data-visible-id={lane.sessionId}
            data-subsession={created ? '' : undefined}
            style={row.depth > 0 ? ({ '--depth': row.depth } as CSSProperties) : undefined}
          >
            {row.children > 0 && (
              <GroupToggle row={row} name={nameOf(lane)} noun="sub-session" onToggle={toggle} />
            )}
            <Link
              className="xt-lane-find"
              to={sessionHref(lane.sessionId, listAddress(lane, range))}
              // The visible name can be shortened to fit, so the link says
              // which session it opens, in full. The detail route names it
              // exactly.
              aria-label={`Open session ${title ? `${title}, ` : ''}${lane.sessionId}`}
              title={title ? `${title} · ${lane.sessionId}` : lane.sessionId}
            >
              {title ?? `Session ${shortId(lane.sessionId)}`}
            </Link>
            {created && (
              // Between the name and the context, so the context gives way
              // first. The link carries the list state that finds the parent,
              // as the name's own link does for this session.
              <ParentMarker
                className="xt-lane-parent"
                parent={created.parent}
                name={created.name}
                to={sessionHref(
                  created.parent.session_id,
                  listAddress(
                    { sessionId: created.parent.session_id, host: created.parent.host },
                    range,
                  ),
                )}
              />
            )}
            {/* Spoken once, by the row's own description below, so the dense
                visible line is not read twice. The context gives way to the
                ellipsis first; the name keeps its room up to most of the cell. */}
            <span className="xt-lane-dot" aria-hidden="true">
              ·
            </span>
            <span
              className="xt-lane-repo"
              aria-hidden="true"
              title={contextTitle(session?.repo ?? null, session?.branch ?? null)}
            >
              {contextLead(session?.repo ?? null, session?.branch ?? null)}
            </span>
            <span className="sr-only">{` — ${rowText(lane)}${groupText(row)}${
              leftOutNote(row) ? ` ${leftOutNote(row)}` : ''
            }`}</span>
          </span>
        );
      },
    },
    {
      key: 'compactions',
      header: <span title={COMPACTION_MEANING}>Compactions</span>,
      width: '92px',
      render: onLane((lane) =>
        linkOf(lane) ? null : (
          <CompactionBadge
            outcome={compaction(lane.sessionId)}
            reading={compaction.reading(lane.sessionId)}
          />
        ),
      ),
    },
    {
      key: 'prs',
      header: (
        <RulePopover ruleId="M-13" context={PR_MEANING}>
          <button
            type="button"
            className="xt-table-metric-header"
            aria-label="Recorded pull request links, definition"
          >
            PRs
          </button>
        </RulePopover>
      ),
      width: '60px',
      align: 'right',
      render: onLane((lane) => {
        const session = found(lane);
        if (!session || session.pr_links === null)
          return <MetricCell value={null} align="right" size={10.5} reason={NO_CONTEXT} />;
        const inferred = session.inferred_pr_links ?? 0;
        return (
          <span
            className="xt-lane-prs"
            data-inferred={inferred > 0 || undefined}
            data-empty={session.pr_links === 0 || undefined}
            title={`${prText(lane)} · whole session, merged or not`}
          >
            {inferred > 0 && (
              <span aria-hidden="true">
                <EvidenceDot evidence="inferred" />
              </span>
            )}
            <Icon name="prs" size={13} className="xt-lane-prs-icon" />
            <span aria-hidden="true">{session.pr_links.toLocaleString('en-US')}</span>
            <span className="sr-only">{prText(lane)}</span>
          </span>
        );
      }),
    },
    {
      key: 'started',
      header: <span title="When the session began, even if before this range.">started</span>,
      width: '98px',
      render: onLane((lane) => {
        const session = found(lane);
        if (!session || session.started_at_ms === null)
          return <MetricCell value={null} size={10.5} reason={session ? NO_START : NO_CONTEXT} />;
        return (
          <time
            className="xt-lane-from"
            dateTime={new Date(session.started_at_ms).toISOString()}
            title={recordedTime(session.started_at_ms, window)}
          >
            {clockTime(session.started_at_ms, window)}
          </time>
        );
      }),
    },
    {
      key: 'spans',
      header: <Axis start={start} span={span} window={window} />,
      width: 'minmax(120px, 1.7fr)',
      render: onLane((lane) => (
        // The gridlines are spoken by the row's description. Each span and
        // each compaction tick drawn over the track is a focusable, named
        // mark whose bubble opens on hover or focus.
        <span className="xt-lane-track">
          {[1, 2, 3].map((line) => (
            <i key={line} style={{ left: `${line * 25}%` }} aria-hidden="true" />
          ))}
          {lane.spans.map((item) => {
            const left = position(item.start_ms);
            return (
              <LaneSpan
                // A row's spans start at distinct instants, and a live span
                // keeps its start while its end grows, so an open bubble
                // survives each refresh of the report.
                key={item.start_ms}
                span={item}
                name={nameOf(lane)}
                left={left}
                width={position(item.end_ms) - left}
                window={window}
              />
            );
          })}
          {/* A sub-session is not read for compactions; its parent is. Ticks
              outside the window are not drawn; inside it they are drawn
              wherever they fall, idle gaps included. */}
          {!linkOf(lane) &&
            compactionEvents(compaction(lane.sessionId), start, end).map((event, index) => (
              <CompactionTick
                key={`${index}-${event.at_ms}`}
                event={event}
                left={position(event.at_ms)}
                when={recordedTime(event.at_ms, window)}
              />
            ))}
        </span>
      )),
    },
    {
      key: 'cost',
      // The measured column carries its own definition, and says in the same
      // place that it covers the whole session, so the number is never read as
      // a total of the spans drawn beside it.
      header: (
        <RulePopover ruleId="M-04" context={laneCostDefinition(hours)}>
          <button
            type="button"
            className="xt-table-metric-header"
            aria-label="Whole-session cost, definition"
          >
            cost
          </button>
        </RulePopover>
      ),
      // Wide enough for "Σ $12,345+" and "Σ $999.99+" without an ellipsis.
      width: '80px',
      align: 'right',
      // Every row, an absent parent's included: collapsed, it shows its
      // sub-sessions' total; open, it is blank, as it has no cost.
      render: (row) => {
        const cost = rowCost(row);
        return cost && <LaneCost shown={cost} note={leftOutNote(row)} />;
      },
    },
  ];

  return (
    <div
      className="xt-lanes"
      data-empty={sessions.length === 0 || undefined}
      style={
        {
          // The table's minimum is three rows however few it lists; its most
          // is from the rows drawn, so opening a group grows the table up to
          // the card's share and scrolls inside it beyond that.
          '--rows': MIN_LANE_ROWS,
          '--count': rows.length,
        } as CSSProperties
      }
    >
      <DataTable
        label="Session lanes"
        columns={columns}
        rows={rows}
        getRowKey={(row) => row.key}
        rowHeight={22}
        hover="track"
        stickyHeader
        emptyMessage={`${
          returned.length > 0 ? 'No session to list' : 'No active span'
        } in the last ${hours} hours (${clockTime(start, window)} – ${clockTime(end, window)}).`}
      />
    </div>
  );
}

/** The cost column's definition, in plain words. */
export const laneCostDefinition = (hours: number) =>
  `Cost of the whole session, not just the last ${hours} hours. Σ is the session plus its sub-sessions; + means part is unpriced.`;

/** Unpriced responses of one model for one reason, every tier merged. */
interface UnpricedGap {
  model: string | null;
  reason: DashboardUnpriced['reason'];
  /** Distinct recorded tiers, in report order; named only when the tier is the reason. */
  tiers: string[];
  observations: number;
}

/**
 * The report groups unpriced responses by model, service tier and reason;
 * the words below name only the model and reason, so entries that differ
 * only by tier are merged here (counts summed, report order kept) and never
 * read as the same phrase twice.
 */
export const mergeUnpriced = (unpriced: readonly DashboardUnpriced[]): UnpricedGap[] => {
  const merged = new Map<string, UnpricedGap>();
  for (const item of unpriced) {
    const model = item.reason === 'missing_model' ? null : item.model;
    const key = JSON.stringify([model, item.reason]);
    const gap = merged.get(key) ?? { model, reason: item.reason, tiers: [], observations: 0 };
    gap.observations += item.observations;
    if (item.service_tier !== null && !gap.tiers.includes(item.service_tier))
      gap.tiers.push(item.service_tier);
    merged.set(key, gap);
  }
  return [...merged.values()];
};

/**
 * What could not be priced, by model and reason, in the cost report's words:
 * "codex-auto-review has no published price". With counts, "20
 * codex-auto-review have no published price".
 */
const unpricedGap = (gap: UnpricedGap, counted: boolean) => {
  const many = gap.observations !== 1;
  const count = counted ? `${gap.observations.toLocaleString('en-US')} ` : '';
  if (gap.reason === 'missing_model')
    return counted
      ? `${count}${many ? 'responses record' : 'response records'} no model`
      : 'no model recorded';
  const model = gap.model ?? NO_MODEL;
  if (gap.reason === 'unknown_model')
    return `${count}${model} ${counted && many ? 'have' : 'has'} no published price`;
  const reason =
    gap.reason === 'unknown_service_tier' && gap.tiers.length > 0
      ? `service ${gap.tiers.length === 1 ? 'tier' : 'tiers'} ${gap.tiers.join(', ')} not in the price catalog`
      : unpricedText[gap.reason];
  return `${count}${model}${counted ? (many ? ' responses' : ' response') : ''}: ${reason}`;
};
export const unpricedNames = (cost: DashboardLaneCost) =>
  cost.unpriced.length === 0
    ? `none of ${plural(cost.selected_observations, 'response')} could be priced`
    : mergeUnpriced(cost.unpriced)
        .map((gap) => unpricedGap(gap, false))
        .join('; ');
/** "980 of 1,000 responses priced; 20 codex-auto-review have no published price" */
export const partialText = (cost: DashboardLaneCost) =>
  `${cost.priced_observations.toLocaleString('en-US')} of ${plural(cost.selected_observations, 'response')} priced; ${
    cost.unpriced.length === 0
      ? `${cost.unpriced_observations.toLocaleString('en-US')} could not be priced`
      : mergeUnpriced(cost.unpriced)
          .map((gap) => unpricedGap(gap, true))
          .join('; ')
  }`;

/**
 * The cost one row shows: a session's own, or a collapsed group's total of
 * every row opening it lists. `unknown` counts sessions in it whose cost is
 * unknown, and `notShown` sub-sessions the report left out; neither is in the
 * amount, which is then a floor.
 */
export interface ShownCost {
  cost: DashboardLaneCost | null;
  /** A collapsed group's total, marked Σ. */
  total: boolean;
  /** How many sessions the amount covers. */
  sessions: number;
  unknown: number;
  notShown: number;
  /** The report's walk for some of these stopped early: `notShown` is a floor. */
  cutShort: boolean;
}

/**
 * Sessions' whole costs added up, in the report's own shape: counts and
 * priced subtotals add, unpriced responses keep their model and reason, and
 * the total is known only when every session's is and none is left out. A
 * session with no response adds nothing and leaves the total known.
 */
export const shownCost = (
  sessions: readonly (DashboardLaneSession | null | undefined)[],
  {
    total = false,
    notShown = 0,
    cutShort = false,
  }: { total?: boolean; notShown?: number; cutShort?: boolean } = {},
): ShownCost => {
  const costs = sessions.flatMap((session) => (session?.cost ? [session.cost] : []));
  const unknown = sessions.length - costs.length;
  const add = (pick: (cost: DashboardLaneCost) => number) =>
    costs.reduce((sum, cost) => sum + pick(cost), 0);
  const subtotal = add((cost) => cost.priced_subtotal_usd);
  const selected = add((cost) => cost.selected_observations);
  return {
    cost:
      costs.length === 0
        ? null
        : {
            total_usd:
              unknown === 0 &&
              notShown === 0 &&
              !cutShort &&
              selected > 0 &&
              costs.every((cost) => cost.selected_observations === 0 || cost.total_usd !== null)
                ? subtotal
                : null,
            priced_subtotal_usd: subtotal,
            selected_observations: selected,
            priced_observations: add((cost) => cost.priced_observations),
            unpriced_observations: add((cost) => cost.unpriced_observations),
            assumed_tier_observations: add((cost) => cost.assumed_tier_observations),
            unpriced: costs.flatMap((cost) => cost.unpriced),
          },
    total,
    sessions: sessions.length,
    unknown,
    notShown,
    cutShort,
  };
};

/** Sub-sessions left out, e.g. "3 more sub-sessions not shown"; "at least" when the count is a floor. */
const notShownText = (count: number, cutShort: boolean) =>
  cutShort && count === 0
    ? 'more sub-sessions may not be shown'
    : `${cutShort ? 'at least ' : ''}${plural(count, 'more sub-session')} not shown`;

/** The same, said of one session's own row, which shows no total of them. */
export const hiddenNote = (count: number, cutShort: boolean) =>
  count === 0 && !cutShort
    ? ''
    : cutShort && count === 0
      ? 'More sub-sessions under this session may not be shown or counted.'
      : `${cutShort ? 'At least ' : ''}${plural(count, 'more sub-session')} under this session ${
          count === 1 && !cutShort ? 'is' : 'are'
        } not shown or counted.`;

/** What a total leaves out, e.g. "2 sessions' cost unknown, not included". */
const leftOut = ({ unknown, notShown, cutShort }: ShownCost) => {
  const parts = [
    unknown > 0
      ? `${unknown === 1 ? "1 session's" : `${unknown.toLocaleString('en-US')} sessions'`} cost unknown`
      : '',
    notShown > 0 || cutShort ? notShownText(notShown, cutShort) : '',
  ].filter(Boolean);
  return parts.length === 0 ? '' : `${parts.join(' and ')}, not included`;
};
const andLeftOut = (shown: ShownCost) => (leftOut(shown) ? `; ${leftOut(shown)}` : '');

/** Why an amount is only a floor: what could not be priced, then what is left out. */
const floorText = (cost: DashboardLaneCost, shown: ShownCost) =>
  `${
    cost.unpriced_observations > 0
      ? partialText(cost)
      : `all ${plural(cost.selected_observations, 'response')} priced`
  }${andLeftOut(shown)}`;

/** A shown cost in words, for a row's spoken description. */
export const shownCostText = (shown: ShownCost) => {
  const { cost } = shown;
  if (!cost) return 'cost unknown';
  if (cost.selected_observations === 0)
    return leftOut(shown) ? `no responses to price; ${leftOut(shown)}` : 'no responses to price';
  if (cost.priced_observations === 0)
    return `cost unknown: ${unpricedNames(cost)}${andLeftOut(shown)}`;
  if (cost.total_usd !== null)
    return `${usd(cost.total_usd)} API-equivalent cost of ${plural(cost.selected_observations, 'response')}`;
  return `at least ${usd(cost.priced_subtotal_usd)} API-equivalent cost: ${floorText(cost, shown)}`;
};

/** One row's whole-session cost: a session's own, or a collapsed group's Σ total. */
function LaneCost({ shown, note = '' }: { shown: ShownCost; note?: string }) {
  // Sub-sessions left out under a row that shows no total of them.
  const aside = note ? ` ${note}` : '';
  const unknown = (reason: string) => (
    <MetricCell value={null} align="right" size={10.5} reason={`${reason}${aside}`} />
  );
  const { cost, total } = shown;
  if (!cost) return unknown(total ? 'Cost unknown for these sessions' : NO_SESSION);
  if (cost.selected_observations === 0)
    return unknown(
      leftOut(shown)
        ? `No responses to price; ${leftOut(shown)}`
        : total
          ? NO_GROUP_RESPONSES
          : NO_RESPONSES,
    );
  // A model name keeps its own spelling, so the reason is not capitalised.
  if (cost.priced_observations === 0) return unknown(`${unpricedNames(cost)}${andLeftOut(shown)}`);
  const mark = total ? 'Σ ' : '';
  const scope = total ? `Total for these ${plural(shown.sessions, 'session')}: ` : '';
  // Kept short: the row's spoken description carries the full breakdown.
  const assumed =
    cost.assumed_tier_observations > 0
      ? ` ${plural(cost.assumed_tier_observations, 'Codex response')} priced at the standard tier.`
      : '';
  if (cost.total_usd !== null)
    return (
      <MetricCell
        value={`${mark}${usd(cost.total_usd)}`}
        align="right"
        size={10.5}
        title={`${scope}${usd(cost.total_usd)} at public API prices.${assumed}${aside}`}
      />
    );
  return (
    <MetricCell
      value={`${mark}${usd(cost.priced_subtotal_usd)}+`}
      align="right"
      size={10.5}
      title={`${scope ? `${scope}at least` : 'At least'} ${usd(cost.priced_subtotal_usd)}: ${
        cost.unpriced_observations > 0
          ? `${cost.unpriced_observations.toLocaleString('en-US')} of ${plural(cost.selected_observations, 'response')} ${cost.unpriced_observations === 1 ? 'has' : 'have'} no price${andLeftOut(shown)}`
          : leftOut(shown) || 'some responses have no price'
      }.${assumed}${aside}`}
    />
  );
}
