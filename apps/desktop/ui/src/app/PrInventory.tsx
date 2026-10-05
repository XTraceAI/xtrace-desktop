import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router';
import { useData } from '../data/DataProvider';
import { queryKeys } from '../data/query-client';
import type { PrRow } from '../data/generated/PrRow';
import type { PrStateReport } from '../data/generated/PrStateReport';
import { Button } from '../kit/Button';
import type { ControlTone } from '../kit/control-tone';
import { DataTable, type Column } from '../kit/DataTable';
import { count } from '../kit/format';
import { MetricCell, Unmeasured } from '../kit/MetricCell';
import { Search } from '../kit/Search';
import { SectionCard } from '../kit/SectionCard';
import { refreshErrorText } from './dashboard/pr-effort';
import { plural } from './dashboard/present';
import '../styles/prs.css';

/**
 * A cached inventory, and nothing more: every stored pull request a session
 * still links, with the facts the last manual refresh left in storage. It is
 * the PRs page's second view, independent of the merged-PR report beside it:
 * lifetime link counts of every confidence, never the selected range or the
 * confidence mode, and never joined to the report's rows. No value here is
 * calculated: each cell is a field of `PrRow` as read, or says that the field
 * is not cached. Null is never drawn as zero, open or unmerged.
 */

/**
 * Day, year and 24-hour time in this Mac's own zone, as the Sessions list
 * states a recorded time. Cached facts have no age limit, so the year is shown
 * rather than left to a tooltip. There is no report window on this page, and
 * none is made up to borrow a zone from.
 */
const timeFormat: Intl.DateTimeFormatOptions = {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
};
/** A stored instant outside what a Date can hold is stated, never thrown on. */
const timeText = (ms: number) =>
  Number.isNaN(new Date(ms).getTime())
    ? 'an unreadable time'
    : new Date(ms).toLocaleString(undefined, timeFormat);

const NOT_CACHED = 'no successful refresh has stored it';

const identity = (row: PrRow) => `${row.pull_request.repository}#${row.pull_request.number}`;

function PullRequest({ row }: { row: PrRow }) {
  return (
    <div className="xt-pr-name">
      {row.title === null ? (
        <span className="xt-pr-title" data-missing title={`Title not cached: ${NOT_CACHED}`}>
          Title not cached
        </span>
      ) : (
        <span className="xt-pr-title" title={row.title}>
          {row.title}
        </span>
      )}
      {/* Not a link: nothing on this page opens the network. The canonical
          address is stated in the tooltip instead, with the whole branch name. */}
      <span
        className="xt-pr-meta"
        title={
          row.head_ref_name === null
            ? row.pull_request.url
            : `${row.pull_request.url} · branch ${row.head_ref_name}`
        }
      >
        {identity(row)}
        {row.head_ref_name !== null && (
          <span className="xt-pr-branch">
            {' '}
            <span aria-hidden="true">⑂</span> {row.head_ref_name}
          </span>
        )}
      </span>
    </div>
  );
}

const stateTone: Record<PrStateReport, ControlTone> = {
  open: 'success',
  closed: 'meta',
  merged: 'accent',
};

/** The cached state as stored; merged is never inferred from a merge time. */
function State({ row }: { row: PrRow }) {
  if (row.state === null)
    return (
      <Unmeasured
        reason={`State not cached: ${NOT_CACHED}, so it is not known to be open, closed or merged`}
      />
    );
  const mergedAt = row.merged_at === null ? Number.NaN : Date.parse(row.merged_at);
  return (
    <div className="xt-pr-state">
      <span className="xt-pr-state-word xt-control-tone" data-tone={stateTone[row.state]}>
        <span className="xt-state-dot" aria-hidden="true" />
        {row.state}
      </span>
      {row.state === 'merged' && (
        <span className="xt-pr-meta">
          {row.merged_at === null ? (
            'merge time not cached'
          ) : Number.isNaN(mergedAt) ? (
            // Stored, but not an instant this can read: shown as stored.
            row.merged_at
          ) : (
            <time dateTime={new Date(mergedAt).toISOString()}>{timeText(mergedAt)}</time>
          )}
        </span>
      )}
    </div>
  );
}

