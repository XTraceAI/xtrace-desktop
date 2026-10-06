import { useQuery } from '@tanstack/react-query';
import { useMemo, type ReactNode } from 'react';
import { useData } from '../../data/DataProvider';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { MetricTile } from '../../data/generated/MetricTile';
import { queryKeys } from '../../data/query-client';
import { count, delta as formatDelta, isMeasured } from '../../kit/format';
import { MetricCell } from '../../kit/MetricCell';
import { MetricIcon, type MetricIconName } from '../../kit/metric-icons';
import type { RuleId } from '../../kit/rules';
import type { TimeRange } from '../../kit/TopBar';
import { continuous } from '../metric-format';
import { DashCard } from './DashCard';
import { DayLine } from './DayLine';
import { DefinitionInfo } from './DefinitionInfo';
import {
  CONCURRENCY_DEFINITION,
  HANDS_OFF_DEFINITION,
  OVERVIEW_DEFINITION,
  type DayValue,
  concurrencyByDay,
  concurrencyTakeaway,
  handsOffByDay,
  handsOffTakeaway,
  hoursText,
  latestMerged,
  mergedTakeaway,
  leverageByDay,
  leverageTakeaway,
} from './overview';
import { MERGED_PRS_DEFINITION, mergedTileView, type AutoCheck } from './pr-effort';
import {
  excludedNote,
  plural,
  rangeDays,
  sentence,
  tileDelta,
  tileReason,
  tileTip,
} from './present';
import '../../styles/overview.css';

/** How many merged pull requests the tile lists, latest first. */
export const MERGED_LISTED = 4;

/**
 * Leverage, Concurrency, Merged PRs and Hands-off median for the selected
 * range, two by two, each with its change against the previous period, a
 * day-by-day line where a day can be measured on its own, and one computed
 * takeaway. Every number is the Rust report's.
 */
export function OverviewCard({
  report,
  range,
  autoCheck,
}: {
  report: DashboardMetrics;
  range: TimeRange;
  autoCheck?: AutoCheck;
}) {
  const { tiles } = report;
  const leverage = useMemo(() => leverageByDay(report), [report]);
  const concurrency = useMemo(() => concurrencyByDay(report), [report]);
  const handsOff = useMemo(() => handsOffByDay(report), [report]);
  const human = report.human_hours.current;
  return (
    <DashCard
      title="Overview"
      rule="M-05"
      definition={OVERVIEW_DEFINITION}
      className="xt-overview-card"
      layered
      actions={<span className="xt-overview-vs">vs. previous {rangeDays[range]} days</span>}
    >
      <div className="xt-overview-grid" data-testid="overview-grid">
        <OverviewTile
          label="Leverage"
          icon="bolt"
          rule="M-08"
          tile={tiles.leverage}
          format={continuous}
          unit="×"
          sub={
            // The two sides the value divides: the same whole local days.
            human.active_ms !== null
              ? `${hoursText(report.leverage.agent_ms)} agent h ÷ ${hoursText(human.active_ms)} your h`
              : undefined
          }
          takeaway={leverageTakeaway(leverage)}
        >
          <DayChart
            days={leverage}
            average={tiles.leverage.value}
            format={(value) => `${continuous(value)}×`}
            name="Leverage by day"
          />
        </OverviewTile>
        <OverviewTile
          label="Concurrency"
          icon="lanes"
          rule="M-06"
          definition={CONCURRENCY_DEFINITION}
          tile={tiles.concurrency_mean}
          format={continuous}
          sub={
            tiles.concurrency_max.value === null
              ? undefined
              : `max ${count(tiles.concurrency_max.value)}`
          }
          takeaway={concurrencyTakeaway(report)}
        >
          <DayChart
            days={concurrency}
            average={tiles.concurrency_mean.value}
            format={continuous}
            name="Average concurrency by day"
          />
        </OverviewTile>
        <MergedTile report={report} range={range} autoCheck={autoCheck} />
        <OverviewTile
          label="Hands-off median"
          icon="shield"
          rule="M-09"
          definition={HANDS_OFF_DEFINITION}
          context={excludedNote(report.hands_off_excluded_surfaces)}
          tile={tiles.hands_off_median}
          format={continuous}
          unit="min"
          sub={
            tiles.hands_off_p90.value === null
              ? undefined
              : `p90 ${continuous(tiles.hands_off_p90.value)} min`
          }
          takeaway={handsOffTakeaway(report)}
        >
          <DayChart
            days={handsOff}
            average={tiles.hands_off_median.value}
            format={(value) => `${continuous(value)} min`}
            name="Hands-off median by day"
          />
        </OverviewTile>
      </div>
    </DashCard>
  );
}

function DayChart({
  days,
  average,
  format,
  name,
}: {
  days: readonly DayValue[];
  average: number | null;
  format: (value: number) => string;
  name: string;
}) {
  // Nothing measured on any day: the space stays, with no empty axes.
  if (!days.some((day) => day.value !== null))
    return <div className="xt-dayline" data-empty aria-hidden="true" />;
  return (
    <DayLine
      days={days}
      average={average}
      format={format}
      name={name}
      averageName={average === null ? 'none' : `the range, ${format(average)}`}
    />
  );
}

