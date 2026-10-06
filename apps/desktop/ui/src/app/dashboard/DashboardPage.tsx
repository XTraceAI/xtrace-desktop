import { useQuery } from '@tanstack/react-query';
import { useEffect, useId, useState, type ReactNode } from 'react';
import { Link } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricTile } from '../../data/generated/MetricTile';
import { queryKeys } from '../../data/query-client';
import { Button } from '../../kit/Button';
import { LoadingMark } from '../../kit/LoadingMark';
import { MetricCell } from '../../kit/MetricCell';
import { RulePopover } from '../../kit/RulePopover';
import { SectionCard } from '../../kit/SectionCard';
import type { TimeRange } from '../../kit/TopBar';
import { sessionParams } from '../session-search';
import { ActivityLanes, laneDefinition, laneSubtitle } from './ActivityLanes';
import { DashCard } from './DashCard';
import { DashboardDetailsProvider } from './DashboardDetailsProvider';
import { CoverageDetail, UsageMeasurements, useDetail } from './DashboardDetails';
import { DefinitionInfo } from './DefinitionInfo';
import { EffortByType, EffortMethod, EffortMetricToggle } from './EffortByType';
import { OverviewCard } from './OverviewCard';
import { type AutoCheck, type EffortMetric } from './pr-effort';
import { PrRefresh } from './PrRefresh';
import { SplitHandle } from './SplitHandle';
import { favoriteUnknownText, plainReason, plural } from './present';
import { useDashboardReport, useSelectedRange } from './range';
import '../../styles/dashboard.css';

const TITLE = 'What your agents did';
export function DashboardPage() {
  const { source } = useData();
  const range = useSelectedRange();
  const report = useDashboardReport(range);
  // Kept while another range loads, like the open measurement detail.
  const [effortMetric, setEffortMetric] = useState<EffortMetric>('agent');
  const autoCheck = source.prAutoCheck;
  const auto = useQuery({
    queryKey: queryKeys.prAutoCheck,
    queryFn: () =>
      autoCheck ? autoCheck.status() : Promise.reject(new Error('No automatic check here.')),
    enabled: autoCheck !== undefined,
  });
  // Showing the Dashboard asks the app to check pull requests on GitHub; it
  // decides for itself whether anything is due, and never makes this wait.
  useEffect(() => {
    autoCheck?.request().catch(() => {});
  }, [autoCheck]);
  let body: ReactNode;
  if (source.kind === 'preview')
    body = <p className="xt-dash-status">Open the desktop app to read local Dashboard metrics.</p>;
  else if (report.isPending)
    body = (
      <div className="xt-dash-loading">
        <LoadingMark />
        <p role="status" className="xt-dash-status">
          Reading Dashboard metrics for the last {range}…
        </p>
      </div>
    );
  else if (report.isError)
    body = (
      <p role="alert" className="xt-dash-status">
        Dashboard metrics could not be loaded.{' '}
        <Button
          variant="outline"
          height={28}
          disabled={report.isFetching}
          onClick={() => void report.refetch()}
        >
          Retry
        </Button>
      </p>
    );
  else
    body = (
      <DashboardReport
        report={report.data}
        range={range}
        effortMetric={effortMetric}
        onEffortMetric={setEffortMetric}
        // A source that never checks on its own says so with null.
        autoCheck={autoCheck === undefined ? null : auto.data}
      />
    );
  return (
    <section className="xt-dashboard" aria-busy={report.isFetching || undefined}>
      <h1>{TITLE}</h1>
      <DashboardDetailsProvider>{body}</DashboardDetailsProvider>
    </section>
  );
}

/**
 * The primary composition, sized to the window rather than to its data: the
 * heading and the summary line keep their height, and the two rows under
 * them — Effort beside Overview, then Sessions — take the rest. By default
 * Sessions keeps room for eight rows however few it lists and the effort row
 * takes what is left; each row still shows at least three rows of its own,
 * and more of them in a taller window. The boundary between the rows and the
 * one between Effort and Overview can be dragged or moved from the
 * keyboard (`SplitHandle`); the chosen proportions are kept across restarts,
 * and a double-click or Enter on a boundary puts its default back. The
 * report's agent/human hours and rule fires are not drawn here. Nothing here
 * is a page to scroll: every further row, day, table and explanation opens
 * over the page from the card it belongs to — tokens and cost from Effort's
 * Details, coverage and untimed history from Sessions — or on the full
 * Sessions view. The index's state is the sidebar's and Settings'.
 */
