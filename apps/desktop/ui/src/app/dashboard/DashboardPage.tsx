import { useState, type ReactNode } from 'react';
import { Link } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricTile } from '../../data/generated/MetricTile';
import { Button } from '../../kit/Button';
import { count } from '../../kit/format';
import { LoadingMark } from '../../kit/LoadingMark';
import { MetricCell } from '../../kit/MetricCell';
import { RulePopover } from '../../kit/RulePopover';
import { SectionCard } from '../../kit/SectionCard';
import { StatTile } from '../../kit/StatTile';
import type { TimeRange } from '../../kit/TopBar';
import { continuous } from '../metric-format';
import { sessionParams } from '../session-search';
import { ActivityLanes, laneDefinition } from './ActivityLanes';
import { DashCard } from './DashCard';
import { DashboardDetailsProvider } from './DashboardDetailsProvider';
import { CoverageDetail, UsageMeasurements, useDetail } from './DashboardDetails';
import { DefinitionInfo } from './DefinitionInfo';
import { EffortByType, EffortMethod, EffortMetricToggle } from './EffortByType';
import { EnvironmentPanel } from './EnvironmentPanel';
import { mergedAside, prFreshnessText, type EffortMetric } from './pr-effort';
import { PrRefresh, PrRefreshStatus } from './PrRefresh';
import {
  excludedSurfaceText,
  favoriteUnknownText,
  plural,
  ruleId,
  tileDelta,
  tileTip,
} from './present';
import { useDashboardReport, useSelectedRange } from './range';
import '../../styles/dashboard.css';

const TITLE = 'What your agents did';
/** The Concurrency tile's primary number, which carries no visible unit. */
const CONCURRENCY_SHOWN = 'Displayed value is mean concurrency; max is the peak overlap.';

export function DashboardPage() {
  const { source } = useData();
  const range = useSelectedRange();
  const report = useDashboardReport(range);
  // Kept while another range loads, like the open measurement detail.
  const [effortMetric, setEffortMetric] = useState<EffortMetric>('agent');
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
 * heading and the four tiles keep their height, and the two rows under them —
 * effort beside environment, then Sessions — take the rest, each showing at
 * least three rows of its own and more of them in a taller window. The
 * report's agent/human hours and rule fires are not drawn here. Nothing here is a page to scroll: every further row, day, table and
 * explanation opens over the page from the card it belongs to — tokens and
 * cost from Effort's Method, coverage and untimed history from Sessions — or
 * on the full Sessions view. The index's state is the sidebar's and Settings'.
 */
function DashboardReport({
  report,
  range,
  effortMetric,
  onEffortMetric,
}: {
  report: DashboardMetrics;
  range: TimeRange;
  effortMetric: EffortMetric;
  onEffortMetric: (metric: EffortMetric) => void;
}) {
  const { tiles, favorite } = report;
  const prTile = report.pr_effort.current.tile;
  const favoriteModel = favorite.current.model;
  const method = useDetail('method');
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
      <div className="xt-dash-tiles">
        <StatTile
          label="Agent h/day"
          icon="clock"
          ruleId={ruleId(tiles.agent_hours_per_day.rule_id, 'M-05')}
          value={tiles.agent_hours_per_day.value}
          format={continuous}
          unit="h"
          reason={tiles.agent_hours_per_day.reason ?? undefined}
          delta={tileDelta(tiles.agent_hours_per_day)}
          tip={tileTip(tiles.agent_hours_per_day)}
        />
        <StatTile
          label="Concurrency"
          icon="lanes"
          ruleId={ruleId(tiles.concurrency_mean.rule_id, 'M-06')}
          value={tiles.concurrency_mean.value}
          format={continuous}
          reason={tiles.concurrency_mean.reason ?? undefined}
          delta={tileDelta(tiles.concurrency_mean)}
          aside={
            tiles.concurrency_max.value === null
              ? undefined
              : `max ${count(tiles.concurrency_max.value)}`
          }
          // The tile shows no unit, so its definition says which of the two
          // the primary number is.
          tip={[CONCURRENCY_SHOWN, tileTip(tiles.concurrency_mean)].filter(Boolean).join(' ')}
        />
        <StatTile
          label="Merged PRs"
          icon="merge"
          ruleId={ruleId(tiles.merged_prs.rule_id, 'M-19')}
          value={tiles.merged_prs.value}
          format={count}
          reason={tiles.merged_prs.reason ?? undefined}
          delta={tileDelta(tiles.merged_prs)}
          aside={mergedAside(prTile)}
          tip={[
            tileTip(tiles.merged_prs),
            prTile.unresolved_type > 0
              ? `${plural(prTile.unresolved_type, 'merged pull request')} without a cached type.`
              : '',
            prFreshnessText(prTile.freshness, report.window),
          ]
            .filter(Boolean)
            .join(' ')}
        />
        <StatTile
          label="Hands-off median"
          icon="shield"
          ruleId={ruleId(tiles.hands_off_median.rule_id, 'M-09')}
          value={tiles.hands_off_median.value}
          format={continuous}
          unit="min"
          reason={tiles.hands_off_median.reason ?? undefined}
          delta={tileDelta(tiles.hands_off_median)}
          aside={
            tiles.hands_off_p90.value === null
              ? undefined
              : `p90 ${continuous(tiles.hands_off_p90.value)}`
          }
          tip={[
            'How long your agents run before they need you.',
            tileTip(
              tiles.hands_off_median,
              report.hands_off_excluded_surfaces.length > 0
                ? `Excluded for timestamp health: ${report.hands_off_excluded_surfaces.map(excludedSurfaceText).join('; ')}.`
                : 'No surface is excluded for timestamp health.',
            ),
          ].join(' ')}
        />
      </div>

      <div className="xt-dash-row xt-dash-main-row">
        <DashCard
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
              <PrRefresh window={report.window} />
            </>
          }
        >
          <EffortByType report={report} metric={effortMetric} />
          <PrRefreshStatus freshness={prTile.freshness} window={report.window} />
        </DashCard>
        <EnvironmentPanel range={range} />
      </div>

      <SectionCard
        title="Sessions"
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

/** What the Effort card itself shows, read after the M-19 rule in its definition. */
const EFFORT_CONTEXT =
  'This card shows the total for every session in the range, one bar per day, and does not split it by work type. Each session’s hours count toward the model it used most.';

function SummaryCount({ tile, one }: { tile: MetricTile; one: string }) {
  return tile.value === null ? (
    <span>
      <MetricCell value={null} reason={tile.reason ?? undefined} /> {one}s
    </span>
  ) : (
    <span>{plural(tile.value, one)}</span>
  );
}
