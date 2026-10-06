import { useMemo, useRef, useState } from 'react';
import type { UseQueryResult } from '@tanstack/react-query';
import type { DashboardWindow } from '../data/generated/DashboardWindow';
import type { MetricPrAnalyticsRow } from '../data/generated/MetricPrAnalyticsRow';
import type { MetricPrMedian } from '../data/generated/MetricPrMedian';
import type { PrAnalyticsPage } from '../data/generated/PrAnalyticsPage';
import { EvidenceDot } from '../kit/Badge';
import { Button } from '../kit/Button';
import { DataTable, type Column } from '../kit/DataTable';
import { count, tokens as formatTokens } from '../kit/format';
import { MetricCell, Unmeasured } from '../kit/MetricCell';
import type { MetricIconName } from '../kit/metric-icons';
import type { RuleId } from '../kit/rules';
import { Search } from '../kit/Search';
import { SectionCard } from '../kit/SectionCard';
import { StatTile } from '../kit/StatTile';
import { Toggle } from '../kit/Toggle';
import { agentDuration } from './agent-duration';
import {
  clockTime,
  excludedNote,
  plural,
  recordedTime,
  windowLabel,
  zoneOf,
} from './dashboard/present';
import { continuous } from './metric-format';
import {
  emptyReportText,
  evidenceMix,
  evidenceWords,
  freshnessText,
  medianAside,
  medianGap,
  medianSample,
  emptyTypesText,
  REPORT_FAILED,
  REPORT_PENDING,
  medianView,
  mixedEvidence,
  prIdentity,
  rowHandsOff,
  rowHumanReason,
  rowTokensReason,
  rowTokensTitle,
  tokenMedianView,
  workTypeLabel,
  type MedianView,
} from './pr-analytics';
import { PrSessionsDrawer, type PrTarget } from './PrSessionsDrawer';
import { MetricHeader } from './session-cells';

const agentText = (ms: number) => agentDuration(ms).visible;
/** The merge day alone, in the report's zone; the time sits beneath it. */
const mergedDay = (ms: number, window: DashboardWindow) =>
  new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: 'numeric',
    timeZone: zoneOf(window),
  }).format(ms);
const minutesText = (value: number) => `${continuous(value)}m`;

/** A tile's value and label from one report median; never a recomputed one. */
function MedianTile({
  label,
  icon,
  ruleId,
  view,
  median,
  format,
  note,
  noSample,
  unavailable,
}: {
  label: string;
  icon: MetricIconName;
  ruleId: RuleId;
  view: MedianView | undefined;
  median: MetricPrMedian | undefined;
  format: (value: number) => string;
  /** A short live note added after the rule's summary, such as a left-out app. */
  note?: string;
  noSample?: string;
  /** Why there is no view: the report is still being read, or its read failed. */
  unavailable: string;
}) {
  const measuredOnly = view?.kind === 'measured';
  return (
    <div className="xt-prs-tile" data-median={view?.kind ?? 'pending'}>
      <StatTile
        label={label}
        icon={icon}
        ruleId={ruleId}
        value={view && view.kind !== 'none' ? view.value : null}
        format={format}
        reason={view ? (view.kind === 'none' ? view.reason : undefined) : unavailable}
        aside={
          median
            ? measuredOnly
              ? `measured PRs · ${median.measured_prs} of ${median.eligible_prs}`
              : medianAside(median)
            : undefined
        }
        tip={[median && medianGap(median, noSample), note].filter(Boolean).join(' ') || undefined}
      />
    </div>
  );
}

/**
 * A value from one per-type median, with that metric's own sample: a mark
 * when it covers measured PRs only, its n when not every PR of the type gave
 * a value, and the whole sample in words for a screen reader and the title.
 */
