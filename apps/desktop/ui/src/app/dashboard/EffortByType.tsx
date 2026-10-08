import { Tooltip } from '@base-ui/react/tooltip';
import { useMemo, type CSSProperties, type ReactNode } from 'react';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricEffortCost } from '../../data/generated/MetricEffortCost';
import type { MetricLeverageDay } from '../../data/generated/MetricLeverageDay';
import { RulePopover } from '../../kit/RulePopover';
import { ruleSummary, type RuleId } from '../../kit/rules';
import { Segmented } from '../../kit/Segmented';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { InfoIcon } from './DefinitionInfo';
import { DetailDialog } from './DetailDialog';
import {
  NO_MODEL,
  OVER_FULL_DAY,
  dayCenter,
  dayLabel,
  effortChart,
  unpricedLine,
  type EffortBar,
  type EffortChart as Chart,
} from './effort-chart';
import { UNMEASURED } from '../../kit/format';
import { agentTime } from '../agent-duration';
import { costCell } from './pr-effort';
import type { BarMetric, EffortMetric } from './pr-effort';
import { HumanTimeline } from './HumanTimeline';
import { plural, unpricedText, usd } from './present';

const metricOptions = [
  { value: 'agent', label: 'agent h' },
  { value: 'human', label: 'human h' },
  { value: 'dollars', label: 'cost' },
] as const;

/** The bar chart's measure; the days and markers are the same in every view. */
const barMetric = (metric: EffortMetric): BarMetric => (metric === 'dollars' ? 'dollars' : 'agent');

export function EffortMetricToggle({
  value,
  onChange,
}: {
  value: EffortMetric;
  onChange: (value: EffortMetric) => void;
}) {
  return (
    <Segmented<EffortMetric>
      label="Effort measure"
      options={metricOptions}
      value={value}
      onChange={onChange}
    />
  );
}

const shortDate = (date: string) => {
  const [, month, day] = date.split('-');
  return `${month}-${day}`;
};
const percent = (fraction: number) => `${fraction * 100}%`;

/**
 * M-19 effort: the range's total of the chosen measure, then one bar per
 * local day for every session in the range, on one shared scale from zero.
 * Hovering or focusing a day reads every model it went to. Merge markers sit
 * on their own lane under the baseline and are not an allocation of effort.
 */
export function EffortByType({
  report,
  metric,
}: {
  report: DashboardMetrics;
  metric: EffortMetric;
}) {
  const { current } = report.pr_effort;
  const chart = useMemo(() => effortChart(current, barMetric(metric)), [current, metric]);
  const empty = current.cohort.sessions === 0;
  if (metric === 'human') return <HumanTimeline report={report} />;
  return (
    <div className="xt-effort" data-testid="effort-by-type">
      {empty ? (
        <p className="xt-dash-empty">No agent work was recorded in this range.</p>
      ) : (
        <p className="xt-effort-headline" data-testid="effort-headline">
          <strong data-testid="effort-total">{chart.headline.text}</strong>
        </p>
      )}
      {chart.days.length > 0 && <EffortPlot chart={chart} drawn={!empty} />}
    </div>
  );
}

/**
 * The chart composite: a gutter for the scale and the marker count, the plot
 * and the axis. Everything on the axis — each day's bar, the date ticks and
 * the merge markers — is placed from the one day-centre mapping in
 * `effort-chart`, so a bar, its tick and its marker line up whatever the
 * width. Each day is one tab stop whose name carries its total and largest
 * models; hovering or focusing it opens the day's card with every model.
 */
