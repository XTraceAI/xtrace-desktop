import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { MetricTile } from '../data/generated/MetricTile';
import { Button } from '../kit/Button';
import { count, tokens as formatTokens } from '../kit/format';
import { StatTile } from '../kit/StatTile';
import type { TimeRange } from '../kit/TopBar';
import { plural, ruleId, tileDelta, tileTip } from './dashboard/present';
import { useDashboardReport } from './dashboard/range';
import { continuous } from './metric-format';
import '../styles/sessions.css';

/**
 * The four measurements the reference puts above the session list, read from
 * the same range report the Dashboard reads — the cached `dashboard(days)`
 * query, not a sum of the rows this page happens to have loaded and not a
 * query of its own. The list below pages and filters; these do not, which is
 * exactly why the page's subtitle says so out loud in one short line, and why
 * the whole sentence is the summary's description.
 */
export const SUMMARY_SCOPE =
  'The summary counts all indexed activity in the selected range; filters narrow only the table.';
/** The same rule as the heading's one visible line. */
export const SUMMARY_SCOPE_SHORT = 'Range: all indexed activity · filters: table only';

/** Agent time is reported in hours (M-05); the tile states it in minutes. */
const MINUTES_PER_HOUR = 60;
const AGENT_MINUTES = 'Active-span time in this range, stated in minutes.';
const OUTPUT_INDEPENDENT =
  'Output tokens are measured on their own, so they are shown even when the range total is not.';
/**
 * M-04 leaves output null when its counters are absent and when a response
 * mixes measured counters with missing ones; the report does not say which, so
 * the tile must not read the null as "there was no output".
 */
const NO_OUTPUT = 'Output counters are absent or incomplete';

export function SessionsSummary({
  range,
  describedBy,
}: {
  range: TimeRange;
  /** The page's own statement of what the summary covers, `SUMMARY_SCOPE`. */
  describedBy: string;
}) {
  const report = useDashboardReport(range);
  // A failed refresh keeps the last successful report in the cache, but the
  // page promises that a failure shows no numbers — a retained value, change
  // or busiest day would be presented as current when it is not confirmed. So
  // the data is dropped with the error, not merely annotated with it.
  const metrics = report.isError ? undefined : report.data;
  // Until the report answers, every tile says why it has no number. A summary
  // that is loading or failed never stands a zero in for an unread metric, and
  // never replaces the table underneath it.
  const pending: string | undefined = report.isError
    ? 'Range metrics could not be loaded'
    : metrics
      ? undefined
      : 'Range metrics are still being read';
  const tip = (tile: MetricTile | undefined, ...extra: (string | null | undefined)[]) =>
    tile ? tileTip(tile, ...extra) : pending;
  return (
    <section
      className="xt-sessions-summary"
      aria-label="Range summary"
      aria-describedby={describedBy}
      aria-busy={report.isFetching || undefined}
    >
      {report.isError && (
        <p className="xt-sessions-summary-error" role="status">
          <span>Range metrics could not be loaded; the session list below is unaffected.</span>
          <Button
            variant="outline"
            height={28}
            disabled={report.isFetching}
            onClick={() => void report.refetch()}
          >
            Retry
          </Button>
        </p>
      )}
      <div className="xt-sessions-tiles">
        <HumanMessages metrics={metrics} pending={pending} tip={tip} />
        <OutputTokens metrics={metrics} pending={pending} />
        <AgentMinutes metrics={metrics} pending={pending} tip={tip} />
        <SessionsPerDay metrics={metrics} pending={pending} tip={tip} />
      </div>
    </section>
  );
}

type TileProps = {
  metrics: DashboardMetrics | undefined;
  pending: string | undefined;
  tip: (
    tile: MetricTile | undefined,
    ...extra: (string | null | undefined)[]
  ) => string | undefined;
};

function HumanMessages({ metrics, pending, tip }: TileProps) {
  const tile = metrics?.tiles.human_messages;
  return (
    <StatTile
      label="Human messages"
      icon="msg"
      ruleId={tile ? ruleId(tile.rule_id, 'M-02') : 'M-02'}
      value={tile?.value ?? null}
      format={count}
      reason={pending ?? tile?.reason ?? undefined}
      delta={tile && tileDelta(tile)}
      tip={tip(tile)}
    />
  );
}

/**
 * M-04 counts one usage row per API response and sums the four canonical
 * counters only when all four are measured, so a range whose total is unknown
 * can still have a measured output figure. This tile reports that figure
 * directly from the report's counters; the report publishes no previous-period
 * output, so no change is shown rather than one being derived here.
 */
function OutputTokens({ metrics, pending }: Pick<TileProps, 'metrics' | 'pending'>) {
  const usage = metrics?.tokens;
  const total = usage?.counters.total_tokens ?? null;
  const coverage = metrics?.usage_coverage.total;
  return (
    <StatTile
      label="Output tokens"
      icon="token"
      ruleId="M-04"
      value={usage?.counters.output_tokens ?? null}
      format={formatTokens}
      reason={pending ?? NO_OUTPUT}
      tip={
        metrics && coverage
          ? [
              OUTPUT_INDEPENDENT,
              total === null
                ? 'The range total is unmeasured.'
                : `Range total ${formatTokens(total)} including cache.`,
              // `usage_coverage` counts sessions with no gap at all, so this
              // is complete four-counter, known-model coverage. It is not a
              // count of sessions with measured output, and says so.
              `Coverage: ${coverage.measured} of ${plural(coverage.sessions, 'session')} have all four counters and a known model, which is not the same as having measured output.`,
            ].join(' ')
          : pending
      }
    />
  );
}

function AgentMinutes({ metrics, pending, tip }: TileProps) {
  const tile = metrics?.tiles.agent_hours;
  // An unmeasured hour count stays unmeasured; only a number is converted.
  const value = typeof tile?.value === 'number' ? tile.value * MINUTES_PER_HOUR : null;
  return (
    <StatTile
      label="Agent minutes"
      icon="clock"
      ruleId={tile ? ruleId(tile.rule_id, 'M-05') : 'M-05'}
      value={value}
      format={continuous}
      unit="min"
      reason={pending ?? tile?.reason ?? undefined}
      // A percentage change is the same in minutes as in hours, so the
      // report's own change carries over without being recalculated.
      delta={tile && tileDelta(tile)}
      tip={tip(tile, AGENT_MINUTES)}
    />
  );
}

/**
 * The mean is the report's own (M-16), denominator included. The aside is the
 * busiest of the very day buckets that mean divides by — the max/day the rule
 * already reports — so it is read from `days`, never recounted, and it is
 * omitted when the report carries no buckets at all.
 */
function SessionsPerDay({ metrics, pending, tip }: TileProps) {
  const tile = metrics?.tiles.sessions_per_day;
  const buckets = metrics?.days ?? [];
  const busiest = buckets.length ? Math.max(...buckets.map((day) => day.sessions)) : null;
  return (
    <StatTile
      label="Sessions / day"
      icon="lanes"
      ruleId={tile ? ruleId(tile.rule_id, 'M-16') : 'M-16'}
      value={tile?.value ?? null}
      format={continuous}
      unit="mean"
      reason={pending ?? tile?.reason ?? undefined}
      delta={tile && tileDelta(tile)}
      aside={busiest === null ? undefined : `max ${count(busiest)}`}
      tip={tip(
        tile,
        busiest === null ? null : 'The max is the busiest single day bucket in this range.',
      )}
    />
  );
}