function OverviewTile({
  label,
  icon,
  rule,
  definition,
  context,
  tile,
  value = tile.value,
  format,
  unit,
  reason = tileReason(tile),
  placeholder,
  sub,
  takeaway,
  children,
}: {
  label: string;
  icon: MetricIconName;
  rule: RuleId;
  /** Plain words in place of the rule's own summary. */
  definition?: string;
  context?: string;
  tile: MetricTile;
  value?: number | null;
  format: (value: number) => string;
  unit?: string;
  reason?: string;
  /** Words in place of the number while there is none. */
  placeholder?: string;
  sub?: string;
  takeaway?: string | null;
  children: ReactNode;
}) {
  const measured = isMeasured(value);
  // Words in place of the number already say why there is none.
  const words = !measured && placeholder !== undefined;
  const change = measured ? tileDelta(tile) : undefined;
  const tip = [measured || !reason || words ? undefined : sentence(reason), tileTip(tile, context)]
    .filter(Boolean)
    .join(' ');
  return (
    <section
      className="xt-overview-tile"
      aria-label={label}
      data-testid="overview-tile"
      data-label={label}
    >
      <h3 className="xt-overview-label">
        <MetricIcon name={icon} />
        <span>{label}</span>
        <DefinitionInfo ruleId={rule} name={label} text={definition} context={tip || undefined} />
      </h3>
      <p className="xt-overview-value" data-testid="overview-value">
        {words ? (
          <span className="xt-overview-placeholder">{placeholder}</span>
        ) : (
          <MetricCell value={value} format={format} reason={reason} size={24} />
        )}
        {unit && measured && <span className="xt-overview-unit">{unit}</span>}
        {typeof change === 'number' && Number.isFinite(change) && (
          <span className="xt-overview-delta">{formatDelta(change)}</span>
        )}
      </p>
      <p className="xt-overview-sub">{measured || words ? sub : (sub ?? reason)}</p>
      {children}
      {takeaway && (
        <p className="xt-overview-take" data-testid="overview-takeaway">
          {takeaway}
        </p>
      )}
    </section>
  );
}

/**
 * Merged PRs: the count and its change, then the latest merged pull requests
 * of the range with the agent hours their linked sessions spent, read from the
 * same cached report the PRs page shows. No history chart.
 */
function MergedTile({
  report,
  range,
  autoCheck,
}: {
  report: DashboardMetrics;
  range: TimeRange;
  autoCheck?: AutoCheck;
}) {
  const { source } = useData();
  const days = rangeDays[range];
  const prTile = report.pr_effort.current.tile;
  const merged = mergedTileView(prTile, autoCheck, report.window);
  const tile = report.tiles.merged_prs;
  const list = useQuery({
    queryKey: queryKeys.prAnalytics(days, true),
    queryFn: () => source.pullRequestAnalytics(days, true),
    enabled: source.kind !== 'preview',
  });
  // The listed pull requests are this report's own merged ones, the set the
  // number counts; the PRs page only adds each one's title and agent hours.
  const markers = report.pr_effort.current.markers;
  const shown = latestMerged(markers, list.data?.report.rows ?? [], MERGED_LISTED);
  const takeaway = mergedTakeaway({
    merged: markers.length,
    shown: shown.length,
    complete: prTile.complete,
    list: list.isError
      ? 'failed'
      : list.data !== undefined
        ? 'read'
        : list.isFetching
          ? 'reading'
          : 'none',
  });
  return (
    <OverviewTile
      label="Merged PRs"
      icon="merge"
      rule="M-19"
      definition={MERGED_PRS_DEFINITION}
      // How fresh the facts are, then (for a complete count) the tile's one
      // note: untyped merges, a hidden change, or which links count.
      context={[merged.freshness, prTile.complete ? mergedTip(prTile, tile) : undefined]
        .filter(Boolean)
        .join(' ')}
      tile={tile}
      value={tile.value ?? merged.value}
      format={count}
      unit={merged.incomplete ? 'so far' : undefined}
      reason={prTile.complete ? tileReason(tile) : mergedUnknown(prTile)}
      placeholder={merged.placeholder}
      sub={merged.aside}
      takeaway={takeaway}
    >
      <ul className="xt-overview-prs" data-testid="overview-prs">
        {shown.map((row) => (
          <li key={`${row.repository}#${row.number}`}>
            <span className="xt-overview-pr-title">
              <span className="xt-overview-pr-number">#{row.number}</span>
              {row.title ?? row.repository}
            </span>
            <span className="xt-overview-pr-hours">
              {row.agentMs === null ? '—' : `${hoursText(row.agentMs)} h`}
            </span>
          </li>
        ))}
      </ul>
    </OverviewTile>
  );
}

/**
 * What a complete Merged PRs count includes: the Dashboard reads it with
 * inferred links removed first, so only exact and commit links count.
 */
export const MERGED_COUNTS =
  'Counts PRs linked by a direct link or a commit; guessed links are left out.';

/**
 * Why the Merged PRs tile has no number: linked pull requests whose saved
 * facts cannot say whether they merged in the range (never refreshed, a failed
 * refresh, or merged with no merge time).
 */
export const mergedUnknown = (tile: MetricMergedPrs) =>
  `Merge status unknown for ${plural(tile.unknown_facts, 'linked PR')}, so the total is unknown.`;

/**
 * A complete Merged PRs count's one note after its freshness: how many merged
 * ones have no type, else why no change is shown, else which links it counts.
 * An incomplete count has no note here: its freshness line already says what
 * is not checked and, when the checks need the user, points to the Effort
 * card's red !.
 */
export function mergedTip(tile: MetricMergedPrs, merged: MetricTile) {
  if (tile.unresolved_type > 0)
    return `${plural(tile.unresolved_type, 'merged PR')} ${tile.unresolved_type === 1 ? 'has' : 'have'} no known work type yet.`;
  return tileTip(merged) ?? MERGED_COUNTS;
}