/**
 * Cached additions and deletions, exact and never rounded, on two compact
 * lines so a lockfile-sized change reads whole in the column's width. A cached
 * zero is a zero; a null is not. The whole value is also the cell's tooltip,
 * so nothing a line could ever clip is out of reach.
 */
function Size({ row }: { row: PrRow }) {
  if (row.additions === null && row.deletions === null)
    return <MetricCell value={null} align="right" reason={`Size not cached: ${NOT_CACHED}`} />;
  const said = (value: number | null, sign: string, name: string) =>
    value === null ? `${name} not cached` : `${sign}${count(value)} ${name}`;
  const part = (value: number | null, sign: string, name: string) =>
    value === null ? (
      <span title={`${name} not cached`}>
        {sign}
        <span aria-hidden="true">—</span>
        <span className="sr-only"> {name.toLowerCase()} not cached</span>
      </span>
    ) : (
      <span>
        {sign}
        {count(value)}
        <span className="sr-only"> {name.toLowerCase()}</span>
      </span>
    );
  return (
    <span
      className="xt-pr-size"
      title={`${said(row.additions, '+', 'additions')} · ${said(row.deletions, '−', 'deletions')}`}
    >
      <span data-kind="additions">{part(row.additions, '+', 'Additions')}</span>{' '}
      <span data-kind="deletions">{part(row.deletions, '−', 'Deletions')}</span>
    </span>
  );
}

/**
 * What storage holds about refreshing this row, in the words the Dashboard's
 * refresh dialog uses. "refreshed" is when, never how current: storage applies
 * no age policy and neither does this, so it is not drawn as healthy.
 */
function cacheStatus(row: PrRow): { label: string; tone: ControlTone; detail: string | null } {
  const status = row.status;
  const at = (ms: number | null) => (ms === null ? 'an unknown time' : timeText(ms));
  switch (status.status) {
    case 'never_attempted':
      return { label: 'never refreshed', tone: 'meta', detail: null };
    case 'refreshed':
      return { label: 'refreshed', tone: 'info', detail: at(row.refreshed_at_ms) };
    case 'failed_never_refreshed':
      return {
        label: 'failed · never refreshed',
        tone: 'warning',
        detail: `${refreshErrorText[status.error]} · tried ${at(row.last_attempted_at_ms)}`,
      };
    case 'failed_after_refresh':
      // The earlier facts stay in the row beside this, exactly as stored.
      return {
        label: 'stale · last refresh failed',
        tone: 'warning',
        detail: `${refreshErrorText[status.error]} · facts from ${at(row.refreshed_at_ms)}`,
      };
  }
}

function Cache({ row }: { row: PrRow }) {
  const { label, tone, detail } = cacheStatus(row);
  return (
    <div className="xt-pr-cache" title={detail === null ? label : `${label} · ${detail}`}>
      <span className="xt-pr-cache-word xt-control-tone" data-tone={tone}>
        {label}
      </span>
      {detail !== null && <span className="xt-pr-meta">{detail}</span>}
    </div>
  );
}

const columns: readonly Column<PrRow>[] = [
  {
    key: 'pull-request',
    header: 'pull request · repo · branch',
    width: 'minmax(180px, 1.4fr)',
    render: (row) => <PullRequest row={row} />,
  },
  {
    key: 'state',
    header: <span title="The state and merge time the last successful refresh stored.">state</span>,
    width: '132px',
    render: (row) => <State row={row} />,
  },
  {
    key: 'size',
    header: <span title="Cached additions and deletions.">size</span>,
    width: '92px',
    align: 'right',
    render: (row) => <Size row={row} />,
  },
  {
    key: 'sessions',
    header: (
      <span title="Sessions that still link this pull request: every retained link, whatever its evidence, over all time. Not the selected range.">
        sessions
      </span>
    ),
    width: '54px',
    align: 'right',
    render: (row) => <MetricCell value={row.linked_sessions} format={count} align="right" />,
  },
  {
    key: 'cache',
    header: <span title="What storage holds about refreshing these facts.">cache status</span>,
    // Wide enough at the native minimum width for a failure code and a dated
    // fact line to read whole; the title column gives way first.
    width: 'minmax(268px, 1fr)',
    render: (row) => <Cache row={row} />,
  },
];

