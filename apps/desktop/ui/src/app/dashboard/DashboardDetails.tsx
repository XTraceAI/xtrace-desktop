import { createContext, useContext, type ReactNode } from 'react';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricCaptureGap } from '../../data/generated/MetricCaptureGap';
import type { MetricUsageSummary } from '../../data/generated/MetricUsageSummary';
import { StatePill } from '../../kit/Badge';
import { percent, tokens as formatTokens } from '../../kit/format';
import { MetricCell } from '../../kit/MetricCell';
import { DefinitionInfo } from './DefinitionInfo';
import { DetailDialog } from './DetailDialog';
import {
  captureGapText,
  captureText,
  excludedSurfaceText,
  fromPoints,
  inventoryState,
  plural,
  surfaceLabel,
  unpricedText,
  usageGapText,
  usd,
} from './present';
import { TokenDayChart } from './TokenDayChart';
import { UntimedBody, untimedSummary } from './UntimedNotice';

/** The measurement detail a viewer has open, if any. */
export type DetailKey = 'method' | 'coverage';

/** Which detail is open; kept by the page while another range loads. */
export const DetailContext = createContext<{
  open: DetailKey | null;
  setOpen: (key: DetailKey | null) => void;
}>({ open: null, setOpen: () => {} });

/** A `DetailDialog`'s controlled state for one of the page's measurement details. */
export function useDetail(key: DetailKey) {
  const { open, setOpen } = useContext(DetailContext);
  return {
    open: open === key,
    onOpenChange: (next: boolean) => setOpen(next ? key : null),
  };
}

/**
 * The range's recorded tokens and their API-equivalent cost, as two titled
 * sections of the Effort card's Details dialog: output is one of the card's
 * measures, and both are the same selected usage. Each keeps its definition
 * and its scope beside its title, and the cost is never read as a total when
 * part of the usage could not be priced.
 */
export function UsageMeasurements({ report }: { report: DashboardMetrics }) {
  const { cost } = report;
  return (
    <>
      <MeasurementSection
        id="xt-usage-tokens"
        title="Tokens per day"
        meta={tokenMeta(report)}
        testId="usage-tokens"
      >
        <TokenDayChart days={report.days} />
      </MeasurementSection>
      <MeasurementSection
        id="xt-usage-cost"
        title="API-equivalent cost"
        meta={`catalog ${cost.price_version} · as of ${cost.as_of}`}
        testId="usage-cost"
      >
        <CostBody report={report} />
      </MeasurementSection>
    </>
  );
}

function MeasurementSection({
  id,
  title,
  meta,
  testId,
  children,
}: {
  id: string;
  title: string;
  meta: string;
  testId: string;
  children: ReactNode;
}) {
  return (
    <section className="xt-dash-measure" aria-labelledby={id} data-testid={testId}>
      <header className="xt-dash-measure-header">
        <h3 id={id}>{title}</h3>
        <DefinitionInfo ruleId="M-04" name={title} />
        <span className="xt-dash-modal-meta">{meta}</span>
      </header>
      {children}
    </section>
  );
}

/**
 * Coverage, from the Sessions card's header: what the range's numbers could
 * measure — token measurement and its fixed 14-day gate, session capture,
 * hands-off timestamp health — and, when the index holds any, the history
 * with no timestamp that no dated number includes. The control reads as one
 * word; its name also states the capture inventory when it is known and not complete,
 * token measurement when it misses sessions, has gaps or fails its 14-day
 * gate, and the untimed count when there is one, so the exception is heard
 * without opening it.
 */
