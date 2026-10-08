import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import { useInfiniteQuery, useQueries, useQueryClient } from '@tanstack/react-query';
import { Link, useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import type { SessionSort } from '../data/DataSource';
import { queryKeys } from '../data/query-client';
import type { SessionParentLink } from '../data/generated/SessionParentLink';
import type { SessionPrLink } from '../data/generated/SessionPrLink';
import type { SessionRow } from '../data/generated/SessionRow';
import { EvidenceDot } from '../kit/Badge';
import { FilterMenu } from '../kit/FilterMenu';
import { Toggle } from '../kit/Toggle';
import { DataTable, type Column } from '../kit/DataTable';
import { Search } from '../kit/Search';
import { Button } from '../kit/Button';
import { SectionCard } from '../kit/SectionCard';
import { MetricCell } from '../kit/MetricCell';
import { clock, isInstant } from '../kit/clock';
import { count, tokens as formatTokens } from '../kit/format';
import { hostName } from '../kit/hosts';
import {
  absentKey,
  laneRows,
  listedLanes,
  notListed,
  type ShownContext,
  sessionKey,
  type LaneParent,
  type LaneRow,
  type ListedSession,
} from './dashboard/lanes';
import { rangeDays } from './dashboard/present';
import { useSelectedRange } from './dashboard/range';
import { agentDuration } from './agent-duration';
import { handsOffTime } from './metric-format';
import { contextTitle, displayTitle, modelLabel, repoName, shortId } from './session-context';
import {
  formatHosts,
  parseHosts,
  parseSearch,
  parseSessionSort,
  parseWithPrs,
  SEARCH_MAX,
  sessionHosts,
  sessionHref,
  sessionParams,
} from './session-search';
import { RangeIssueFlag, SessionsIssueFlag, type IndexIssue } from './SessionsIssues';
import { SessionsSummary, SUMMARY_SCOPE, SUMMARY_SCOPE_SHORT } from './SessionsSummary';
import { GroupToggle, ParentMarker, parentName, verifiedParent } from './session-parent';
import { useSessionTitles, useVisibleIds, type TitleRow } from './session-titles';
import {
  isLiveSessionHost,
  useLiveSessionStatus,
  type LiveSessionStatus,
} from './live-session-status';
import { LiveSessionBadge, LiveSessionHost } from './LiveSessionBadge';
import { LaneCost, shownCost } from './dashboard/ActivityLanes';
import { RulePopover } from '../kit/RulePopover';
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

const hostOptions = sessionHosts.map((id) => ({ id, label: hostName(id) }));

/**
 * A session's title: its host's own, read from the original source for this
 * view and never stored, else a title the index saved, else none.
 */
type HostTitles = (id: string) => string | null;
const titleOf = (row: SessionRow, hostTitle: HostTitles) =>
  displayTitle(hostTitle(row.id) ?? row.title, row.automated_review);
/**
 * How this view names a verified parent, wherever it does: the row that
 * stands for it, its disclosure and every sub-session's marker in both lists.
 */
type ParentNames = (parent: SessionParentLink) => string;
/**
 * The version a parent is read at when only a group row names it: nothing
 * about its own source is known here, so it is read once for this view, and
 * again only when a row of its own, with a version, takes its place.
 */
const NAMED_PARENT = 'named parent';

/**
 * A loaded row as the Dashboard's grouping reads it: keyed by its exact host
 * and identity, so a parent on another host is never this row.
 */
interface ListedRow extends ListedSession {
  row: SessionRow;
}
type TableRow = LaneRow<SessionParentLink, ListedRow> & {
  contextParent?: SessionParentLink;
  parentLoading?: boolean;
};
const listedRow = (row: SessionRow): ListedRow => ({
  key: sessionKey(row.host, row.id),
  sessionId: row.id,
  host: row.host,
  row,
});
const rowParent = (listed: ListedRow) => verifiedParent(listed.row);

/**
 * The group a drawn row stands for: its own session, or the parent an absent
 * row names. The same group keeps this key when its parent's own row arrives
 * on a later page, so it stays open or closed as it was.
 */
const groupOf = (row: LaneRow<SessionParentLink, ListedSession>) =>
  row.kind === 'absent' ? sessionKey(row.parent.host, row.parent.session_id) : row.key;
/** Open groups as the grouping reads them: each under either of its row keys. */
const openKeys = (open: ReadonlySet<string>) =>
  new Set([...open].flatMap((key) => [key, absentKey(key)]));
const toggled = (open: ReadonlySet<string>, key: string) => {
  const next = new Set(open);
  if (!next.delete(key)) next.add(key);
  return next;
};

/** What a context read says about one session: its host, bit, check and parent. */
interface ChildContext extends ShownContext {
  parent?: SessionParentLink | null;
}
/**
 * A session left out, by exact host and identity, as the Dashboard decides it
 * ([`notListed`]): still being checked, or a known sub-session whose creator is
 * not verified. Nothing known about a session leaves it out too.
 */
const unresolvedIn =
  (find: (id: string, host: string) => ChildContext | undefined) =>
  ({ session_id, host }: LaneParent) => {
    const found = find(session_id, host);
    return notListed(
      found,
      host,
      verifiedParent({ id: session_id, parent: found?.parent }) !== null,
    );
  };

/** Native start, as every instant in the app is written: day and time; the year in full. */
const startedText = (ms: number) => clock(ms, { date: 'day' });
const recordedText = (ms: number) =>
  isInstant(ms) ? clock(ms, { date: 'year' }) : 'an unreadable time';

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

/** A cell that belongs to a read session; an unavailable parent's is blank. */
const onSession =
  (render: (row: SessionRow) => ReactNode) =>
  (row: TableRow): ReactNode =>
    row.kind === 'session' ? render(row.lane.row) : null;

/**
 * A direct parent's row while its own data is loading or unavailable. The
 * saved name, link and child disclosure still work without inventing cells.
 */
function AbsentParent({
  row,
  name,
  address,
  onToggle,
}: {
  row: Extract<TableRow, { kind: 'absent' }>;
  name: string;
  address: URLSearchParams;
  onToggle: (group: string) => void;
}) {
  return (
    <div className="xt-session-name" data-group="absent">
      <span className="xt-session-heading xt-session-title-line">
        <GroupToggle
          row={row}
          name={name}
          noun="loaded sub-session"
          onToggle={() => onToggle(groupOf(row))}
        />
        <Link
          className="xt-session-open"
          to={sessionHref(row.parent.session_id, address)}
          aria-label={`Open parent session ${name}, ${row.parent.session_id}`}
          title={`${name} · ${row.parent.session_id}`}
        >
          {name}
        </Link>
      </span>
      <span className="xt-session-meta">
        {row.parentLoading ? 'Loading main session…' : 'Main session unavailable'}
      </span>
    </div>
  );
}

/**
 * The columns, built with the list's own address so every row can open its
 * session and come back to this list exactly as it is. The address is a
 * parameter rather than a module-level read because a column definition is
 * data: it has no hooks of its own and must not acquire any; the host titles
 * read for this view, and its parent names, are passed in the same way.
 */
const buildColumns = (
  address: URLSearchParams,
  hostTitle: HostTitles,
  nameParent: ParentNames,
  compaction: CompactionLookup,
  observe: ReturnType<typeof useVisibleIds>['observe'],
  liveStatus: (id: string) => LiveSessionStatus | undefined,
  onToggle: (group: string) => void,
): Column<TableRow>[] => [
  {
    key: 'host',
    header: <span className="sr-only">Host</span>,
    width: '20px',
    render: (row) => (
      <LiveSessionHost
        host={row.kind === 'session' ? row.lane.host : row.parent.host}
        status={row.kind === 'session' ? liveStatus(row.lane.sessionId) : undefined}
      />
    ),
  },
  {
    // The host's own title when its source has one, else a title the index
    // saved, else the session's own identity; the repository, branch and
    // model sit under it. No title is derived from what the session said.
    key: 'session',
    header: 'session · repo · branch',
    width: 'minmax(180px, 1fr)',
    render: (drawn) => {
      if (drawn.kind === 'absent')
        return (
          <AbsentParent
            row={drawn}
            name={nameParent(drawn.parent)}
            address={address}
            onToggle={onToggle}
          />
        );
      const { row } = drawn.lane;
      const title =
        titleOf(row, hostTitle) ?? (drawn.contextParent ? parentName(drawn.contextParent) : null);
      const parent = verifiedParent(row);
      const meta = (
        <span className="xt-session-meta" title={contextTitle(row.repo, row.branch)}>
          {drawn.contextParent && (
            <span title="This parent's own row is outside these filters or on a page that has not been loaded.">
              Parent shown for context ·{' '}
            </span>
          )}
          {repoName(row.repo) ?? 'Unknown repository'}
          {row.branch && (
            <span className="xt-session-branch">
              {' '}
              <span aria-hidden="true">⑂</span> {row.branch}
            </span>
          )}
          <span aria-hidden="true"> · </span>
          <span>{modelLabel(row.model, row.other_models)}</span>
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
        <div
          className="xt-session-name"
          ref={observe}
          data-visible-id={row.id}
          style={drawn.depth > 0 ? ({ '--depth': drawn.depth } as CSSProperties) : undefined}
        >
          <span className="xt-session-heading xt-session-title-line">
            {drawn.children > 0 && (
              <GroupToggle
                row={drawn}
                name={title ?? `Session ${shortId(row.id)}`}
                noun="loaded sub-session"
                onToggle={() => onToggle(groupOf(drawn))}
              />
            )}
            <Link
              className="xt-session-open"
              to={sessionHref(row.id, address)}
              aria-label={`Open session ${title ? `${title}, ` : ''}${row.id}`}
              title={title ? `${title} · ${row.id}` : row.id}
            >
              {title ?? `Session ${shortId(row.id)}`}
            </Link>
            {isLiveSessionHost(row.host) && liveStatus(row.id) !== 'running' && (
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
                name={nameParent(parent)}
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
    render: onSession((row) =>
      verifiedParent(row) ? null : (
        <CompactionBadge outcome={compaction(row.id)} reading={compaction.reading(row.id)} />
      ),
    ),
  },
  {
    key: 'prs',
    header: (
      <span title="Linked pull requests. The dot shows how the link was found; linked does not mean merged.">
        PRs
      </span>
    ),
    width: '150px',
    render: onSession((row) => <PrLinks row={row} />),
  },
  {
    key: 'started',
    // The start the host recorded. Claude Code records none, so a Claude
    // session shows its own first event instead, the one sessions per day
    // counts it on (never a message a fork copied from its parent). Other
    // hosts with no recorded start stay unknown.
    header: (
      <span title="When the session began: the start its host recorded, or for Claude Code its earliest message.">
        started
      </span>
    ),
    width: '104px',
    render: onSession((row) =>
      row.started_at_ms === null ? (
        <MetricCell
          value={null}
          reason="No start is known for this session; first recorded work is in its details"
        />
      ) : (
        <time
          className="xt-session-mono"
          dateTime={new Date(row.started_at_ms).toISOString()}
          title={recordedText(row.started_at_ms)}
        >
          {startedText(row.started_at_ms)}
        </time>
      ),
    ),
  },
  {
    key: 'human',
    header: <MetricHeader label="msgs" name="Human messages" ruleId="M-02" />,
    width: '52px',
    align: 'right',
    render: onSession((row) => (
      <MetricCell
        value={indexed(row)?.human_messages}
        format={count}
        align="right"
        reason={indexed(row) ? 'Human classification is unmeasured (M-02)' : NOT_INDEXED}
      />
    )),
  },
  {
    key: 'tools',
    // The Dashboard's tool-call count cites the same rule.
    header: <MetricHeader label="tools" name="Tool calls" ruleId="M-17" />,
    width: '56px',
    align: 'right',
    render: onSession((row) => (
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
    )),
  },
  {
    key: 'agent',
    header: <MetricHeader label="agent" name="Agent time" ruleId="M-05" />,
    width: '64px',
    align: 'right',
    render: onSession((row) => <AgentTime row={row} />),
  },
  {
    key: 'hands-off',
    header: <MetricHeader label="hands-off" name="Hands-off median minutes" ruleId="M-09" />,
    width: '68px',
    align: 'right',
    render: onSession((row) => <HandsOff row={row} />),
  },
  {
    key: 'output',
    header: <MetricHeader label="output" name="Output tokens" ruleId="M-04" />,
    width: '64px',
    align: 'right',
    render: onSession((row) => (
      <MetricCell
        value={indexed(row)?.tokens.counters.output_tokens}
        format={formatTokens}
        align="right"
        reason={
          indexed(row) ? 'Selected responses do not all state output tokens (M-04)' : NOT_INDEXED
        }
      />
    )),
  },
  {
    key: 'cost',
    header: (
      <RulePopover
        ruleId="M-04"
        context="Whole-session cost at public API prices, independent of the selected range. Each row shows its own session only; sub-sessions have their own cost. + means part is unpriced."
      >
        <button
          type="button"
          className="xt-table-metric-header"
          aria-label="Whole-session API-equivalent cost, definition"
        >
          cost
        </button>
      </RulePopover>
    ),
    width: '80px',
    align: 'right',
    render: onSession((row) => (
      <LaneCost shown={shownCost([{ cost: row.cost ?? null }])} size={11} />
    )),
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
            <time dateTime={row.first_ts}>{recordedText(Date.parse(row.first_ts))}</time>
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
            ? `median ${handsOffTime(handsOff.median_min)} over ${handsOff.n} ${
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

export function SessionsPage() {
  const sectionRef = useRef<HTMLElement>(null);
  const anchorRef = useRef<{ key: string; offset: number; scrollTop: number } | null>(null);
  const listIdentityRef = useRef('');
  const { source } = useData();
  const queryClient = useQueryClient();
  const index = useNativeIndexStatus();
  // The filters live in the address, so a link can open this list already
  // filtered, and so the browser's own back and forward — and a session's own
  // Back link — restore what was filtered rather than an empty list. Each is
  // validated here: an unsupported host is dropped, and a longer search than
  // the store accepts is bounded, so a hand-edited address degrades to a
  // working list instead of a failed query.
  const [params, setParams] = useSearchParams();
  const supportedList = source.kind !== 'preview' && !params.has('pr');
  const search = parseSearch(params.get(sessionParams.search));
  const hosts = parseHosts(params.get(sessionParams.host));
  const withPrs = parseWithPrs(params.get(sessionParams.withPrs));
  const sort = parseSessionSort(params.get(sessionParams.sort));
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
  const listIdentity = JSON.stringify([days, search, hostKey, withPrs, sort]);
  const keyForSort = (order: SessionSort) => [
    ...queryKeys.sessions(days),
    search,
    hostKey,
    withPrs,
    order,
  ];
  const query = useInfiniteQuery({
    // Every filter is part of the identity, so a change starts again from the
    // first page rather than appending pages of a different list.
    queryKey: keyForSort(sort),
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) =>
      source.sessionsList(
        { search, hosts: hostKey ? hostKey.split(',') : null, withPrs, sort },
        pageParam,
        days,
      ),
    getNextPageParam: (page) => page.next ?? undefined,
    enabled: supportedList,
  });
  const pages = query.data?.pages;
  // Every loaded row, exactly as the pages returned them: the loaded count,
  // the cursors and each row's own measurements are these, whatever is drawn.
  const rows = useMemo(() => pages?.flatMap((page) => page.rows) ?? [], [pages]);
  const loadedById = useMemo(() => new Map(rows.map((row) => [row.id, row])), [rows]);
  // The loaded rows as the Dashboard groups its lanes: a verified sub-session
  // under the session that created it, collapsed, or under one row naming a
  // parent that is not loaded. A known sub-session whose creator is not
  // verified is left out, with every loaded session under it. Whether a
  // parent that is not loaded is one is read from the context its child's
  // page carried; no further page or ancestor is read for it.
  const listed = useMemo(() => {
    const context = new Map(
      pages
        ?.flatMap((page) => page.referenced_parents ?? [])
        .map((entry) => [sessionKey(entry.host, entry.session_id), entry]),
    );
    return listedLanes(
      rows.map(listedRow),
      rowParent,
      // A loaded row of another host with the same identity is not it.
      unresolvedIn((id, host) => {
        const loadedRow = loadedById.get(id);
        return loadedRow?.host === host ? loadedRow : context.get(sessionKey(host, id));
      }),
    );
  }, [pages, rows, loadedById]);
  // Open groups by the session they stand for, kept while pages load and
  // refresh, so a parent arriving on a later page keeps its group as it was.
  const [openGroups, setOpenGroups] = useState<ReadonlySet<string>>(() => new Set());
  const toggleGroup = useCallback(
    (group: string) => setOpenGroups((previous) => toggled(previous, group)),
    [],
  );
  const groupedTable = useMemo(
    () => laneRows(listed, rowParent, openKeys(openGroups)).rows,
    [listed, openGroups],
  );
  // Read only the direct parents that already have a visible group. Keep
  // these reads out of grouping: fetching a parent never adds its ancestors,
  // alters a page's membership or advances its cursor. The detail page uses
  // the same window cache, and repeated parent identities share one read.
  const absentParents = useMemo(
    () => [
      ...new Map(
        groupedTable.flatMap((row) =>
          row.kind === 'absent' ? [[row.parent.session_id, row.parent] as const] : [],
        ),
      ).values(),
    ],
    [groupedTable],
  );
  const parentQueries = useQueries({
    queries: absentParents.map((parent) => ({
      queryKey: queryKeys.session(days, parent.session_id),
      queryFn: () => source.sessionRow(parent.session_id, days),
      enabled: supportedList && !query.isError,
    })),
  });
  const parentResults = new Map(
    absentParents.map((parent, i) => [parent.session_id, parentQueries[i]]),
  );
  const table: TableRow[] = groupedTable.map((drawn) => {
    if (drawn.kind !== 'absent') return drawn;
    const result = parentResults.get(drawn.parent.session_id);
    const own = result?.isError ? null : result?.data;
    if (
      !own ||
      own.id !== drawn.parent.session_id ||
      own.host !== drawn.parent.host ||
      notListed(own, drawn.parent.host, verifiedParent(own) !== null)
    )
      return { ...drawn, parentLoading: result?.isPending && result.isFetching };
    const lane = listedRow(own);
    return {
      kind: 'session',
      key: lane.key,
      lane,
      depth: drawn.depth,
      children: drawn.children,
      expanded: drawn.expanded,
      members: drawn.members,
      contextParent: drawn.parent,
    };
  });
  /** Session rows drawn now, including read parents outside the loaded pages. */
  const shownRows = useMemo(
    () => table.flatMap((row) => (row.kind === 'session' ? [row.lane.row] : [])),
    [table],
  );
  // A clock or data event replays the pages already loaded. Remember the first
  // visible row before that read, and keep following it if the user scrolls
  // while the read is pending. Restore its latest pixel offset when it lands.
  // A row is remembered by the group it stands for, so a parent that arrives
  // in place of the row naming it is still found. A new range or filter is a
  // new list and starts at its own first page.
  useLayoutEffect(() => {
    if (listIdentityRef.current !== listIdentity) {
      listIdentityRef.current = listIdentity;
      anchorRef.current = null;
      return;
    }
    if (!query.isFetching || table.length === 0) return;
    const scroll = sectionRef.current?.querySelector<HTMLElement>('.xt-table-scroll');
    if (!scroll) return;
    const capture = () => {
      const blocks = [...scroll.querySelectorAll<HTMLElement>('.xt-table-block')];
      const top =
        scroll.getBoundingClientRect().top +
        (scroll.querySelector<HTMLElement>('.xt-table-head')?.getBoundingClientRect().height ?? 0);
      const index = blocks.findIndex((block) => block.getBoundingClientRect().bottom > top);
      if (index < 0 || !table[index]) return;
      anchorRef.current = {
        key: groupOf(table[index]),
        offset: blocks[index].getBoundingClientRect().top - top,
        scrollTop: scroll.scrollTop,
      };
    };
    capture();
    scroll.addEventListener('scroll', capture, { passive: true });
    return () => scroll.removeEventListener('scroll', capture);
  }, [listIdentity, query.isFetching, table]);
  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor || query.isFetching || listIdentityRef.current !== listIdentity) return;
    const scroll = sectionRef.current?.querySelector<HTMLElement>('.xt-table-scroll');
    if (scroll) {
      const index = table.findIndex((row) => groupOf(row) === anchor.key);
      const block = scroll.querySelectorAll<HTMLElement>('.xt-table-block')[index];
      const top =
        scroll.getBoundingClientRect().top +
        (scroll.querySelector<HTMLElement>('.xt-table-head')?.getBoundingClientRect().height ?? 0);
      scroll.scrollTop = block
        ? scroll.scrollTop + block.getBoundingClientRect().top - top - anchor.offset
        : Math.min(anchor.scrollTop, Math.max(0, scroll.scrollHeight - scroll.clientHeight));
    }
    anchorRef.current = null;
  }, [listIdentity, query.isFetching, query.dataUpdatedAt, table]);
  const { observe, visible } = useVisibleIds();
  const liveStatus = useLiveSessionStatus(
    JSON.stringify(['sessions', listIdentity, params.toString()]),
    supportedList
      ? [
          ...(!query.isError ? shownRows : [])
            .filter((row) => isLiveSessionHost(row.host) && visible.has(row.id))
            .map((row) => row.id),
        ]
      : [],
  );
  // Read names only for drawn table rows and the parents a group names.
  const { titleRows, titleHosts } = useMemo(() => {
    const result: TitleRow[] = [];
    const named = new Map<string, string>();
    if (!supportedList || query.isPending) return { titleRows: result, titleHosts: named };
    const name = (id: string, host: string, version: TitleRow['version']) => {
      if (named.has(id)) return;
      named.set(id, host);
      result.push({ id, version });
    };
    for (const row of shownRows) name(row.id, row.host, row.record_count);
    for (const row of table)
      if (row.kind === 'absent' && !row.parentLoading)
        name(row.parent.session_id, row.parent.host, NAMED_PARENT);
    return { titleRows: result, titleHosts: named };
  }, [supportedList, query.isPending, shownRows, table]);
  const titleVersions = useMemo(
    () => new Map(titleRows.map((row) => [row.id, row.version])),
    [titleRows],
  );
  const readTitle = useSessionTitles(
    JSON.stringify(['sessions', days, search, hostKey, withPrs, sort]),
    titleRows,
  );
  const hostTitle = useCallback<HostTitles>(
    (id) => {
      const version = titleVersions.get(id);
      return version === undefined ? null : readTitle(id, version);
    },
    [readTitle, titleVersions],
  );
  // A parent is named by the host title read for it in this view only when
  // that read was for its exact host and identity. A parent loaded in this
  // list is then named as its own row reads with that title; any other by the
  // title alone; then as the report sent it ([`parentName`]).
  const nameParent = useCallback<ParentNames>(
    (parent) => {
      const own =
        shownRows.find((row) => row.id === parent.session_id && row.host === parent.host) ??
        loadedById.get(parent.session_id);
      const exact: HostTitles = (id) => (titleHosts.get(id) === parent.host ? hostTitle(id) : null);
      return parentName(
        parent,
        own?.host === parent.host ? titleOf(own, exact) : exact(parent.session_id),
      );
    },
    [loadedById, shownRows, hostTitle, titleHosts],
  );
  // A sub-session uses its parent's count; read only drawn main sessions.
  const compaction = useSessionCompactions(
    JSON.stringify(['sessions', days, search, hostKey, withPrs, sort]),
    shownRows.filter((row) => !verifiedParent(row)).map((row) => row.id),
  );
  // Each cell shares this view's title, parent-name and live-state lookups.
  const columns = useMemo(
    () => buildColumns(params, hostTitle, nameParent, compaction, observe, liveStatus, toggleGroup),
    [params, hostTitle, nameParent, compaction, observe, liveStatus, toggleGroup],
  );
  // Whether a host's scan is a problem is the app's one answer, shared with
  // the sidebar, Welcome and Settings; a host waiting on the running scan is not.
  const incomplete = index.data?.needs_attention;
  const scanning = index.data?.phase.phase === 'scanning';
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
            // Counts cover indexed matches; only messages and time are windowed.
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
          <SessionsSummary
            summary={query.isError && !query.isFetchNextPageError ? null : pages?.[0]?.summary}
            describedBy={scopeId}
            pending={query.isPending}
            failed={
              (query.isError && !query.isFetchNextPageError) || (!!pages && !pages[0]?.summary)
            }
            refreshing={query.isFetching}
            onRetry={() => void query.refetch()}
          />
          <SectionCard
            title="All sessions"
            right={
              <>
                <label className="xt-sessions-sort">
                  <span className="sr-only">Sort sessions</span>
                  <select
                    aria-label="Sort sessions"
                    value={sort}
                    onChange={(event) => {
                      const next = parseSessionSort(event.target.value);
                      if (next === sort) return;
                      // Deliberate sorting starts at page one. Returning from
                      // session details does not run this handler and keeps its pages.
                      queryClient.removeQueries({ queryKey: keyForSort(next), exact: true });
                      setFilter(sessionParams.sort, next === 'started' ? null : next);
                    }}
                  >
                    <option value="started">Started</option>
                    <option value="recently_active">Recently active</option>
                  </select>
                </label>
                <Button
                  variant="outline"
                  height={28}
                  disabled={query.isFetching || parentQueries.some((parent) => parent.isFetching)}
                  onClick={() =>
                    void Promise.all([
                      query.refetch(),
                      ...parentQueries.map((parent) => parent.refetch()),
                    ])
                  }
                >
                  Refresh
                </Button>
              </>
            }
          >
            {query.isError && pages && (
              <p role="alert">
                {query.isFetchNextPageError
                  ? 'More sessions could not be loaded. Showing previously loaded sessions. '
                  : 'Sessions could not be refreshed. Showing previously loaded sessions. '}
                <button
                  onClick={() =>
                    void (query.isFetchNextPageError ? query.fetchNextPage() : query.refetch())
                  }
                >
                  Try again
                </button>
              </p>
            )}
            {query.isError && !pages ? (
              <p role="alert">
                Sessions could not be loaded.{' '}
                <button onClick={() => void query.refetch()}>Try again</button>
              </p>
            ) : (
              <DataTable
                label="Indexed sessions"
                columns={columns}
                rows={table}
                // A loaded session keeps its own identity as its key, so its
                // details stay open where it moves; a row naming a parent
                // that is not loaded has a key no session can have.
                getRowKey={(row) => (row.kind === 'session' ? row.lane.sessionId : row.key)}
                rowHeight={40}
                loading={query.isPending}
                emptyMessage={
                  rows.length > 0
                    ? 'No session to list: every loaded session is a sub-session whose main session is unknown.'
                    : search || hosts || withPrs
                      ? 'No sessions match these filters.'
                      : 'No indexed sessions yet. Check indexing status in Settings.'
                }
                minWidth={860}
                expandedKeys={expanded}
                onExpandedChange={setExpanded}
                canExpand={(row) => row.kind === 'session'}
                renderExpanded={(row) =>
                  row.kind === 'session' ? <SessionDetails row={row.lane.row} /> : null
                }
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