/** A case-insensitive substring of the title, the repository or the number. */
const matches = (row: PrRow, needle: string) =>
  `${identity(row)} ${row.title ?? ''}`.toLowerCase().includes(needle);

export function PrInventory() {
  const { source } = useData();
  // The same stored list, under the same key, that the Dashboard's refresh
  // dialog reads: a committed refresh, a committed import, turn, backfill or
  // settled index change, and a reconnect re-read it, and nothing else does.
  // No interval, no focus read, no refresh of its own.
  const query = useQuery({
    queryKey: queryKeys.pullRequests,
    queryFn: () => source.pullRequests(),
    enabled: source.kind !== 'preview',
  });
  // Narrows the rows already read; it never asks storage for anything.
  const [filter, setFilter] = useState('');
  const needle = filter.trim().toLowerCase();
  const all = query.data?.rows;
  const rows = useMemo(
    () => (all ?? []).filter((row) => needle === '' || matches(row, needle)),
    [all, needle],
  );
  const total = all?.length ?? 0;
  const population =
    needle === ''
      ? `${plural(total, 'pull request')} indexed`
      : `${rows.length.toLocaleString('en-US')} of ${total.toLocaleString('en-US')} shown`;

  return (
    <>
      <div className="xt-prs-toolbar">
        {/* First in the tab order, drawn at the row's end. */}
        <Search
          label="Filter pull requests"
          placeholder="Filter by title, repo or number…"
          value={filter}
          onValueChange={setFilter}
        />
        {/* The limit stays in view; what it means is one disclosure away. */}
        <div className="xt-prs-context">
          <p className="xt-prs-note" data-testid="prs-scope">
            Cached inventory · every linked PR, all time · not the merged-PR report
          </p>
          <details className="xt-prs-counted" data-testid="prs-counted">
            <summary>What is listed and counted</summary>
            <p>
              Listed is every stored pull request an indexed session still links, whatever its state
              or merge date: the selected range does not apply. Sessions counts those links over all
              time, whatever their evidence (exact, commit or inferred), and a link says a session
              referenced the pull request, not who wrote it. No effort is shown here; the merged-PR
              report is the other view of this page.
            </p>
            <p>
              Title, state, merge time, size and branch are what the last successful refresh stored.
              Nothing here contacts GitHub: facts change only through Refresh PR facts on the{' '}
              <Link to="/dashboard">Dashboard</Link>. The list is read from storage when this view
              opens, again after such a refresh commits, after an import, turn, backfill or index
              change commits new sessions or links, and after live updates reconnect.
            </p>
          </details>
        </div>
      </div>
      {source.kind === 'preview' ? (
        <p>Open the desktop app to see its indexed pull requests.</p>
      ) : (
        <SectionCard
          title="Linked pull requests"
          meta={`${query.isSuccess ? `${population} · ` : ''}cached facts · lifetime link counts · by repo, then number`}
        >
          {query.isError ? (
            <p role="alert" className="xt-prs-status">
              Indexed pull requests could not be read.{' '}
              {/* A local read of storage again; it refreshes nothing from GitHub. */}
              <Button
                variant="outline"
                height={28}
                disabled={query.isFetching}
                onClick={() => void query.refetch()}
              >
                Retry
              </Button>
            </p>
          ) : (
            <DataTable
              label="Linked pull requests"
              columns={columns}
              rows={rows}
              getRowKey={(row) => String(row.pull_request.id)}
              rowHeight={40}
              loading={query.isPending}
              emptyMessage={
                total === 0
                  ? 'No session links a pull request yet.'
                  : 'No pull request matches this filter.'
              }
              minWidth={770}
              // The rows take the page's remaining height and scroll on their
              // own under a fixed header, as the Sessions list does.
              stickyHeader
            />
          )}
        </SectionCard>
      )}
    </>
  );
}
