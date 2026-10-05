import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { Link, useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import { queryKeys } from '../data/query-client';
import type { DashboardLaneSession } from '../data/generated/DashboardLaneSession';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { SessionPrLink } from '../data/generated/SessionPrLink';
import type { SessionRow } from '../data/generated/SessionRow';
import { EvidenceDot } from '../kit/Badge';
import { FilterMenu } from '../kit/FilterMenu';
import { Toggle } from '../kit/Toggle';
import { DataTable, type Column } from '../kit/DataTable';
import { HostGlyph } from '../kit/HostGlyph';
import { Search } from '../kit/Search';
import { Button } from '../kit/Button';
import { SectionCard } from '../kit/SectionCard';
import { MetricCell } from '../kit/MetricCell';
import { count, tokens as formatTokens } from '../kit/format';
import { groupSessionLanes, type SessionLane } from './dashboard/lanes';
import { rangeDays } from './dashboard/present';
import { useSelectedRange } from './dashboard/range';
import { agentDuration } from './agent-duration';
import { continuous } from './metric-format';
import { contextTitle, displayTitle, repoName, shortId } from './session-context';
import {
  formatHosts,
  parseHosts,
  parseSearch,
  parseWithPrs,
  SEARCH_MAX,
  sessionHosts,
  sessionHref,
  sessionParams,
  type SessionHost,
} from './session-search';
import { RangeIssueFlag, SessionsIssueFlag, type IndexIssue } from './SessionsIssues';
import { SessionsSummary, SUMMARY_SCOPE, SUMMARY_SCOPE_SHORT } from './SessionsSummary';
import { ParentMarker, parentName, verifiedParent } from './session-parent';
import { useSessionTitles, useVisibleIds, type TitleRow } from './session-titles';
import {
  isLiveSessionHost,
  useLiveSessionStatus,
  type LiveSessionStatus,
} from './live-session-status';
import { LiveSessionBadge } from './LiveSessionBadge';
import { useNativeIndexStatus } from './useAppInfo';
import {
  COMPACTION_MEANING,
  CompactionBadge,
  useSessionCompactions,
  type CompactionLookup,
} from './session-compactions';
import {
  AgentTime,
  evidenceWords,
  HandsOff,
  handsOffReason,
  indexed,
  MetricHeader,
  NOT_INDEXED,
} from './session-cells';
import '../styles/sessions.css';

const hostLabels: Record<SessionHost, string> = {
  claude: 'Claude Code',
  codex: 'Codex',
  cursor: 'Cursor',
};
const hostOptions = sessionHosts.map((id) => ({ id, label: hostLabels[id] }));
const RECENT_SESSION_LIMIT = 8;

/**
 * A session's title: its host's own, read from the original source for this
 * view and never stored, else a title the index saved, else none.
 */
type HostTitles = (id: string) => string | null;
const titleOf = (row: SessionRow, hostTitle: HostTitles) =>
  displayTitle(hostTitle(row.id) ?? row.title, row.automated_review);
/** A loaded row by its exact identity, or nothing when it is not loaded. */
type LoadedRow = (id: string) => SessionRow | undefined;

/** Native start, as the reference states it: day and 24-hour time. */
const startedFormat: Intl.DateTimeFormatOptions = {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
};
const recordedFormat: Intl.DateTimeFormatOptions = { ...startedFormat, year: 'numeric' };

/** One link's own evidence in words, so an inferred link never reads as exact. */
const linkLabel = (link: SessionPrLink) =>
  `Pull request #${link.number} in ${link.repository}, ${evidenceWords[link.confidence]} evidence` +
  (link.title ? `: ${link.title}` : '');

function PrBadge({ link }: { link: SessionPrLink }) {
  return (
    // Not a link: nothing here opens the network. The whole identity, its
    // evidence and any stored title are in the label and the tooltip.
    <span
      className="xt-session-pr"
      data-confidence={link.confidence}
      role="img"
      aria-label={linkLabel(link)}
      title={`${link.url} · ${evidenceWords[link.confidence]} evidence${
        link.title ? ` · ${link.title}` : ''
      }`}
    >
      <EvidenceDot evidence={link.confidence} />#{link.number}
    </span>
  );
}

function PrLinks({ row }: { row: SessionRow }) {
  if (row.pr_links.length === 0)
    return (
      <span className="xt-session-mono" title="No pull request link was recorded">
        <span aria-hidden="true">—</span>
        <span className="sr-only">No recorded pull request</span>
      </span>
    );
  const more = row.pr_links.length - 2;
  return (
    <span className="xt-session-prs">
      {row.pr_links.slice(0, 2).map((link) => (
        <PrBadge key={link.url} link={link} />
      ))}
      {more > 0 && (
        <span className="xt-session-mono" title={`${more} more in this row's details`}>
          +{more}
        </span>
      )}
    </span>
  );
}

/**
 * The columns, built with the list's own address so every row can open its
 * session and come back to this list exactly as it is. The address is a
 * parameter rather than a module-level read because a column definition is
 * data: it has no hooks of its own and must not acquire any; the host titles
 * read for this view are passed in the same way.
 */
const buildColumns = (
  address: URLSearchParams,
  hostTitle: HostTitles,
  loaded: LoadedRow,
  compaction: CompactionLookup,
  observe: ReturnType<typeof useVisibleIds>['observe'],
  liveStatus: (id: string) => LiveSessionStatus | undefined,
): Column<SessionRow>[] => [
  {
    key: 'host',
    header: <span className="sr-only">Host</span>,
    width: '20px',
    render: (row) => <HostGlyph host={row.host} size={18} />,
  },
  {
    // The host's own title when its source has one, else a title the index
    // saved, else the session's own identity; the repository, branch and
    // model sit under it. No title is derived from what the session said.
    key: 'session',
    header: 'session · repo · branch',
    width: 'minmax(180px, 1fr)',
    render: (row) => {
      const title = titleOf(row, hostTitle);
      const parent = verifiedParent(row);
      // A parent already loaded in this list is named as its own row reads
      // now; one filtered out or on a later page keeps the name the report
      // sent, and no title is read for it.
      const shown = parent ? loaded(parent.session_id) : undefined;
      const meta = (
        <span className="xt-session-meta" title={contextTitle(row.repo, row.branch)}>
          {repoName(row.repo) ?? 'Unknown repository'}
          {row.branch && (
            <span className="xt-session-branch">
              {' '}
              <span aria-hidden="true">⑂</span> {row.branch}
            </span>
          )}
          <span aria-hidden="true"> · </span>
          <span>{row.model ?? 'Unknown model'}</span>
          {row.has_conflict && (
            // Kept in the row, not only in its details: the index holds
            // observations of this session that disagree.
            <span className="xt-session-flag" data-flagged>
              {' '}
              · source conflict
            </span>
          )}
        </span>
      );
      return (
        <div className="xt-session-name" ref={observe} data-visible-id={row.id}>
          <span className="xt-session-heading xt-session-title-line">
            <Link
              className="xt-session-open"
              to={sessionHref(row.id, address)}
              aria-label={`Open session ${title ? `${title}, ` : ''}${row.id}`}
              title={title ? `${title} · ${row.id}` : row.id}
            >
              {title ?? `Session ${shortId(row.id)}`}
            </Link>
            {isLiveSessionHost(row.host) && (
              <LiveSessionBadge host={row.host} status={liveStatus(row.id)} />
            )}
          </span>
          {parent ? (
            // The parent leads the second line, where the row's other
            // context already is, so the row keeps its height; the context
            // gives way to the ellipsis first. The link carries this list's
            // own address, so the parent's Back returns here as it is.
            <span className="xt-session-line">
              <ParentMarker
                className="xt-session-parent"
                parent={parent}
                name={parentName(
                  parent,
                  shown && shown.host === parent.host ? titleOf(shown, hostTitle) : undefined,
                )}
                to={sessionHref(parent.session_id, address)}
              />
              {meta}
            </span>
          ) : (
            meta
          )}
        </div>
      );
    },
  },
  {
    key: 'compactions',
    header: <span title={COMPACTION_MEANING}>Compactions</span>,
    width: '92px',
    render: (row) =>
      verifiedParent(row) ? null : <CompactionBadge outcome={compaction(row.id)} />,
  },
  {
    key: 'prs',
    header: (
      <span title="Recorded session → pull request links; the dot is the link's evidence (M-13). A link says the session referenced it, not that it merged.">
        PRs
      </span>
    ),
    width: '150px',
    render: (row) => <PrLinks row={row} />,
  },
  {
    key: 'started',
    // The start the host recorded. Claude Code records none, so a Claude
    // session shows its earliest imported message instead (for a forked
    // session that can be a message copied from its parent). Other hosts with
    // no recorded start stay unknown.
    header: (
      <span title="When the session began: the start its host recorded, or for Claude Code its earliest message.">
        started
      </span>
    ),
    width: '104px',
    render: (row) =>
      row.started_at_ms === null ? (
        <MetricCell
          value={null}
          reason="No start is known for this session; first recorded work is in its details"
        />
      ) : (
        <time
          className="xt-session-mono"
          dateTime={new Date(row.started_at_ms).toISOString()}
          title={new Date(row.started_at_ms).toLocaleString(undefined, recordedFormat)}
        >
          {new Date(row.started_at_ms).toLocaleString(undefined, startedFormat)}
        </time>
      ),
  },
  {
    key: 'human',
    header: <MetricHeader label="msgs" name="Human messages" ruleId="M-02" />,
    width: '52px',
    align: 'right',
    render: (row) => (
      <MetricCell
        value={indexed(row)?.human_messages}
        format={count}
        align="right"
        reason={indexed(row) ? 'Human classification is unmeasured (M-02)' : NOT_INDEXED}
      />
    ),
  },
  {
    key: 'tools',
    // The Dashboard's tool-call count cites the same rule.
    header: <MetricHeader label="tools" name="Tool calls" ruleId="M-17" />,
    width: '56px',
    align: 'right',
    render: (row) => (
      <MetricCell
        value={indexed(row)?.tool_calls}
        format={count}
        align="right"
        reason={
          indexed(row)
            ? 'A record in this window did not state how many tool calls it made'
            : NOT_INDEXED
        }
      />
    ),
  },
  {
    key: 'agent',
    header: <MetricHeader label="agent min" name="Agent minutes" ruleId="M-05" />,
    width: '64px',
    align: 'right',
    render: (row) => <AgentTime row={row} />,
  },
  {
    key: 'hands-off',
    header: <MetricHeader label="hands-off" name="Hands-off median minutes" ruleId="M-09" />,
    width: '68px',
    align: 'right',
    render: (row) => <HandsOff row={row} />,
  },
  {
    key: 'output',
    header: <MetricHeader label="output" name="Output tokens" ruleId="M-04" />,
    width: '64px',
    align: 'right',
    render: (row) => (
      <MetricCell
        value={indexed(row)?.tokens.counters.output_tokens}
        format={formatTokens}
        align="right"
        reason={
          indexed(row) ? 'Selected responses do not all state output tokens (M-04)' : NOT_INDEXED
        }
      />
    ),
  },
];

/**
 * What the design's row has no room for, kept one disclosure away instead of
 * dropped: when work was first recorded, how many records the index holds for
 * it, whether its sources agree, the M-04 total, the hands-off sample, and
 * every pull request link with its evidence.
 */
function SessionDetails({ row }: { row: SessionRow }) {
  const metrics = indexed(row);
  const handsOff = row.hands_off;
  return (
    <dl className="xt-session-details">
      <div>
        <dt>First recorded</dt>
        <dd>
          {row.first_ts ? (
            <time dateTime={row.first_ts}>
              {new Date(row.first_ts).toLocaleString(undefined, recordedFormat)}
            </time>
          ) : (
            '—'
          )}{' '}
          <small>earliest visible work, including copies inherited from a fork</small>
        </dd>
      </div>
      <div>
        <dt>Records</dt>
        <dd>
          {row.record_count.toLocaleString()}{' '}
          <small>visible, including inherited copies; copied work is measured once</small>
        </dd>
      </div>
      <div>
        <dt>Source</dt>
        <dd data-flagged={row.has_conflict || undefined}>
          {row.has_conflict
            ? 'Some source observations disagree'
            : 'No source observation disagrees'}
        </dd>
      </div>
      <div>
        <dt>Agent time</dt>
        <dd>
          {metrics ? (
            <>
              {agentDuration(metrics.agent_ms).visible}{' '}
              <small>exactly {agentDuration(metrics.agent_ms).exact} active in this range</small>
            </>
          ) : (
            NOT_INDEXED
          )}
        </dd>
      </div>
      <div>
        <dt>Total tokens</dt>
        <dd>
          <MetricCell
            value={metrics?.tokens.counters.total_tokens}
            format={formatTokens}
            reason={
              metrics ? 'Selected usage counters are absent or incomplete (M-04)' : NOT_INDEXED
            }
          />
        </dd>
      </div>
      <div>
        <dt>Hands-off</dt>
        <dd>
          {handsOff.state === 'measured' && handsOff.median_min !== null
            ? `median ${continuous(handsOff.median_min)} min over ${handsOff.n} ${
                handsOff.n === 1 ? 'stretch' : 'stretches'
              }`
            : handsOffReason(row)}
        </dd>
      </div>
      <div>
        <dt>Pull requests</dt>
        <dd>
          {row.pr_links.length === 0 ? (
            'None recorded'
          ) : (
            <ul>
              {row.pr_links.map((link) => (
                <li key={link.url}>
                  <EvidenceDot evidence={link.confidence} /> {link.repository}#{link.number} ·{' '}
                  {evidenceWords[link.confidence]}
                  {link.title ? ` · ${link.title}` : ''}
                </li>
              ))}
            </ul>
          )}
        </dd>
      </div>
    </dl>
  );
}

/** Recent recorded spans are a way into old-start sessions, not a live state. */
function RecentIndexedActivity({
  address,
  metrics,
  failed,
  sessions,
  foundCount,
  hostTitle,
  compaction,
  liveStatus,
}: {
  address: URLSearchParams;
  metrics: DashboardMetrics | undefined;
  failed: boolean;
  sessions: readonly SessionLane[];
  foundCount: number;
  hostTitle: HostTitles;
  compaction: CompactionLookup;
  liveStatus: (id: string) => LiveSessionStatus | undefined;
}) {
  // The report orders this context by ID, not by the activity shown here.
  const context = new Map<string, DashboardLaneSession>(
    metrics?.lane_sessions.map((session) => [
      `${session.host}\u0000${session.session_id}`,
      session,
    ]) ?? [],
  );

  return (
    <section className="xt-sessions-recent" aria-label="Recent indexed activity">
      <div className="xt-sessions-recent-heading">
        <h2>Recent indexed activity · last 48 hours</h2>
        <p>
          Recorded activity is history; Claude Code and Codex badges show their runtime’s live
          state.
        </p>
      </div>
      <p className="xt-sessions-recent-note">
        {metrics && !failed && sessions.length > 0 && foundCount > RECENT_SESSION_LIMIT && (
          <span>
            {RECENT_SESSION_LIMIT} most recent indexed sessions with activity in the last 48 hours
          </span>
        )}{' '}
        <span>Search and filters affect All sessions only.</span>
      </p>
      {failed ? (
        <p role="status">Recent indexed activity could not be loaded.</p>
      ) : !metrics ? (
        <p role="status">Reading recent indexed activity…</p>
      ) : (
        <>
          {sessions.length === 0 ? (
            <p>No indexed activity in the last 48 hours.</p>
          ) : (
            <>
              <ol className="xt-sessions-recent-list">
                {sessions.map((lane) => {
                  const saved = context.get(lane.key);
                  const title = displayTitle(
                    hostTitle(lane.sessionId) ?? saved?.title ?? null,
                    saved?.automated_review ?? false,
                  );
                  const host = hostLabels[lane.host as SessionHost] ?? lane.host;
                  return (
                    <li key={lane.key}>
                      <span className="xt-sessions-recent-state">
                        <span className="xt-sessions-recent-host">{host}</span>
                        {isLiveSessionHost(lane.host) && (
                          <LiveSessionBadge host={lane.host} status={liveStatus(lane.sessionId)} />
                        )}
                      </span>
                      <span className="xt-session-title-line">
                        <Link
                          to={sessionHref(lane.sessionId, address)}
                          aria-label={`Open recent ${host} session ${title ? `${title}, ` : ''}${lane.sessionId}`}
                          title={`${title ? `${title} · ` : ''}${lane.sessionId} · ${contextTitle(saved?.repo ?? null, saved?.branch ?? null)}`}
                        >
                          {title && <span className="xt-sessions-recent-name">{title}</span>}
                          <span className="xt-sessions-recent-id">{lane.sessionId}</span>
                        </Link>
                        {!verifiedParent({ id: lane.sessionId, parent: saved?.parent }) && (
                          <CompactionBadge outcome={compaction(lane.sessionId)} />
                        )}
                      </span>
                      <time dateTime={new Date(lane.lastMs).toISOString()}>
                        Last recorded{' '}
                        {new Date(lane.lastMs).toLocaleString(undefined, recordedFormat)}
                      </time>
                    </li>
                  );
                })}
              </ol>
            </>
          )}
          {metrics.lanes_truncated && (
            <p className="xt-sessions-recent-limit">
              Only the most recent activity spans were returned; earlier activity can be missing.
            </p>
          )}
        </>
      )}
    </section>
  );
}

export function SessionsPage() {
  const sectionRef = useRef<HTMLElement>(null);
  const anchorRef = useRef<{ id: string; offset: number; scrollTop: number } | null>(null);
  const listIdentityRef = useRef('');
  const { source } = useData();
  const index = useNativeIndexStatus();
  // The filters live in the address, so a link can open this list already
  // filtered, and so the browser's own back and forward — and a session's own
  // Back link — restore what was filtered rather than an empty list. Each is
  // validated here: an unsupported host is dropped, and a longer search than
  // the store accepts is bounded, so a hand-edited address degrades to a
  // working list instead of a failed query.
  const [params, setParams] = useSearchParams();
  const supportedRecent = source.kind !== 'preview' && !params.has('pr');
  const search = parseSearch(params.get(sessionParams.search));
  const hosts = parseHosts(params.get(sessionParams.host));
  const withPrs = parseWithPrs(params.get(sessionParams.withPrs));
  // What the box shows while it is being typed in, before the debounce writes
  // it to the address. It follows the address whenever that changes under it —
  // a link, or back and forward — and is otherwise the user's own typing.
  const [input, setInput] = useState(search);
  const [shown, setShown] = useState(search);
  if (search !== shown) {
    setShown(search);
    setInput(search);
  }
  const [expanded, setExpanded] = useState<string[]>([]);
  const scopeId = useId();
  // The Shell's selected range is this list's explicit event window (M-01);
  // every measured column below describes that window, not whole sessions. A
  // link may name that range in the address, and the Shell applies it there.
  const range = useSelectedRange();
  const days = rangeDays[range];
  // Stable while the address is, so the debounce below is not restarted by an
  // unrelated re-render.
  const setFilter = useCallback(
    (key: string, value: string | null) =>
      setParams(
        (previous) => {
          const updated = new URLSearchParams(previous);
          if (value) updated.set(key, value);
          else updated.delete(key);
          return updated;
        },
        // Filtering is not a page of its own: replacing the entry keeps Back
        // meaning the previous page, as it did before the filters were in the
        // address, and keeps a keystroke out of the history stack.
        { replace: true },
      ),
    [setParams],
  );
  useEffect(() => {
    if (input === search) return;
    const timer = setTimeout(() => setFilter(sessionParams.search, input), 200);
    return () => clearTimeout(timer);
  }, [input, search, setFilter]);
  const hostKey = hosts?.join(',') ?? null;
  const listIdentity = JSON.stringify([days, search, hostKey, withPrs]);
  const query = useInfiniteQuery({
    // Every filter is part of the identity, so a change starts again from the
    // first page rather than appending pages of a different list.
    queryKey: [...queryKeys.sessions(days), search, hostKey, withPrs],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) =>
      source.sessionsList(
        { search, hosts: hostKey ? hostKey.split(',') : null, withPrs },
        pageParam,
        days,
      ),
    getNextPageParam: (page) => page.next ?? undefined,
    enabled: supportedRecent,
  });
  const rows = useMemo(() => query.data?.pages.flatMap((page) => page.rows) ?? [], [query.data]);
  // A clock or data event replays the pages already loaded. Remember the first
  // visible row before that read, and keep following it if the user scrolls
  // while the read is pending. Restore its latest pixel offset when it lands.
  // A new range or filter is a new list and starts at its own first page.
  useLayoutEffect(() => {
    if (listIdentityRef.current !== listIdentity) {
      listIdentityRef.current = listIdentity;
      anchorRef.current = null;
      return;
    }
    if (!query.isFetching || rows.length === 0) return;
    const scroll = sectionRef.current?.querySelector<HTMLElement>('.xt-table-scroll');
    if (!scroll) return;
    const capture = () => {
      const blocks = [...scroll.querySelectorAll<HTMLElement>('.xt-table-block')];
      const top =
        scroll.getBoundingClientRect().top +
        (scroll.querySelector<HTMLElement>('.xt-table-head')?.getBoundingClientRect().height ?? 0);
      const index = blocks.findIndex((block) => block.getBoundingClientRect().bottom > top);
      if (index < 0 || !rows[index]) return;
      anchorRef.current = {
        id: rows[index].id,
        offset: blocks[index].getBoundingClientRect().top - top,
        scrollTop: scroll.scrollTop,
      };
    };
    capture();
    scroll.addEventListener('scroll', capture, { passive: true });
    return () => scroll.removeEventListener('scroll', capture);
  }, [listIdentity, query.isFetching, rows]);
  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor || query.isFetching || listIdentityRef.current !== listIdentity) return;
    const scroll = sectionRef.current?.querySelector<HTMLElement>('.xt-table-scroll');
    if (scroll) {
      const index = rows.findIndex((row) => row.id === anchor.id);
      const block = scroll.querySelectorAll<HTMLElement>('.xt-table-block')[index];
      const top =
        scroll.getBoundingClientRect().top +
        (scroll.querySelector<HTMLElement>('.xt-table-head')?.getBoundingClientRect().height ?? 0);
      scroll.scrollTop = block
        ? scroll.scrollTop + block.getBoundingClientRect().top - top - anchor.offset
        : Math.min(anchor.scrollTop, Math.max(0, scroll.scrollHeight - scroll.clientHeight));
    }
    anchorRef.current = null;
  }, [listIdentity, query.isFetching, query.dataUpdatedAt, rows]);
  const recentReport = useQuery({
    queryKey: queryKeys.dashboard(days),
    queryFn: () => source.dashboard(days),
    enabled: supportedRecent,
  });
  // A failed refresh can retain the previous report in the query cache. Its
  // recent rows must disappear until a new report succeeds. A disabled query
  // can also expose cached data from a supported view, so guard that here.
  const recentMetrics = supportedRecent && !recentReport.isError ? recentReport.data : undefined;
  const foundRecent = useMemo(
    () =>
      recentMetrics
        ? groupSessionLanes(recentMetrics.lanes).sort((a, b) => b.lastMs - a.lastMs)
        : [],
    [recentMetrics],
  );
  const recentSessions = useMemo(() => foundRecent.slice(0, RECENT_SESSION_LIMIT), [foundRecent]);
  const { observe, visible } = useVisibleIds();
  const liveStatus = useLiveSessionStatus(
    JSON.stringify(['sessions', listIdentity, params.toString()]),
    supportedRecent
      ? [
          ...recentSessions
            .filter((lane) => isLiveSessionHost(lane.host))
            .map((lane) => lane.sessionId),
          ...(!query.isError ? rows : [])
            .filter((row) => isLiveSessionHost(row.host) && visible.has(row.id))
            .map((row) => row.id),
        ]
      : [],
  );
  // One title read queue serves the list and the recent section. Wait for
  // both initial queries before starting it, so an overlapping session is
  // named once. Loaded list rows keep priority; only recent IDs not already
  // loaded follow them. A shared version changes when either source changes.
  const titleRows = useMemo<TitleRow[]>(() => {
    if (!supportedRecent || query.isPending || recentReport.isPending) return [];
    const recentById = new Map(recentSessions.map((lane) => [lane.sessionId, lane.lastMs]));
    const named = new Set<string>();
    const result: TitleRow[] = [];
    for (const row of rows) {
      if (named.has(row.id)) continue;
      named.add(row.id);
      const lastMs = recentById.get(row.id);
      result.push({
        id: row.id,
        version: lastMs === undefined ? row.record_count : `${row.record_count}\u0000${lastMs}`,
      });
    }
    for (const lane of recentSessions) {
      if (named.has(lane.sessionId)) continue;
      named.add(lane.sessionId);
      result.push({ id: lane.sessionId, version: lane.lastMs });
    }
    return result;
  }, [supportedRecent, query.isPending, recentReport.isPending, recentSessions, rows]);
  const titleVersions = useMemo(
    () => new Map(titleRows.map((row) => [row.id, row.version])),
    [titleRows],
  );
  const readTitle = useSessionTitles(
    JSON.stringify(['sessions', days, search, hostKey, withPrs]),
    titleRows,
  );
  const hostTitle = useCallback<HostTitles>(
    (id) => {
      const version = titleVersions.get(id);
      return version === undefined ? null : readTitle(id, version);
    },
    [readTitle, titleVersions],
  );
  // A sub-session names its parent by the parent's own row when this list
  // has loaded it, and only then; nothing is read for a parent it has not.
  const pages = query.data?.pages;
  const recentContext = new Map(
    recentMetrics?.lane_sessions.map((session) => [session.session_id, session]),
  );
  const compaction = useSessionCompactions(
    JSON.stringify(['sessions', days, search, hostKey, withPrs]),
    [
      ...rows.filter((row) => !verifiedParent(row)).map((row) => row.id),
      ...recentSessions
        .filter(
          (lane) =>
            !verifiedParent({
              id: lane.sessionId,
              parent: recentContext.get(lane.sessionId)?.parent,
            }),
        )
        .map((lane) => lane.sessionId),
    ],
  );
  const loaded = useMemo<LoadedRow>(() => {
    const byId = new Map(pages?.flatMap((page) => page.rows.map((row) => [row.id, row])));
    return (id) => byId.get(id);
  }, [pages]);
  // Each cell shares this view's title and live-state lookups.
  const columns = useMemo(
    () => buildColumns(params, hostTitle, loaded, compaction, observe, liveStatus),
    [params, hostTitle, loaded, compaction, observe, liveStatus],
  );
  const incomplete = index.data?.hosts.some((h) => h.state !== 'complete');
  const scanning =
    index.data?.phase.phase === 'scanning' || index.data?.hosts.some((h) => h.state === 'pending');
  const degraded = index.data?.freshness.freshness === 'degraded';
  const indexUnavailable =
    source.kind === 'native' &&
    (index.isError || ['disabled', 'stopped'].includes(index.data?.phase.phase ?? ''));

  // The most serious known state, stated in the heading's warning flag with
  // its sentence and the way to Settings. Nothing is said when the index is
  // healthy or its status has not been read, but silence is not a claim that
  // it is healthy.
  const coverage: IndexIssue | null = indexUnavailable
    ? {
        state: 'Index unavailable',
        text: 'Indexing is unavailable; showing previously indexed history.',
      }
    : scanning
      ? { state: 'Indexing', text: 'History is still being indexed; this list may be incomplete.' }
      : degraded
        ? {
            state: 'Updates interrupted',
            text: 'Live indexing is interrupted; this list may be out of date.',
          }
        : incomplete
          ? { state: 'History incomplete', text: 'Some history could not be fully indexed.' }
          : null;
  // Only a page that shows the range summary reads the range report, so only
  // there can the flag count untimed history.
  const summarised = !params.has('pr') && source.kind !== 'preview';

  return (
    <section className="xt-sessions" ref={sectionRef}>
      <div className="xt-sessions-heading">
        <div className="xt-sessions-title">
          <h1>Sessions</h1>
          {/* With the range summary, the one line says what its tiles cover;
              without it there is no summary to describe. */}
          <p>{summarised ? SUMMARY_SCOPE_SHORT : 'Every indexed session on this Mac'}</p>
          {summarised && (
            // The range summary's own description, whole: its tiles do not
            // follow the filters.
            <span id={scopeId} className="sr-only">
              {SUMMARY_SCOPE}
            </span>
          )}
        </div>
        <div className="xt-sessions-filters">
          <Search
            label="Search sessions"
            // Search reads the index; a host title shown from a local source
            // is not indexed, so the box does not offer to search titles.
            placeholder="Search ID, repo or branch…"
            value={input}
            onValueChange={setInput}
            maxLength={SEARCH_MAX}
          />
          <FilterMenu
            label="Filter by host"
            options={hostOptions}
            selected={hosts ?? sessionHosts}
            // A list of no hosts is not a filter anyone asks for: the last
            // selected host stays selected.
            onChange={(next) => {
              if (next.length > 0) setFilter(sessionParams.host, formatHosts(next));
            }}
          />
          <Toggle
            label="With PRs only"
            checked={withPrs}
            onChange={(checked) => setFilter(sessionParams.withPrs, checked ? '1' : null)}
          />
          {summarised ? (
            <RangeIssueFlag index={coverage} range={range} />
          ) : (
            <SessionsIssueFlag index={coverage} />
          )}
        </div>
      </div>
      {params.has('pr') ? (
        <p role="status">
          Pull request filtering is not available yet. <Link to="/sessions">View all sessions</Link>
        </p>
      ) : source.kind === 'preview' ? (
        <p>Open the desktop app to browse your indexed sessions.</p>
      ) : (
        <>
          <SessionsSummary range={range} describedBy={scopeId} />
          <RecentIndexedActivity
            address={params}
            metrics={recentMetrics}
            failed={recentReport.isError}
            sessions={recentSessions}
            foundCount={foundRecent.length}
            hostTitle={hostTitle}
            compaction={compaction}
            liveStatus={liveStatus}
          />
          <SectionCard
            title="All sessions"
            meta={`${rows.length} loaded · sorted by started ↓, first recorded when a start is unknown · measured over the last ${range.replace('d', ' days')}`}
            right={
              <Button
                variant="outline"
                height={28}
                disabled={query.isFetching}
                onClick={() => void query.refetch()}
              >
                Refresh
              </Button>
            }
          >
            {query.isError ? (
              <p role="alert">
                Sessions could not be loaded.{' '}
                <button onClick={() => void query.refetch()}>Try again</button>
              </p>
            ) : (
              <DataTable
                label="Indexed sessions"
                columns={columns}
                rows={rows}
                getRowKey={(row) => row.id}
                rowHeight={40}
                loading={query.isPending}
                emptyMessage={
                  search || hosts || withPrs
                    ? 'No sessions match these filters.'
                    : 'No indexed sessions yet. Check indexing status in Settings.'
                }
                minWidth={860}
                expandedKeys={expanded}
                onExpandedChange={setExpanded}
                renderExpanded={(row) => <SessionDetails row={row} />}
                // The list takes the page's remaining height and scrolls on
                // its own, as the reference does: its header stays put above
                // the rows, and the page around it does not scroll too.
                stickyHeader
              />
            )}
            {query.hasNextPage && (
              <div className="xt-sessions-more">
                <Button disabled={query.isFetching} onClick={() => void query.fetchNextPage()}>
                  Load more sessions
                </Button>
              </div>
            )}
          </SectionCard>
        </>
      )}
    </section>
  );
}