function TypeValue({
  median,
  format,
  noSample,
  testId,
}: {
  median: MetricPrMedian;
  format: (value: number) => string;
  noSample?: string;
  testId: string;
}) {
  const view = medianView(median, noSample);
  const sample = medianSample(median, noSample);
  if (view.kind === 'none')
    return (
      <span data-testid={testId}>
        <Unmeasured reason={view.reason} />
      </span>
    );
  return (
    <span
      className="xt-prs-type-value"
      data-testid={testId}
      title={view.kind === 'measured' ? `Median over measured PRs only · ${sample}` : sample}
    >
      {format(view.value)}
      {view.kind === 'measured' && <span aria-hidden="true">*</span>}
      {median.measured_prs !== median.eligible_prs && (
        <span className="xt-pr-meta" aria-hidden="true">
          {' '}
          {median.measured_prs}/{median.eligible_prs}
        </span>
      )}
      <span className="sr-only">
        {view.kind === 'measured' ? ', measured PRs only' : ''}, {sample}
      </span>
    </span>
  );
}

const matches = (row: MetricPrAnalyticsRow, needle: string) =>
  `${prIdentity(row)} ${row.title ?? ''} ${workTypeLabel(row.work_type)}`
    .toLowerCase()
    .includes(needle);

const rowKey = (row: MetricPrAnalyticsRow) => prIdentity(row);

/** Names one control that opens a drilldown, so focus can find it again. */
const openerName = (row: MetricPrAnalyticsRow, control: 'sessions' | 'tokens') =>
  `${prIdentity(row)}|${control}`;

