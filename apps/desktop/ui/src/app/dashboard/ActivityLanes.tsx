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
import { Icon } from '../../kit/icons';
import { MetricCell } from '../../kit/MetricCell';
import { RulePopover } from '../../kit/RulePopover';
import type { TimeRange } from '../../kit/TopBar';
import { contextLead, contextTitle, displayTitle, shortId } from '../session-context';
import { ParentMarker, parentName, verifiedParent } from '../session-parent';
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
import { clockTime, plural, recordedTime, unpricedText, usd } from './present';
import { groupSessionLanes, laneRows, type LaneRow, type SessionLane } from './lanes';

type Row = LaneRow<SessionParentLink>;

const HOUR = 3_600_000;
/** Said of a row no indexed session owns, so nothing was priced. */
const NO_SESSION = 'No indexed session owns this identifier, so nothing was priced';
/** Said of an indexed session with no selected response in the window. */
const NO_RESPONSES = 'No selected responses in this window, so there is nothing to price';
/** Said of a row the report carries no context row for. */
const NO_CONTEXT = 'No indexed context was read for this session';
/** Said of an indexed session with no known start. */
const NO_START = 'No start is known for this session; its first active span is not its start';
/** What the PRs column counts, wherever it is explained. */
export const PR_MEANING =
  'Recorded session → pull request links: every evidence level (exact, commit, inferred), all indexed time, not merge status. Zero means no link is recorded, not that no pull request exists; a grey dot marks a count that includes inferred links.';

/**
 * Rows the lane table always shows. The table takes the Sessions card's share
 * of a taller window and shows more; beyond what it shows, it scrolls.
 */
export const MIN_LANE_ROWS = 3;

/** The whole hours of the report's fixed recent axis. */
export const laneHours = (report: DashboardMetrics) =>
  Math.round(Math.max(1, report.lane_end_ms - report.lane_start_ms) / HOUR);

/** What the rows are and are not, read from the card's definition. */
export const laneDefinition = (report: DashboardMetrics) => {
  const hours = laneHours(report);
  return `Session rows are active spans on the fixed recent ${hours}-hour axis, whatever range is selected: a session row is that window only, never a whole session. Metrics include every span, drawn or not, and cost covers each listed session across the whole ${hours} hours, including spans the table does not draw. Started is when the session began, whenever that was: the start its host recorded, or for Claude Code, which records none, its earliest message; PRs counts recorded links, not merged pull requests. A verified sub-session is listed under the session that created it, collapsed; a group sits where its newest returned member does. A parent with no span returned in this report is only named, on a row of its own that has no activity lane, measurement or start, with the count of its returned sub-sessions; sub-sessions outside the returned spans are not counted.`;
};

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
            {clockTime(start + (span * index) / 4, window, index === 4)}
          </span>
        ))}
      </span>
    </span>
  );
}

/**
 * Returned active spans on the report's fixed recent axis, independent of the
 * selected range.
 *
 * One row per session, carrying every span the report returned for it. A row
 * is never a whole session: the axis covers only the recent window, and the
 * report caps how many spans it returns.
 *
 * A session with a verified parent is listed under that parent, collapsed
 * behind a disclosure on the parent's row — or, when the report returned no
 * span for the parent, on a row that only names it and counts its returned
 * sub-sessions, with no measurement of its own. Opening a group lists each
 * child's own row as it would read ungrouped. This is the compact Dashboard's
 * arrangement only: every child is still its own session on Sessions.
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
 * The cost column is the one measurement here, and it is taken over the
 * whole fixed window rather than over the drawn spans, so the display cap
 * never reduces a number. The column's own definition says so.
 */