export function CoverageDetail({ report }: { report: DashboardMetrics }) {
  const { untimed_history: untimed, usage_gate_14d: gate } = report;
  const usage = report.usage_coverage.total;
  const inventory = inventoryState[report.capture_inventory];
  const gaps = gapsText(usage);
  const flags = [
    report.capture_inventory === 'fresh_complete' || report.capture_inventory === 'unknown'
      ? null
      : inventory.label,
    usage.measured < usage.sessions || gaps
      ? `tokens measured for ${usage.measured}/${plural(usage.sessions, 'session')}${gaps ? ` (${gaps})` : ''}`
      : null,
    gate.passes === false ? 'trailing 14 days below the 90% gate' : null,
    untimed.records > 0 ? `${plural(untimed.records, 'untimed record')}` : null,
  ].filter(Boolean);
  return (
    <DetailDialog
      {...useDetail('coverage')}
      trigger="Coverage"
      label={flags.length > 0 ? `Coverage: ${flags.join(', ')}` : undefined}
      title="Coverage"
      rule="M-18"
      meta={`${report.capture_inventory === 'unknown' ? '' : `${inventory.label} · `}tokens measured for ${usage.measured}/${plural(usage.sessions, 'session')}`}
      testId="coverage"
    >
      <CoverageBody report={report} />
      {/* This history is in no day of any range, so it changes none of the
          dated numbers; it is stated with the other measurement limits, and
          only when the report counts some. */}
      {untimed.records > 0 && (
        <section
          className="xt-untimed xt-coverage-untimed"
          aria-labelledby="xt-untimed-coverage"
          data-testid="coverage-untimed"
        >
          <h3 id="xt-untimed-coverage">Untimed history</h3>
          <p className="xt-dash-note" data-testid="untimed-summary">
            {untimedSummary(untimed)}
          </p>
          <UntimedBody untimed={untimed} />
        </section>
      )}
    </DetailDialog>
  );
}

function tokenMeta(report: DashboardMetrics) {
  const total = report.tokens.counters.total_tokens;
  // Coverage denominator: all sessions in range, including those without selected usage.
  const { measured, sessions } = report.usage_coverage.total;
  return `${total === null ? 'total unmeasured' : `${formatTokens(total)} recorded`} · ${measured}/${plural(sessions, 'session')} measured`;
}

function CostBody({ report }: { report: DashboardMetrics }) {
  const { cost } = report;
  const partial = cost.total_usd === null && cost.priced_observations > 0;
  return (
    <div className="xt-cost">
      <div className="xt-cost-total" data-testid="cost-total">
        <span className="xt-dash-label">Total</span>
        <MetricCell
          value={cost.total_usd}
          format={usd}
          size={24}
          reason={
            report.tiles.cost.reason ??
            `${cost.unpriced_observations} of ${plural(cost.selected_observations, 'selected response')} unpriced`
          }
        />
      </div>
      {cost.total_usd === null && (
        <p className="xt-dash-note" data-testid="cost-subtotal">
          {partial
            ? `Partial subtotal ${usd(cost.priced_subtotal_usd)} covers only ${cost.priced_observations} of ${plural(cost.selected_observations, 'selected response')}; it is not a total.`
            : cost.selected_observations === 0
              ? 'No selected usage to price in this range.'
              : `None of ${plural(cost.selected_observations, 'selected response')} could be priced.`}
        </p>
      )}
      <dl className="xt-dash-facts">
        <dt>Priced</dt>
        <dd>
          {cost.priced_observations} of {plural(cost.selected_observations, 'response')}
        </dd>
        <dt>Unpriced</dt>
        <dd>{plural(cost.unpriced_observations, 'response')}</dd>
        {cost.assumed_tier_observations > 0 && (
          <>
            <dt>Assumed tier</dt>
            <dd data-testid="cost-assumed-tier">
              {plural(cost.assumed_tier_observations, 'Codex response')} at OpenAI’s default tier
              (no tier recorded)
            </dd>
          </>
        )}
      </dl>
      {cost.unpriced.length > 0 && (
        <ul className="xt-dash-list" aria-label="Unpriced usage">
          {cost.unpriced.map((item, index) => (
            <li key={index}>
              <span>
                {item.model ?? 'unknown model'} · {item.service_tier ?? 'no service tier'}
              </span>
              <span>
                {unpricedText[item.reason]} · {plural(item.observations, 'response')}
              </span>
            </li>
          ))}
        </ul>
      )}
      <p className="xt-dash-note">{cost.basis}</p>
    </div>
  );
}

const usageText = (usage: MetricUsageSummary) =>
  `${usage.measured} of ${plural(usage.sessions, 'session')}`;
const gapsText = (usage: MetricUsageSummary) =>
  usage.gaps.length ? usage.gaps.map((gap) => usageGapText[gap]).join(', ') : null;