function buildColumns(
  window: DashboardWindow,
  open: (row: MetricPrAnalyticsRow, control: 'sessions' | 'tokens') => void,
): Column<MetricPrAnalyticsRow>[] {
  return [
    {
      key: 'pull-request',
      header: 'pull request',
      width: 'minmax(112px, 1.6fr)',
      render: (row) => (
        <div className="xt-pr-name">
          {row.title === null ? (
            <span className="xt-pr-title" data-missing title="No refresh stored a title">
              Title not cached
            </span>
          ) : (
            <span className="xt-pr-title" title={row.title}>
              {row.title}
            </span>
          )}
          {/* Not a link: nothing on this page opens the network. */}
          <span className="xt-pr-meta" title={row.url}>
            {prIdentity(row)}
          </span>
        </div>
      ),
    },
    {
      key: 'type',
      header: <span title="Work type, read from the PR title, then its branch name.">type</span>,
      width: '48px',
      render: (row) =>
        row.work_type === null ? (
          <span className="xt-pr-type" data-missing title="A fact the type needs is not cached">
            unresolved
          </span>
        ) : (
          <span className="xt-pr-type" title={row.work_type}>
            {row.work_type}
          </span>
        ),
    },
    {
      key: 'merged',
      header: <span title="The cached merge instant, in the report's zone.">merged</span>,
      width: '46px',
      render: (row) => (
        <div className="xt-pr-name">
          <time
            className="xt-pr-mono"
            dateTime={new Date(row.merged_at_ms).toISOString()}
            title={`${recordedTime(row.merged_at_ms, window)} · ${freshnessText(row.freshness)}`}
          >
            {mergedDay(row.merged_at_ms, window)}
          </time>
          {row.freshness.state === 'failed_after_refresh' ? (
            <span className="xt-pr-stale" title={freshnessText(row.freshness)}>
              stale
            </span>
          ) : (
            <span className="xt-pr-meta">{clockTime(row.merged_at_ms, window, false)}</span>
          )}
        </div>
      ),
    },
    {
      key: 'sessions',
      header: (
        <MetricHeader label="sessions" name="Linked sessions, active in range" ruleId="M-11" />
      ),
      width: '52px',
      align: 'right',
      render: (row) => (
        <button
          type="button"
          tabIndex={0}
          className="xt-pr-open"
          data-opener={openerName(row, 'sessions')}
          aria-haspopup="dialog"
          aria-label={`${plural(row.linked_sessions, 'linked session')}, ${count(row.active_sessions)} active in range: open the linked sessions of ${prIdentity(row)}`}
          title={`${plural(row.linked_sessions, 'linked session')} · ${count(row.active_sessions)} with work in this range${row.unmeasured_links > 0 ? ` · ${plural(row.unmeasured_links, 'link')} to a non-user session, not measured` : ''}`}
          onClick={() => open(row, 'sessions')}
        >
          <span>{count(row.linked_sessions)}</span>
          <span className="xt-pr-meta">{count(row.active_sessions)} active</span>
        </button>
      ),
    },
    {
      key: 'human',
      header: <MetricHeader label="msgs" name="Human messages" ruleId="M-12" />,
      width: '34px',
      align: 'right',
      render: (row) => (
        <MetricCell
          value={row.human_messages}
          format={count}
          align="right"
          reason={rowHumanReason(row)}
        />
      ),
    },
    {
      key: 'tokens',
      header: <MetricHeader label="tokens" name="Tokens, four counters" ruleId="M-11" />,
      width: '48px',
      align: 'right',
      render: (row) => (
        <button
          type="button"
          tabIndex={0}
          className="xt-pr-open"
          data-opener={openerName(row, 'tokens')}
          aria-haspopup="dialog"
          aria-label={`${row.tokens.counters.total_tokens === null ? rowTokensReason(row) : `${count(row.tokens.counters.total_tokens)} tokens`}: open the linked sessions of ${prIdentity(row)}`}
          title={
            row.tokens.counters.total_tokens === null
              ? `${rowTokensReason(row)} · ${rowTokensTitle(row)}`
              : rowTokensTitle(row)
          }
          onClick={() => open(row, 'tokens')}
        >
          {row.tokens.counters.total_tokens === null ? (
            <span aria-hidden="true">—</span>
          ) : (
            formatTokens(row.tokens.counters.total_tokens)
          )}
        </button>
      ),
    },
    {
      key: 'agent',
      header: <MetricHeader label="agent" name="Agent time" ruleId="M-11a" />,
      width: '56px',
      align: 'right',
      render: (row) =>
        row.agent_ms === null ? (
          <MetricCell value={null} align="right" reason="No indexed session is linked" />
        ) : (
          <span title={`Exactly ${agentDuration(row.agent_ms).exact}; parallel sessions add`}>
            <MetricCell value={agentText(row.agent_ms)} align="right" />
          </span>
        ),
    },
    {
      key: 'hands-off',
      header: (
        <MetricHeader label="hands-off" name="Hands-off, pooled median minutes" ruleId="M-12a" />
      ),
      width: '60px',
      align: 'right',
      render: (row) => {
        const { value, text } = rowHandsOff(row);
        return (
          <div className="xt-pr-name xt-pr-right" title={text}>
            <MetricCell value={value} format={minutesText} align="right" reason={text} />
            <span className="xt-pr-meta">
              {row.hands_off.n === null ? '' : `n=${count(row.hands_off.n)}`}
              {row.hands_off.excluded_sessions > 0 && <span className="xt-pr-excluded"> excl</span>}
            </span>
            {value !== null && <span className="sr-only">{text}</span>}
          </div>
        );
      },
    },
    {
      key: 'evidence',
      header: (
        <span title="How the PR is linked: its strongest link, then each session's own.">
          evidence
        </span>
      ),
      width: '64px',
      render: (row) => (
        <div
          className="xt-pr-name"
          title={`Strongest ${evidenceWords[row.confidence]} · ${evidenceMix(row)}`}
        >
          <span className="xt-pr-evidence">
            <EvidenceDot evidence={row.confidence} />
            {evidenceWords[row.confidence]}
          </span>
          {/* The whole mix is the tooltip and is spoken; the line only says
              whether the sessions' own evidence differs from the strongest. */}
          <span className="xt-pr-meta" aria-hidden="true">
            {mixedEvidence(row) ? 'mixed' : `all ${count(row.linked_sessions)}`}
          </span>
          <span className="sr-only">, linked sessions: {evidenceMix(row)}</span>
        </div>
      ),
    },
  ];
}

/**
 * The merged-PR report: overlapping linked-session effort for each pull
 * request known to have merged in the selected window, per-PR medians and
 * per-type medians, all as the Rust report computed them. The table filter
 * narrows rows only; every median and count stays the whole report's.
 */