export function ActivityLanes({ report, range }: { report: DashboardMetrics; range: TimeRange }) {
  const { lanes, lane_start_ms: start, lane_end_ms: end, window } = report;
  const span = Math.max(1, end - start);
  const hours = laneHours(report);
  const sessions = groupSessionLanes(lanes);
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
  // Context is addressed by identity, not by position: the report orders it by
  // identifier while the rows keep the order the spans were returned in.
  const context = new Map<string, DashboardLaneSession>(
    report.lane_sessions.map((session) => [session.session_id, session]),
  );
  // A session the report named no context row for keeps an honest unknown
  // rather than borrowing another row's repository.
  const found = (lane: SessionLane) => context.get(lane.sessionId) ?? null;
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
  const linkOf = (lane: SessionLane) =>
    verifiedParent({ id: lane.sessionId, parent: found(lane)?.parent });
  const compaction = useSessionCompactions(
    JSON.stringify(['dashboard', range]),
    sessions
      .filter((lane) => visible.has(lane.sessionId) && !linkOf(lane))
      .map((lane) => lane.sessionId),
    { keepResolved: true, retryTransient: true },
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
  const costText = (lane: SessionLane) => {
    const session = found(lane);
    if (!session) return 'cost unknown: no indexed context';
    const { cost } = session;
    if (!cost) return 'cost unknown: no indexed session';
    if (cost.selected_observations === 0) return 'no selected responses to price';
    if (cost.priced_observations === 0) return `cost unknown: ${unpricedNames(cost)}`;
    if (cost.total_usd !== null)
      return `${usd(cost.total_usd)} API-equivalent cost of ${plural(cost.selected_observations, 'response')}`;
    return `at least ${usd(cost.priced_subtotal_usd)} API-equivalent cost: ${partialText(cost)}`;
  };
  const startedText = (lane: SessionLane) => {
    const session = found(lane);
    if (!session) return 'start unknown: no indexed context';
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
    return `${lane.host} session ${lane.sessionId}, repository ${
      session?.repo ?? 'unknown'
    }, branch ${session?.branch ?? 'unknown'}${title ? `, titled ${title}` : ''}${
      created ? `, sub-session of ${created.name} (${created.parent.session_id})` : ''
    }: ${plural(lane.spans.length, 'active span')} in the last ${hours} hours — ${spanText(
      lane,
    )}. ${startedText(lane)}; ${prText(lane)}. Over the whole ${hours} hours: ${costText(lane)}.`;
  };

  // Only the returned sessions are placed, each once: under its parent's row,
  // else under a row that names an absent parent. The table draws the rows
  // this union holds, so a collapsed child is unmounted, not hidden.
  const { rows } = laneRows(sessions, linkOf, open);
  const nameOf = (lane: SessionLane) => titleOf(lane) ?? `Session ${shortId(lane.sessionId)}`;
  const groupText = (row: Row) =>
    row.children === 0
      ? ''
      : ` ${plural(row.children, 'returned sub-session')} ${
          row.expanded ? 'listed below' : 'collapsed under this row'
        }.`;
  const absentText = (row: Extract<Row, { kind: 'absent' }>) =>
    `Sub-sessions of ${row.parent.host} session ${row.parent.session_id}, which has no span returned in this report: it is only named here, with no measurement of its own.${groupText(
      row,
    )} Only sub-sessions with returned spans are counted.`;
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
              <GroupToggle row={row} name={name} onToggle={toggle} />
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
            {row.children > 0 && <GroupToggle row={row} name={nameOf(lane)} onToggle={toggle} />}
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
            <span className="sr-only">{` — ${rowText(lane)}${groupText(row)}`}</span>
          </span>
        );
      },
    },
    {
      key: 'compactions',
      header: <span title={COMPACTION_MEANING}>Compactions</span>,
      width: '92px',
      render: onLane((lane) =>
        linkOf(lane) ? null : <CompactionBadge outcome={compaction(lane.sessionId)} />,
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
            title={`${prText(lane)} · all evidence levels, all indexed time, not merge status`}
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
      header: (
        <span title="When the session began, whenever that was: the start its host recorded, or for Claude Code its earliest message">
          started
        </span>
      ),
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
      // place what window it covers, so the number is never read as a total of
      // the spans drawn beside it.
      header: (
        <RulePopover ruleId="M-04" context={laneCostDefinition(hours)}>
          <button
            type="button"
            className="xt-table-metric-header"
            aria-label={`Cost over the last ${hours} hours, definition`}
          >
            cost
          </button>
        </RulePopover>
      ),
      // Wide enough for "$1,234+" and "$999.99+" without an ellipsis.
      width: '72px',
      align: 'right',
      render: onLane((lane) => <LaneCost session={found(lane)} />),
    },
  ];

  return (
    <div
      className="xt-lanes"
      data-empty={sessions.length === 0 || undefined}
      style={
        {
          // From the rows drawn, so opening a group grows the table up to the
          // card's share and scrolls inside it beyond that.
          '--rows': Math.min(rows.length, MIN_LANE_ROWS),
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
        emptyMessage={`No active span in the last ${hours} hours (${clockTime(start, window)} – ${clockTime(end, window)}).`}
      />
    </div>
  );
}

/**
 * The disclosure for a group: a real button in the name cell, so the row keeps
 * its columns and its 22px height. Its name says whose sub-sessions and how
 * many were returned; `aria-expanded` says whether they are listed. It controls
 * no single element — the children are rows of the same table, mounted only
 * while open — so it names none with `aria-controls`.
 */
function GroupToggle({
  row,
  name,
  onToggle,
}: {
  row: Row;
  name: string;
  onToggle: (key: string) => void;
}) {
  const label = `${plural(row.children, 'returned sub-session')} of ${name}`;
  return (
    <button
      type="button"
      className="xt-lane-toggle"
      // WebKit leaves a button out of the Tab order unless it is named here.
      tabIndex={0}
      aria-expanded={row.expanded}
      aria-label={label}
      title={`${row.expanded ? 'Hide' : 'Show'} ${label}`}
      onClick={() => onToggle(row.key)}
    >
      <span className="xt-lane-chevron" aria-hidden="true">
        ›
      </span>
      <span aria-hidden="true">{row.children.toLocaleString('en-US')}</span>
    </button>
  );
}

/** The cost column's definition, in plain words. */
export const laneCostDefinition = (hours: number) =>
  `API-equivalent cost of every token each session used over the whole ${hours}-hour window, including active spans this table does not draw, at each model's public rate. "+" means some responses have no published price, so the amount is a floor. "—" means nothing could be priced. Codex responses with no service tier recorded are priced at OpenAI's default (standard) tier. A measured zero is shown as $0.00.`;

/**
 * A lane amount: cents below $1,000, whole dollars from there, so the
 * column stays narrow. "<$0.01" for a positive amount below a cent.
 */
const wholeDollars = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 0,
});
export const laneUsd = (value: number) =>
  value >= 999.995 ? wholeDollars.format(value) : usd(value);

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
  const model = gap.model ?? 'unknown model';
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
    ? `none of ${plural(cost.selected_observations, 'selected response')} could be priced`
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