/** Nothing checks the session inventory yet, so "unknown" says nothing and is not shown. */
const shownGaps = (gaps: MetricCaptureGap[]) => gaps.filter((gap) => gap !== 'inventory_unknown');

function CoverageBody({ report }: { report: DashboardMetrics }) {
  const { usage_coverage: usage, usage_gate_14d: gate } = report;
  const inventory = inventoryState[report.capture_inventory];
  return (
    <div className="xt-coverage">
      <section aria-labelledby="xt-usage-coverage">
        <h3 id="xt-usage-coverage">Token measurement</h3>
        <p className="xt-dash-line" data-testid="usage-coverage">
          Tokens measured for{' '}
          <MetricCell
            value={fromPoints(usage.total.pct)}
            format={percent}
            reason="No sessions to measure"
          />{' '}
          of sessions ({usageText(usage.total)})
          {gapsText(usage.total) && ` · ${gapsText(usage.total)}`}
        </p>
        {usage.by_surface.length > 0 && (
          <ul className="xt-dash-list" aria-label="Token measurement by surface">
            {usage.by_surface.map((row) => (
              <li key={`${row.host}:${row.surface}`}>
                <span>{surfaceLabel(row.host, row.surface)}</span>
                <span>
                  <MetricCell
                    value={fromPoints(row.usage.pct)}
                    format={percent}
                    reason="No sessions to measure"
                  />{' '}
                  · {usageText(row.usage)}
                  {gapsText(row.usage) && ` · ${gapsText(row.usage)}`}
                </span>
              </li>
            ))}
          </ul>
        )}
        <p className="xt-dash-note" data-testid="usage-gate">
          Fixed trailing 14 days: <MetricCell value={fromPoints(gate.pct)} format={percent} /> of{' '}
          {plural(gate.eligible_sessions, 'eligible session')} measured ·{' '}
          {gate.passes === null
            ? 'gate unknown'
            : gate.passes
              ? 'meets the 90% gate'
              : 'below the 90% gate'}
          {gate.excluded_surfaces.length > 0 &&
            ` · excluded: ${gate.excluded_surfaces.map((item) => surfaceLabel(item.host, item.surface)).join(', ')}`}
        </p>
      </section>
      <section aria-labelledby="xt-capture-coverage">
        <h3 id="xt-capture-coverage">
          Session capture
          {report.capture_inventory !== 'unknown' && (
            <>
              {' '}
              <StatePill tone={inventory.tone} height={20}>
                {inventory.label}
              </StatePill>
            </>
          )}
        </h3>
        {report.capture_coverage.length === 0 ? (
          <p className="xt-dash-line">No session capture coverage rows available.</p>
        ) : (
          <ul className="xt-dash-list" aria-label="Session capture by surface">
            {report.capture_coverage.map((row) => (
              <li key={`${row.host}:${row.surface}`}>
                <span>{surfaceLabel(row.host, row.surface)}</span>
                <span>
                  {captureText(row)} ·{' '}
                  <MetricCell
                    value={fromPoints(row.pct)}
                    format={percent}
                    reason="Capture share unknown"
                  />
                  {shownGaps(row.incomplete_reasons).length > 0 &&
                    ` · ${shownGaps(row.incomplete_reasons)
                      .map((gap) => captureGapText[gap])
                      .join(', ')}`}
                </span>
              </li>
            ))}
          </ul>
        )}
        <p className="xt-dash-note">
          Receipts are historical: a captured session does not mean a surface is capturing now.
          Indexing does not certify capture coverage.
        </p>
      </section>
      <section aria-labelledby="xt-hands-off-coverage">
        <h3 id="xt-hands-off-coverage">Hands-off timestamp health</h3>
        {report.hands_off_excluded_surfaces.length === 0 ? (
          <p className="xt-dash-line">No surface is excluded from hands-off.</p>
        ) : (
          <ul className="xt-dash-list" aria-label="Surfaces excluded from hands-off">
            {report.hands_off_excluded_surfaces.map((item) => (
              <li key={`${item.host}:${item.surface}`}>{excludedSurfaceText(item)}</li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
