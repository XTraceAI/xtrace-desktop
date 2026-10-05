import type { DashboardUntimed } from '../../data/generated/DashboardUntimed';
import { plural, surfaceLabel } from './present';
import '../../styles/untimed.css';

/**
 * One wording for a fact both the Dashboard and Sessions have to state: part of
 * the indexed history carries no timestamp, so no date-based measurement on
 * either page can include it.
 *
 * Everything shown here was counted by the Rust report. The count covers all
 * indexed history — it is not a window measurement and cannot become one, since
 * a record with no timestamp belongs to no day — so the wording says out loud
 * that it does not follow the selected dates or the table's own filters.
 *
 * Nothing is reported when the count is zero: a page says this only when it is
 * true of the history it is reading. Nothing here calls a host broken, supplies
 * a timestamp the source never stated, or turns the count into a share of
 * anything, which would read as a capture or coverage percentage.
 */
export const UNTIMED_NOTICE =
  'Some indexed history has no timestamps and cannot contribute to date-based measurements.';

/** The one line that stays in view: what this history is and how much of it there is. */
export const UNTIMED_SUMMARY = 'outside dated measurements';

/** The count and its scope, as the line's summary states them. */
export const untimedSummary = (untimed: DashboardUntimed) =>
  `${plural(untimed.records, 'record')} · ${UNTIMED_SUMMARY}`;

/**
 * The explanation, the all-history count and the host and surface breakdown:
 * the same body whichever page's disclosure opens it.
 */
export function UntimedBody({ untimed }: { untimed: DashboardUntimed }) {
  return (
    <div className="xt-untimed-body">
      <p className="xt-untimed-lead">{UNTIMED_NOTICE}</p>
      <p className="xt-untimed-count" data-testid="untimed-count">
        {plural(untimed.records, 'record')} in all indexed history, independent of the selected
        dates and of any filter on the table.
      </p>
      {untimed.by_surface.length > 0 && (
        <ul className="xt-dash-list" aria-label="Untimed indexed history by host and surface">
          {/* An unstated surface keys its own row, never the same key as a
              surface a source happens to have named "null" or "". */}
          {untimed.by_surface.map((row) => (
            <li key={JSON.stringify([row.host, row.surface])}>
              <span>{surfaceLabel(row.host, row.surface)}</span>
              <span>{plural(row.records, 'record')}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Compact by design: a single line among the page's measurement limits, not a
 * banner over its numbers. The line itself states the limit and the count, so
 * it is discoverable without opening anything; the explanation and the host
 * and surface breakdown open from its own keyboard-operable summary.
 *
 * `inline` draws the same line as one item in a row of page-status items, for
 * a page that states its limits beside its range summary rather than in a
 * measurement-details list. Wording, count and disclosure are unchanged.
 */
export function UntimedNotice({
  untimed,
  inline = false,
}: {
  untimed: DashboardUntimed | undefined;
  inline?: boolean;
}) {
  if (!untimed || untimed.records === 0) return null;
  return (
    <section
      className="xt-untimed"
      data-inline={inline || undefined}
      aria-label="Untimed indexed history"
    >
      <details className="xt-untimed-details">
        <summary>
          <span className="xt-untimed-title">Untimed history</span>
          <span className="xt-untimed-meta" data-testid="untimed-summary">
            {untimedSummary(untimed)}
          </span>
          {/* Inline, the row's own marker shows the disclosure; the word would cost the row a line. */}
          {!inline && (
            <span className="xt-untimed-toggle" aria-hidden="true">
              Details
            </span>
          )}
        </summary>
        <UntimedBody untimed={untimed} />
      </details>
    </section>
  );
}