function EffortPlot({ chart, drawn }: { chart: Chart; drawn: boolean }) {
  const count = chart.days.length;
  const name = `Effort chart: daily ${chart.unit}, one bar per day${
    chart.scale ? ` on one shared scale from 0 to ${chart.scale.topText}` : ''
  }`;
  return (
    <div
      className="xt-effort-chart"
      role="group"
      aria-label={name}
      data-testid="effort-chart"
      data-empty={!drawn || undefined}
      style={{ '--days': count } as CSSProperties}
    >
      {drawn && (
        <>
          <div className="xt-effort-scale" aria-hidden="true">
            {chart.scale && (
              <>
                <span data-at="top">{chart.scale.topText}</span>
                <span data-at="mid">{chart.scale.midText}</span>
              </>
            )}
            {!chart.unmeasured && <span data-at="zero">0</span>}
          </div>
          <div className="xt-effort-plot" data-testid="effort-plot">
            <div className="xt-effort-grid" aria-hidden="true">
              {chart.scale && <span className="xt-effort-gridline" data-at="top" />}
              {chart.scale && <span className="xt-effort-gridline" data-at="mid" />}
              <span className="xt-effort-baseline" />
              {chart.reference && (
                <span
                  className="xt-effort-reference"
                  data-testid="effort-reference"
                  style={{ bottom: percent(chart.reference.fraction) }}
                >
                  {chart.reference.text && <small>{chart.reference.text}</small>}
                </span>
              )}
            </div>
            <div className="xt-effort-columns">
              {chart.bars.map((bar) => (
                <DayBar key={bar.date} bar={bar} metric={chart.metric} count={count} />
              ))}
            </div>
            {chart.unmeasured && (
              <p className="xt-effort-unmeasured" data-testid="effort-unmeasured">
                {chart.unmeasured}
              </p>
            )}
          </div>
        </>
      )}
      <small
        className="xt-effort-lane-label"
        title="Merged pull requests, by local merge day"
        data-testid="effort-marker-count"
      >
        {plural(
          chart.markers.reduce((sum, day) => sum + day.merged.length, 0),
          'PR',
        )}
      </small>
      <div className="xt-effort-axis">
        <div
          className="xt-effort-lane"
          role="group"
          aria-label="Merged pull requests by local merge day"
        >
          {chart.markers.map((day) => (
            <span
              key={day.date}
              role="img"
              aria-label={day.text}
              title={day.text}
              className="xt-effort-marker"
              style={{ left: percent(dayCenter(day.index, count)) }}
            >
              {day.merged.length}
            </span>
          ))}
        </div>
        <div className="xt-effort-ticks" aria-hidden="true">
          {chart.ticks.map((index) => (
            <span key={index} style={{ left: percent(dayCenter(index, count)) }}>
              {shortDate(chart.days[index]!.date)}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

/**
 * One day: its column is the hover and focus target, the bar inside it the
 * day's total. A day with nothing to draw keeps its empty slot, and still
 * says what it is.
 */
function DayBar({ bar, metric, count }: { bar: EffortBar; metric: BarMetric; count: number }) {
  const theme = useSurfaceTheme();
  // Days in the right half open their card to the left of the column.
  const side = bar.index >= count / 2 ? 'left' : 'right';
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        delay={0}
        closeOnClick={false}
        render={
          <span
            role="img"
            // WebKit skips non-controls on Tab unless they say otherwise.
            tabIndex={0}
            aria-label={bar.name}
            className="xt-effort-column"
            data-testid="effort-day"
            data-date={bar.date}
            data-state={bar.state}
            data-partial={bar.partial || undefined}
          />
        }
      >
        {bar.state === 'measured' && bar.fraction > 0 && (
          <span
            className="xt-effort-bar"
            data-testid="effort-bar"
            style={{ height: percent(bar.fraction) }}
          />
        )}
        {bar.partial && (
          <span
            className="xt-effort-plus"
            data-testid="effort-partial"
            style={{ bottom: percent(bar.fraction) }}
          >
            +
          </span>
        )}
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side={side}
          align="start"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup
            className="xt-rule-popover xt-effort-tip"
            data-testid="effort-day-card"
            aria-hidden="true"
          >
            <DayCard bar={bar} metric={metric} />
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

function DayCard({ bar, metric }: { bar: EffortBar; metric: BarMetric }) {
  return (
    <>
      <span className="xt-effort-tip-head">
        <span>{bar.label}</span>
        <b>{bar.totalText}</b>
      </span>
      {bar.models.length === 0 && (
        <span className="xt-effort-tip-note">
          {bar.state === 'unknown'
            ? 'No response on this day could be priced'
            : metric === 'agent'
              ? 'No agent time this day'
              : 'No usage this day'}
        </span>
      )}
      {bar.models.map((model) => (
        <span key={model.name} className="xt-effort-tip-row" data-testid="effort-day-model">
          <span>{model.name}</span>
          <b>{model.text}</b>
          <em>{model.share}</em>
        </span>
      ))}
      {bar.unpriced.length > 0 && (
        <span className="xt-effort-tip-note">{unpricedLine(bar.unpriced)}</span>
      )}
      {bar.overFullDay && <span className="xt-effort-tip-note">{OVER_FULL_DAY}</span>}
      {bar.merged.length > 0 && (
        <span className="xt-effort-tip-note">
          Merged {bar.merged.map((marker) => `#${marker.number}`).join(', ')}
        </span>
      )}
    </>
  );
}

/**
 * What the table's two kinds of day are: the range's own days for the bars,
 * whole days for the pair that leverage divides.
 */
const WHOLE_DAY_NOTE =
  'Agent time, cost and merged cover the range, so its first day starts when the range does. The whole-day columns run from midnight to midnight, as human time and leverage do; they are the same agent hours the Overview divides.';

/** A whole day's agent time from the leverage pair; unknown (—) when the report has none. */
const wholeAgentHours = (day: MetricLeverageDay | undefined) =>
  day === undefined ? UNMEASURED : agentTime(day.agent_ms);

/** A whole day's human time, written as agent time is; unknown (—) when a message's sender is. */
const yourHours = (ms: number | null | undefined) =>
  ms === null || ms === undefined ? UNMEASURED : agentTime(ms);

/** A day's cost, compact; a trailing `+` marks a priced subtotal that leaves responses out. */
function costShort(cost: MetricEffortCost) {
  const cell = costCell(cost);
  if (cell.state === 'none') return 'no usage';
  if (cell.state === 'unknown') return UNMEASURED;
  return `${usd(cell.value)}${cost.unpriced_observations > 0 ? '+' : ''}`;
}

/** Each unpriced model and why, as the cost report names them. */
const unpricedModels = (cost: MetricEffortCost) =>
  cost.unpriced
    .map(
      (item) =>
        `${item.model ?? NO_MODEL}: ${unpricedText[item.reason]}, ${plural(item.observations, 'response')}`,
    )
    .join('; ');

/**
 * How the card counts, one step away over the page: the card's definition,
 * the measure and its exceptions, then each day's totals and merged pull
 * requests as a table. Its trigger is the ⓘ beside the card's title: hover or
 * focus reads the short definition, as every card's ⓘ does, and a click opens
 * this dialog, so the card needs no further button and keeps its height.
 */
export function EffortMethod({
  report,
  metric,
  rule,
  context,
  open,
  onOpenChange,
  children,
}: {
  report: DashboardMetrics;
  metric: EffortMetric;
  /** The card's rule: its summary is the ⓘ's tip and the dialog's first line. */
  rule: RuleId;
  /** Read after the rule's summary, in the tip and in the dialog. */
  context?: string;
  /** Held by the page, so an open Details survives another range loading. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Further measurements of the same usage, after the daily values. */
  children?: ReactNode;
}) {
  const { current } = report.pr_effort;
  const chart = useMemo(() => effortChart(current, barMetric(metric)), [current, metric]);
  const cost = current.cohort.cost;
  const human = report.human_hours.current;
  // Agent and human time over the same whole day, paired in Rust (the
  // leverage days), matched to the range's rows by date.
  const wholeDays = useMemo(
    () => new Map(report.leverage.by_day.map((day) => [day.date, day])),
    [report.leverage],
  );
  return (
    <DetailDialog
      trigger={<InfoIcon />}
      label="Effort definition"
      title="How effort is counted · daily values"
      className="xt-effort-method"
      triggerBase=""
      triggerClassName="xt-dash-definition"
      testId="effort-method"
      open={open}
      onOpenChange={onOpenChange}
      wrapTrigger={(button) => (
        <RulePopover ruleId={rule} context={context} closeOnClick>
          {button}
        </RulePopover>
      )}
    >
      <p className="xt-dash-note" data-testid="effort-definition">
        {ruleSummary(rule)}
        {context && ` ${context}`}
      </p>
      <p className="xt-dash-note" data-testid="effort-notes">
        {metric === 'human'
          ? `Human time comes from the times you sent messages to agents, across every conversation and tool, on one timeline. When two of your messages are at most ${human.break_minutes} minutes apart (the break length in Settings), the time between them counts; a longer gap is a break. Each bar runs from a message to your last one before a break; a single message is a thin tick and adds no time. Stretches are split at local midnight. Every row is a whole day, midnight to midnight, the first one too; a stretch that began the evening before counts from midnight. Which messages are yours is the same as the human message count; if any is unknown, human time are unknown.`
          : `${
              metric === 'agent'
                ? `Each bar is one local day’s agent time over every session in the range, on one shared scale from zero. Agents running at the same time add up, so a day can pass 24 h; the dashed line marks 24 h. Each session’s hours count toward the model it used most within this range: the most responses, then the most output tokens. A session none of whose responses names a model counts its records that name one instead; one whose work names no model counts as ${NO_MODEL}. Sessions uses the same rule over the session’s whole history, so a session that changed models can show a different model there.`
                : 'Each bar is one local day’s API-equivalent cost over every session in the range, on the day each response was recorded, not the merge day, on one shared scale from zero. Every token of each response (input, output, cache reads and cache writes, Claude cache writes split by five-minute and one-hour lifetime) is priced at its own model’s public rate, as the API-equivalent cost below prices it, and counts toward that model.'
            } Markers count the pull requests merged on each local day, through confirmed links (exact and SHA; inferred links are removed first), and allocate no effort.`}
        {` ${WHOLE_DAY_NOTE}`}
        {metric === 'dollars' &&
          cost.unpriced_observations > 0 &&
          ` ${cost.unpriced_observations} of ${plural(cost.selected_observations, 'selected response')} could not be priced (${unpricedModels(cost)}); they add nothing to the bars. A day with some of them is drawn at its priced subtotal with a + above it, a day with none priced has no bar, and any total that leaves responses out is marked + in the table.`}
        {metric === 'dollars' &&
          cost.assumed_tier_observations > 0 &&
          ` ${plural(cost.assumed_tier_observations, 'Codex response')} recorded no service tier and ${cost.assumed_tier_observations === 1 ? 'is' : 'are'} priced at OpenAI’s default (standard) tier, which applies when a request names none.`}
      </p>
      {chart.days.length > 0 && (
        // The rows scroll inside the dialog; the region takes focus so the
        // keyboard reaches every day, in WebKit as in Chromium.
        <div
          className="xt-dash-table-scroll"
          role="region"
          aria-label="Effort per day scroll area"
          tabIndex={0}
        >
          <table className="xt-dash-table">
            <caption className="sr-only">Effort per day</caption>
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">Agent time</th>
                <th scope="col">Cost</th>
                <th scope="col">Merged</th>
                <th scope="col">Whole day: agent time</th>
                <th scope="col">Whole day: human time</th>
              </tr>
            </thead>
            <tbody>
              {chart.days.map((day, index) => (
                <tr key={day.date}>
                  <th scope="row">{dayLabel(day.date)}</th>
                  <td>{agentTime(day.agent_ms)}</td>
                  <td>{costShort(day.cost)}</td>
                  <td>
                    {chart.markers
                      .find((merged) => merged.index === index)
                      ?.merged.map((marker) => `#${marker.number}`)
                      .join(', ') || 'none'}
                  </td>
                  <td>{wholeAgentHours(wholeDays.get(day.date))}</td>
                  <td>{yourHours(wholeDays.get(day.date)?.human_ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {children}
    </DetailDialog>
  );
}
