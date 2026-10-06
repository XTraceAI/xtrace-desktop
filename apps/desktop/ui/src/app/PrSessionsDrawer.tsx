import { useId, useMemo, type RefObject } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { useData } from '../data/DataProvider';
import type { PullRequestSessionsRequest } from '../data/DataSource';
import { queryKeys } from '../data/query-client';
import type { PrAnalyticsPage } from '../data/generated/PrAnalyticsPage';
import type { SessionRow } from '../data/generated/SessionRow';
import { EvidenceDot } from '../kit/Badge';
import { Button } from '../kit/Button';
import { DataTable, type Column } from '../kit/DataTable';
import { count, tokens as formatTokens } from '../kit/format';
import { HostGlyph } from '../kit/HostGlyph';
import { MetricCell } from '../kit/MetricCell';
import { Modal, ModalClose } from '../kit/Modal';
import { agentDuration } from './agent-duration';
import { clockTime, plural, recordedTime, windowLabel, zoneLabel } from './dashboard/present';
import { evidenceWords, prIdentity, rowTokensReason } from './pr-analytics';
import {
  AgentTime,
  HandsOff,
  indexed,
  MetricHeader,
  NOT_INDEXED,
  sessionName,
} from './session-cells';
import { contextTitle, repoName } from './session-context';

/** The pull request a drilldown was opened for; everything else is the report's. */
export interface PrTarget {
  repository: string;
  number: number;
}

/**
 * The report's own request for one pull request's linked sessions: its
 * canonical identity, and the displayed report's preset, confidence mode and
 * `window.end_ms`. It is derived from the report on every render, never
 * stored, so a newly read report (a new anchor, range or mode) replaces the
 * whole drilldown at once under a new key, and nothing of the old one is
 * painted under the new heading. Absent while no report row names the pull
 * request.
 */
export function drilldownRequest(
  target: PrTarget | null,
  page: PrAnalyticsPage | undefined,
): PullRequestSessionsRequest | null {
  if (!target || !page) return null;
  const listed = page.report.rows.some(
    (row) => row.repository === target.repository && row.number === target.number,
  );
  return listed
    ? {
        repository: target.repository,
        number: target.number,
        confirmedOnly: page.report.confirmed_only,
        windowDays: page.window.days,
        windowEndMs: page.window.end_ms,
      }
    : null;
}

/** The link that made this session a member, with its own evidence. */
const selectedLink = (row: SessionRow, target: PrTarget) =>
  row.pr_links.find(
    (link) => link.repository === target.repository && link.number === target.number,
  );