/** One lane session's cost over the lane window. */
function LaneCost({ session }: { session: DashboardLaneSession | null | undefined }) {
  const unknown = (reason: string) => (
    <MetricCell value={null} align="right" size={10.5} reason={reason} />
  );
  if (!session) return unknown(NO_CONTEXT);
  const { cost } = session;
  if (!cost) return unknown(NO_SESSION);
  if (cost.selected_observations === 0) return unknown(NO_RESPONSES);
  // A model name keeps its own spelling, so the reason is not capitalised.
  if (cost.priced_observations === 0) return unknown(unpricedNames(cost));
  const assumed =
    cost.assumed_tier_observations > 0
      ? ` ${plural(cost.assumed_tier_observations, 'Codex response')} recorded no service tier and ${
          cost.assumed_tier_observations === 1 ? 'is' : 'are'
        } priced at OpenAI's default (standard) tier.`
      : '';
  if (cost.total_usd !== null)
    return (
      <MetricCell
        value={laneUsd(cost.total_usd)}
        align="right"
        size={10.5}
        title={`${usd(cost.total_usd)} API-equivalent, ${
          cost.selected_observations === 1
            ? '1 response priced'
            : `all ${plural(cost.selected_observations, 'response')} priced`
        }.${assumed}`}
      />
    );
  return (
    <MetricCell
      value={`${laneUsd(cost.priced_subtotal_usd)}+`}
      align="right"
      size={10.5}
      title={`At least ${usd(cost.priced_subtotal_usd)}: ${partialText(cost)}; the unpriced ones are not included.${assumed}`}
    />
  );
}
