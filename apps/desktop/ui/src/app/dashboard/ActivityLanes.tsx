import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import { HostGlyph } from '../../kit/HostGlyph';
import { clockTime, plural } from './present';

const HOUR = 3_600_000;
const shortId = (id: string) => id.replace(/^(codex|cursor)-/, '').slice(0, 8);

/**
 * Returned active spans on the report's fixed recent axis, independent of the selected range.
 * One row per span: a session with several spans (gaps over 20 minutes) has several rows.
 */
export function ActivityLanes({ report }: { report: DashboardMetrics }) {
  const { lanes, lane_start_ms: start, lane_end_ms: end, window } = report;
  const span = Math.max(1, end - start);
  const hours = Math.round(span / HOUR);
  const ticks = Array.from({ length: 5 }, (_, index) => start + (span * index) / 4);
  const position = (ms: number) => (Math.min(end, Math.max(start, ms)) - start) / span;
  if (lanes.length === 0)
    return (
      <p className="xt-dash-empty">
        No active span in the last {hours} hours ({clockTime(start, window)} –{' '}
        {clockTime(end, window)}).
      </p>
    );
  return (
    <div className="xt-lanes">
      <div className="xt-lanes-axis" aria-hidden="true">
        <span />
        <span className="xt-lanes-ticks">
          {ticks.map((tick, index) => (
            <span key={index} style={{ left: `${(index / 4) * 100}%` }}>
              {clockTime(tick, window, index === 0 || index === 4)}
            </span>
          ))}
        </span>
      </div>
      <div className="xt-lanes-scroll" tabIndex={0} role="region" aria-label="Session lanes">
        <ol className="xt-lanes-list">
          {lanes.map((lane) => {
            const left = position(lane.start_ms);
            const width = position(lane.end_ms) - left;
            const text = `${lane.host} session ${lane.session_id}: active span ${clockTime(lane.start_ms, window)} – ${clockTime(lane.end_ms, window)}${lane.start_ms === lane.end_ms ? ' (single event)' : ''}`;
            return (
              <li
                key={JSON.stringify([lane.host, lane.session_id, lane.start_ms, lane.end_ms])}
                className="xt-lane"
              >
                <span className="xt-lane-label" title={lane.session_id}>
                  <HostGlyph host={lane.host} size={16} />
                  <span aria-hidden="true">{shortId(lane.session_id)}</span>
                </span>
                <span className="xt-lane-track" aria-hidden="true">
                  {[1, 2, 3].map((line) => (
                    <i key={line} style={{ left: `${line * 25}%` }} />
                  ))}
                  <span
                    className="xt-lane-span"
                    title={text}
                    style={{ left: `${left * 100}%`, width: `${width * 100}%` }}
                  />
                </span>
                <span className="sr-only">{text}</span>
              </li>
            );
          })}
        </ol>
      </div>
      <p className="xt-dash-note" data-testid="lanes-disclosure">
        {report.lanes_truncated
          ? `Showing the ${plural(lanes.length, 'most recent active span')} of ${report.lanes_total.toLocaleString('en-US')} in the last ${hours} hours. Metrics still include every span.`
          : `${plural(report.lanes_total, 'active span')} in the last ${hours} hours, whatever range is selected. A session can have several spans.`}
      </p>
    </div>
  );
}