const columns = (target: PrTarget): Column<SessionRow>[] => [
  {
    key: 'host',
    header: <span className="sr-only">Host</span>,
    width: '20px',
    render: (row) => <HostGlyph host={row.host} size={18} />,
  },
  {
    // Not a link: the session page measures its own current range, not this
    // pull request's pinned window, so its numbers would not be these.
    key: 'session',
    header: 'session · repo · branch',
    width: 'minmax(170px, 1fr)',
    render: (row) => (
      <div className="xt-pr-name">
        <span className="xt-pr-title" title={row.title ? `${row.title} · ${row.id}` : row.id}>
          {sessionName(row)}
        </span>
        <span className="xt-pr-meta" title={contextTitle(row.repo, row.branch)}>
          {repoName(row.repo) ?? 'Unknown repository'}
          {row.branch && (
            <span className="xt-pr-branch">
              {' '}
              <span aria-hidden="true">⑂</span> {row.branch}
            </span>
          )}
        </span>
      </div>
    ),
  },
  {
    key: 'evidence',
    header: (
      <span title="How this session links to the PR, and how many other PRs it links.">link</span>
    ),
    width: '92px',
    render: (row) => {
      const link = selectedLink(row, target);
      const others = row.pr_links.length - (link ? 1 : 0);
      return (
        <div className="xt-pr-name">
          {link ? (
            <span className="xt-pr-evidence">
              <EvidenceDot evidence={link.confidence} />
              {evidenceWords[link.confidence]}
            </span>
          ) : (
            <MetricCell value={null} reason="This page no longer carries the member's link" />
          )}
          <span
            className="xt-pr-meta"
            title={row.pr_links
              .map((item) => `${prIdentity(item)} · ${evidenceWords[item.confidence]}`)
              .join('\n')}
          >
            {others > 0 ? `+${others} other ${others === 1 ? 'PR' : 'PRs'}` : 'this PR only'}
          </span>
        </div>
      );
    },
  },
  {
    key: 'activity',
    header: <span title="Work events this session recorded inside the window.">events</span>,
    width: '58px',
    align: 'right',
    render: (row) => {
      const metrics = indexed(row);
      if (!metrics) return <MetricCell value={null} align="right" reason={NOT_INDEXED} />;
      return metrics.events === 0 ? (
        <span
          className="xt-pr-meta"
          title="No work in this window. Linked sessions are listed even when idle."
        >
          idle
        </span>
      ) : (
        <MetricCell value={metrics.events} format={count} align="right" />
      );
    },
  },
  {
    key: 'human',
    header: <MetricHeader label="msgs" name="Human messages" ruleId="M-02" />,
    width: '48px',
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
    key: 'tokens',
    header: <MetricHeader label="tokens" name="Tokens, four counters" ruleId="M-04" />,
    width: '60px',
    align: 'right',
    render: (row) => {
      const metrics = indexed(row);
      const total = metrics?.tokens.counters.total_tokens;
      return (
        <span title={typeof total === 'number' ? `${count(total)} tokens` : undefined}>
          <MetricCell
            value={total}
            format={formatTokens}
            align="right"
            reason={
              !metrics
                ? NOT_INDEXED
                : metrics.tokens.selected_responses === 0
                  ? 'No selected usage in this window'
                  : 'A selected usage counter is missing (M-04)'
            }
          />
        </span>
      );
    },
  },
  {
    key: 'agent',
    header: <MetricHeader label="agent" name="Agent time" ruleId="M-05" />,
    width: '64px',
    align: 'right',
    render: (row) => <AgentTime row={row} />,
  },
  {
    key: 'hands-off',
    header: <MetricHeader label="hands-off" name="Hands-off median minutes" ruleId="M-09" />,
    width: '64px',
    align: 'right',
    render: (row) => <HandsOff row={row} />,
  },
];

/**
 * One pull request's linked sessions, over exactly the displayed report's
 * window: its members after the report's confidence filter, 50 to a page in
 * the Sessions list's order, each once however many pull requests it links.
 * A member idle in the window is listed. Each row is that session's own
 * measurement; nothing here adds them, and the report row's values beside
 * them are the report's. Closing returns focus to the control that opened it
 * and leaves the page exactly as it was.
 */
