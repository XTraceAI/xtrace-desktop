import { surfaceLabel } from '../kit/hosts';
import { MetricCell } from '../kit/MetricCell';
import { RulePopover } from '../kit/RulePopover';
import type { RuleId } from '../kit/rules';
import type { SessionRow } from '../data/generated/SessionRow';
import { agentDuration } from './agent-duration';
import { handsOffTime } from './metric-format';
import { displayTitle, shortId } from './session-context';

/**
 * One listed session's cells, shared by the Sessions list and a pull request's
 * linked-session drilldown, which read the same `SessionRow` through the same
 * assembler. Presentation only: every value is the row's own measurement.
 */

/** A row's measurements, or nothing when its session is not indexed (M-01). */
export const indexed = (row: SessionRow) => (row.metrics.state === 'indexed' ? row.metrics : null);
export const NOT_INDEXED = 'This session is not indexed, so nothing was measured';

/**
 * A measured column's own definition, quoting its S2 rule. The header label is
 * the trigger, so the column stays as compact as an unmeasured one, and it is a
 * real button: hover, focus and Escape are Base UI's, and one tab stop per
 * metric reaches it. Every cell beneath is tied to it by the grid's
 * columnheader/cell roles, so a number is never shown without its rule.
 */
export function MetricHeader({
  label,
  name = label,
  ruleId,
}: {
  label: string;
  /** Spoken column name when the visible label is abbreviated for density. */
  name?: string;
  ruleId: RuleId;
}) {
  return (
    <RulePopover ruleId={ruleId}>
      <button
        type="button"
        className="xt-table-metric-header"
        aria-label={`${name}, definition ${ruleId}`}
      >
        {label}
      </button>
    </RulePopover>
  );
}

/** A session's visible name: saved title, verified reviewer, or identity. */
export const sessionName = (row: SessionRow) =>
  displayTitle(row.title, row.automated_review) ?? `Session ${shortId(row.id)}`;

/** A link's evidence in words; one wording, kept beside the PRs report's other words. */
export { evidenceWords } from './pr-analytics';

/** Why a hands-off median is not shown, in the M-09 contract's own terms. */
export function handsOffReason(row: SessionRow): string {
  const handsOff = row.hands_off;
  switch (handsOff.state) {
    case 'missing':
      return NOT_INDEXED;
    case 'unmeasured': {
      const surface = handsOff.excluded_surface;
      return surface
        ? `Excluded: ${surfaceLabel(surface.host, surface.surface)} timestamps are too coarse (${surface.degenerate_sessions} of ${surface.qualifying_sessions} sessions)`
        : 'A record in this window leaves a stretch boundary or its tool use unknown';
    }
    case 'measured':
      return 'No hands-off stretch in this window, so there is no median';
  }
}

export function HandsOff({ row }: { row: SessionRow }) {
  const handsOff = row.hands_off;
  const median = handsOff.state === 'measured' ? handsOff.median_min : null;
  const cell = (
    <MetricCell value={median} format={handsOffTime} align="right" reason={handsOffReason(row)} />
  );
  return handsOff.state === 'measured' && median !== null ? (
    <span
      className="xt-session-handsoff"
      title={`Median of ${handsOff.n} ${handsOff.n === 1 ? 'stretch' : 'stretches'}, in minutes`}
    >
      {cell}
      <span className="sr-only">
        {' '}
        median of {handsOff.n} {handsOff.n === 1 ? 'stretch' : 'stretches'}
      </span>
    </span>
  ) : (
    cell
  );
}

/**
 * The row's M-05 active time as hours and remaining whole minutes
 * (`3 h 13 m`). The compact text is drawn for the eye; a screen reader hears
 * the same value in words with the exact millisecond measurement, which is
 * also the tooltip and a line in the row's details. An unindexed session keeps
 * its reason, never a zero.
 */
export function AgentTime({ row }: { row: SessionRow }) {
  const metrics = indexed(row);
  if (!metrics) return <MetricCell value={null} align="right" reason={NOT_INDEXED} />;
  const duration = agentDuration(metrics.agent_ms);
  return (
    <span className="xt-session-agent" title={`Exactly ${duration.exact} active in this range`}>
      <span aria-hidden="true">
        <MetricCell value={duration.visible} align="right" />
      </span>
      <span className="sr-only">
        {duration.spoken}, exactly {duration.exact}
      </span>
    </span>
  );
}