function DashboardReport({
  report,
  range,
  effortMetric,
  onEffortMetric,
  autoCheck,
}: {
  report: DashboardMetrics;
  range: TimeRange;
  effortMetric: EffortMetric;
  onEffortMetric: (metric: EffortMetric) => void;
  autoCheck: AutoCheck;
}) {
  const { tiles, favorite } = report;
  const prTile = report.pr_effort.current.tile;
  const favoriteModel = favorite.current.model;
  const method = useDetail('method');
  const mainRowId = useId();
  const effortId = useId();
  return (
    <>
      <p className="xt-dash-summary" data-testid="dashboard-summary">
        <SummaryCount tile={tiles.sessions} one="session" />
        <SummaryCount tile={tiles.human_messages} one="human message" />
        <SummaryCount tile={tiles.tool_calls} one="tool call" />
        <span>
          favorite{' '}
          <RulePopover
            ruleId="M-10"
            context={
              favorite.previous.model
                ? `Previous period: ${favorite.previous.model}.`
                : 'Previous period: no favorite model.'
            }
          >
            <button type="button" className="xt-dash-inline-metric">
              <MetricCell
                value={favoriteModel}
                reason={
                  favorite.current.unknown_reason
                    ? favoriteUnknownText[favorite.current.unknown_reason]
                    : 'No favorite model'
                }
              />
            </button>
          </RulePopover>
        </span>
      </p>
      {tiles.sessions.value === 0 && (
        <p role="status" className="xt-dash-status">
          No agent activity was recorded in this range. Measured zeros are shown as 0.
        </p>
      )}
      <div id={mainRowId} className="xt-dash-row xt-dash-main-row">
        <DashCard
          id={effortId}
          title="Effort"
          rule="M-19"
          context={EFFORT_CONTEXT}
          className="xt-effort-card"
          layered
          actions={
            <>
              <EffortMetricToggle value={effortMetric} onChange={onEffortMetric} />
              <EffortMethod report={report} metric={effortMetric} {...method}>
                <UsageMeasurements report={report} />
              </EffortMethod>
              <PrRefresh window={report.window} tile={prTile} auto={autoCheck} />
            </>
          }
        >
          <EffortByType report={report} metric={effortMetric} />
        </DashCard>
        {/* Must sit directly between Effort and Overview: it resizes its siblings. */}
        <SplitHandle
          axis="columns"
          label="Resize Effort and Overview"
          names={['Effort', 'Overview']}
          controls={effortId}
        />
        <OverviewCard report={report} range={range} autoCheck={autoCheck} />
      </div>
      {/* Must sit directly between the effort row and Sessions: it resizes its siblings. */}
      <SplitHandle
        axis="rows"
        label="Resize Effort and Overview against Sessions"
        names={['Effort and Overview', 'Sessions']}
        controls={mainRowId}
      />

      <SectionCard
        title="Sessions"
        meta={laneSubtitle(report)}
        right={
          <span className="xt-dash-card-actions">
            <CoverageDetail report={report} />
            <DefinitionInfo ruleId="M-05" name="Sessions" context={laneDefinition(report)} />
            <Link className="xt-dash-link" to={`/sessions?${sessionParams.range}=${range}`}>
              View sessions
            </Link>
          </span>
        }
        headerHeight={36}
        padding="2px 0 2px"
      >
        <ActivityLanes report={report} range={range} />
      </SectionCard>
    </>
  );
}

/**
 * What the Effort card itself shows, read after the rule's summary: its bars
 * cover every session with work in the range, linked to a pull request or not.
 */
export const EFFORT_CONTEXT =
  'This card covers every session in the range, one bar per day, split by model, whether or not it is linked to a PR.';

function SummaryCount({ tile, one }: { tile: MetricTile; one: string }) {
  return tile.value === null ? (
    <span>
      <MetricCell value={null} reason={tile.reason ? plainReason(tile.reason) : undefined} /> {one}s
    </span>
  ) : (
    <span>{plural(tile.value, one)}</span>
  );
}