export function PrSessionsDrawer({
  target,
  page,
  reportState,
  onRetryReport,
  onClose,
  returnFocus,
}: {
  target: PrTarget | null;
  /**
   * The report the page shows now, and nothing else: while its read is
   * running or has failed the page shows none, and neither does this, so the
   * drilldown never carries totals or members of a report that is no longer
   * displayed. The pull request stays selected, and the recovered report's
   * own key brings its members back.
   */
  page: PrAnalyticsPage | undefined;
  reportState: 'pending' | 'failed' | 'ready';
  /** Reads the report again, so a failure is recoverable without closing. */
  onRetryReport: () => void;
  onClose: () => void;
  returnFocus: RefObject<HTMLElement | null>;
}) {
  const { source } = useData();
  const titleId = useId();
  const request = drilldownRequest(target, page);
  const row =
    target && page
      ? page.report.rows.find(
          (item) => item.repository === target.repository && item.number === target.number,
        )
      : undefined;
  const query = useInfiniteQuery({
    queryKey: request
      ? queryKeys.prSessions(request)
      : (['sessions', 'pr-linked', 'none'] as const),
    queryFn: ({ pageParam }) => source.pullRequestSessions(request!, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next,
    enabled: request !== null && source.kind !== 'preview',
  });
  // Every page must measure the requested window; one that does not is
  // refused rather than shown beside the report's.
  const pages = query.data?.pages;
  const pinned =
    !request || !pages || pages.every((item) => item.window.end_ms === request.windowEndMs);
  const rows = useMemo(
    () => (pinned ? (pages ?? []).flatMap((item) => item.rows) : []),
    [pages, pinned],
  );
  const window = page?.window;
  const tableColumns = useMemo(() => (target ? columns(target) : []), [target]);

  return (
    <Modal
      open={target !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      returnFocusRef={returnFocus}
      aria-labelledby={titleId}
      className="xt-pr-drawer"
      data-testid="pr-sessions-drawer"
    >
      {/* Close comes first, so opening by keyboard lands on it. */}
      <header className="xt-pr-drawer-header">
        <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
          Close
        </ModalClose>
        <div className="xt-pr-drawer-title">
          <h2 id={titleId}>Linked sessions</h2>
          {target && (
            // Without a shown report there is no row to take a title from,
            // so the identity stands alone rather than claiming none is cached.
            <span className="xt-pr-meta" title={row?.url}>
              {row ? `${row.title ?? 'Title not cached'} · ` : ''}
              {prIdentity(target)}
            </span>
          )}
        </div>
      </header>
      <p className="xt-pr-drawer-banner" role="note">
        Linked-session effort; sessions may contribute to other PRs. Each row is one session’s own
        measurement over this report’s window, and the rows are not added up here.
      </p>
      {!target ? null : !page ? (
        reportState === 'pending' ? (
          <p role="status" className="xt-prs-status">
            Reading the report for this range…
          </p>
        ) : (
          <p role="alert" className="xt-prs-status">
            The report could not be read, so the linked sessions of {prIdentity(target)} are not
            shown. Nothing here is carried over from an earlier report.{' '}
            <Button variant="outline" height={28} onClick={onRetryReport}>
              Retry
            </Button>
          </p>
        )
      ) : !row || !window ? (
        <p role="status" className="xt-prs-status">
          {prIdentity(target)} is not in the report now shown for this range and mode: its cached
          facts or links changed since it was opened. No members are listed for it.
        </p>
      ) : (
        <>
          <dl className="xt-pr-drawer-facts">
            <div>
              <dt>Window</dt>
              <dd title={`${windowLabel(window)} · ${zoneLabel(window)}`}>
                {windowLabel(window)} ·{' '}
                {page.report.confirmed_only ? 'exact and commit links' : 'all links'}
              </dd>
            </div>
            <div>
              <dt>Merged</dt>
              <dd title={recordedTime(row.merged_at_ms, window)}>
                {clockTime(row.merged_at_ms, window)}
              </dd>
            </div>
            <div>
              <dt>Report row</dt>
              <dd>
                {plural(row.linked_sessions, 'linked session')} · {count(row.active_sessions)}{' '}
                active ·{' '}
                <span
                  title={
                    row.tokens.counters.total_tokens === null
                      ? rowTokensReason(row)
                      : `${count(row.tokens.counters.total_tokens)} tokens`
                  }
                >
                  {row.tokens.counters.total_tokens === null
                    ? 'tokens unknown'
                    : `${formatTokens(row.tokens.counters.total_tokens)} tokens`}
                </span>{' '}
                ·{' '}
                {row.agent_ms === null
                  ? 'agent time unknown'
                  : `${agentDuration(row.agent_ms).visible} agent`}
                {row.unmeasured_links > 0 &&
                  ` · ${plural(row.unmeasured_links, 'link')} to a non-user session, not measured`}
              </dd>
            </div>
          </dl>
          {query.isError || !pinned ? (
            <p role="alert" className="xt-prs-status">
              {pinned
                ? 'These linked sessions could not be read.'
                : 'A page measured a different window, so it is not shown.'}{' '}
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
              label={`Linked sessions of ${prIdentity(target)}`}
              columns={tableColumns}
              rows={rows}
              getRowKey={(item) => item.id}
              rowHeight={40}
              loading={query.isPending}
              emptyMessage="No linked session is listed for this pull request now."
              minWidth={600}
              stickyHeader
            />
          )}
          <footer className="xt-pr-drawer-footer">
            <span className="xt-pr-meta" role="status">
              {query.isSuccess && pinned
                ? `${count(rows.length)} of ${plural(row.linked_sessions, 'linked session')} shown · Sessions list order`
                : ''}
            </span>
            {query.hasNextPage && pinned && (
              <Button
                variant="outline"
                height={28}
                disabled={query.isFetchingNextPage}
                onClick={() => void query.fetchNextPage()}
              >
                {query.isFetchingNextPage ? 'Reading…' : 'Show 50 more'}
              </Button>
            )}
          </footer>
        </>
      )}
    </Modal>
  );
}