export function PrAnalytics({
  query,
  confirmedOnly,
  onConfirmedOnly,
  filter,
  onFilter,
  onInventory,
}: {
  query: UseQueryResult<PrAnalyticsPage>;
  confirmedOnly: boolean;
  onConfirmedOnly: (value: boolean) => void;
  filter: string;
  onFilter: (value: string) => void;
  onInventory: () => void;
}) {
  const page = query.data;
  // A failed read shows nothing it read before: every companion surface
  // follows the table's own error, and recovers with it on Retry.
  const shown = query.isError ? undefined : page;
  const report = shown?.report;
  const window = shown?.window;
  const unavailable = query.isError ? REPORT_FAILED : REPORT_PENDING;
  const [target, setTarget] = useState<PrTarget | null>(null);
  // Which control opened the drilldown, by name rather than by element: a
  // failed read replaces the table, so the button that opened it is gone by
  // the time a recovered report draws its own. Focus returns to the control
  // standing in its place, or nowhere rather than to a detached node.
  const opener = useRef<string | null>(null);
  const returnFocus = useMemo(
    () => ({
      get current() {
        return opener.current
          ? document.querySelector<HTMLElement>(`[data-opener="${opener.current}"]`)
          : null;
      },
    }),
    [],
  );
  const needle = filter.trim().toLowerCase();
  // Newest merge first; the report's own order reversed, nothing re-derived.
  const ordered = useMemo(() => [...(report?.rows ?? [])].reverse(), [report]);
  const rows = useMemo(
    () => ordered.filter((row) => needle === '' || matches(row, needle)),
    [ordered, needle],
  );
  const columns = useMemo(
    () =>
      window
        ? buildColumns(window, (row, control) => {
            opener.current = openerName(row, control);
            setTarget({ repository: row.repository, number: row.number });
          })
        : [],
    [window],
  );
  const summary = report?.summary;
  const excluded = report?.hands_off_excluded_surfaces ?? [];
  const eligibility = report?.eligibility;

  return (
    <>
      <div className="xt-prs-toolbar">
        {/* First in the tab order, drawn at the row's end. */}
        <Search
          label="Filter merged pull requests"
          placeholder="Filter rows by title, repo, number or type…"
          value={filter}
          onValueChange={onFilter}
        />
        <Toggle label="Confirmed only" checked={confirmedOnly} onChange={onConfirmedOnly} />
        <div className="xt-prs-context">
          <p className="xt-prs-note" data-testid="prs-scope">
            Merged {window ? windowLabel(window) : 'in the selected range'} ·{' '}
            {confirmedOnly ? 'exact and commit links only' : 'all links, inferred included'} ·
            cached facts, not live GitHub state
          </p>
          <details className="xt-prs-counted" data-testid="prs-counted">
            <summary>What is measured</summary>
            <p>
              Rows are the linked pull requests whose cached state is merged at an instant inside
              the selected window. Each row adds up the in-window work of every indexed session
              linked to it (after Confirmed only removes inferred links), whether or not the session
              was active in the window. A session linked to two pull requests counts in both rows,
              so rows overlap and are never added together; a link says a session referenced the
              pull request, not who wrote it, and nothing here is a cost.
            </p>
            <p>
              Tiles and the type panel are medians over pull requests; n counts the PRs that gave a
              median a value, beside all the merged PRs it covers. A median marked measured PRs
              leaves out rows whose value is unknown; token medians also wait for 90% token+model
              coverage over a fixed 14 days. A dash is unknown, never zero; hover it for why. Pull
              requests whose cached facts are unknown are not rows; the Cached inventory lists every
              linked pull request.
            </p>
          </details>
        </div>
      </div>
      <div className="xt-prs-tiles" aria-busy={query.isPending || undefined}>
        <MedianTile
          unavailable={unavailable}
          label="Tokens / PR"
          icon="token"
          ruleId="M-11"
          median={summary?.tokens.median}
          view={
            summary && report && window
              ? tokenMedianView(summary.tokens, report.token_gate, window)
              : undefined
          }
          format={formatTokens}
        />
        <MedianTile
          unavailable={unavailable}
          label="Human msgs / PR"
          icon="msg"
          ruleId="M-12"
          median={summary?.human_messages}
          view={summary && medianView(summary.human_messages)}
          format={continuous}
        />
        <MedianTile
          unavailable={unavailable}
          label="Agent time / PR"
          icon="clock"
          ruleId="M-11a"
          median={summary?.agent_ms}
          view={summary && medianView(summary.agent_ms)}
          format={agentText}
        />
        <MedianTile
          unavailable={unavailable}
          label="Hands-off / PR"
          icon="bolt"
          ruleId="M-12a"
          median={summary?.hands_off_min}
          view={summary && medianView(summary.hands_off_min, 'no stretch')}
          format={minutesText}
          noSample="no stretch"
          note={excludedNote(excluded)}
        />
      </div>
      <div className="xt-prs-main">
        <SectionCard
          title="Merged pull requests"
          meta={
            report
              ? `${rows.length === ordered.length ? plural(ordered.length, 'PR') : `${rows.length} of ${ordered.length} PRs shown · medians cover all`} · newest merge first`
              : undefined
          }
        >
          {query.isError ? (
            <p role="alert" className="xt-prs-status">
              The pull-request report could not be read.{' '}
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
              label="Merged pull requests"
              columns={columns}
              rows={rows}
              getRowKey={rowKey}
              rowHeight={40}
              loading={query.isPending}
              emptyMessage={
                ordered.length > 0
                  ? 'No merged pull request matches this filter.'
                  : report
                    ? emptyReportText(report, report.confirmed_only)
                    : ''
              }
              minWidth={600}
              stickyHeader
            />
          )}
        </SectionCard>
        <SectionCard title="By type" meta="per-PR medians">
          {report && window && report.by_type.length > 0 ? (
            <>
              <ul className="xt-prs-types" role="list" aria-label="Per-PR medians by work type">
                {report.by_type.map((group) => (
                  <li key={group.work_type ?? '\u0000'} className="xt-prs-type">
                    <span className="xt-prs-type-head">
                      <span
                        className="xt-pr-type"
                        data-missing={group.work_type === null || undefined}
                      >
                        {workTypeLabel(group.work_type)}
                      </span>
                      <span className="xt-pr-meta">{plural(group.summary.prs, 'PR')}</span>
                    </span>
                    <span className="xt-prs-type-values">
                      <span>
                        <span className="xt-pr-meta">msgs </span>
                        <TypeValue
                          median={group.summary.human_messages}
                          format={continuous}
                          testId="type-msgs"
                        />
                      </span>
                      <span>
                        <span className="xt-pr-meta">agent </span>
                        <TypeValue
                          median={group.summary.agent_ms}
                          format={agentText}
                          testId="type-agent"
                        />
                      </span>
                      <span>
                        <span className="xt-pr-meta">hands-off </span>
                        <TypeValue
                          median={group.summary.hands_off_min}
                          format={minutesText}
                          noSample="no stretch"
                          testId="type-hands-off"
                        />
                      </span>
                    </span>
                  </li>
                ))}
              </ul>
              <p className="xt-prs-type-note">
                * median over measured PRs only; m/n when not every PR of the type gave a value. A
                type's PRs can share sessions with other types.
              </p>
            </>
          ) : (
            <p className="xt-prs-type-note" data-testid="prs-types">
              {report
                ? emptyTypesText(report)
                : query.isPending
                  ? REPORT_PENDING
                  : `${REPORT_FAILED}.`}
            </p>
          )}
        </SectionCard>
      </div>
      {eligibility && (
        <p className="xt-prs-facts" data-testid="prs-eligibility">
          <span>
            {plural(eligibility.merged, 'linked PR')} merged in range · {count(eligibility.outside)}{' '}
            open, closed or merged outside · {count(eligibility.unknown_facts)} with unknown cached
            facts
            {eligibility.unresolved_type > 0 &&
              ` · ${count(eligibility.unresolved_type)} unresolved type`}
            {excluded.length > 0 &&
              ` · hands-off excludes ${excluded.map((item) => `${item.host} ${item.surface ?? 'unknown surface'}`).join(', ')}`}
          </span>{' '}
          <button type="button" tabIndex={0} className="xt-pr-link-button" onClick={onInventory}>
            Cached inventory
          </button>
        </p>
      )}
      <PrSessionsDrawer
        target={target}
        // The report the page shows, never the last one a failed read left.
        page={shown}
        reportState={query.isError ? 'failed' : query.isPending ? 'pending' : 'ready'}
        onRetryReport={() => void query.refetch()}
        onClose={() => setTarget(null)}
        returnFocus={returnFocus}
      />
    </>
  );
}
