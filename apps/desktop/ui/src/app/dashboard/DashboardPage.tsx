import { createContext, useContext, useId, useState, type ReactNode } from 'react';
import { Link } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricTile } from '../../data/generated/MetricTile';
import type { MetricUsageSummary } from '../../data/generated/MetricUsageSummary';
import { StatePill } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import {
  count,
  delta as formatDelta,
  hours,
  minutes,
  percent,
  tokens as formatTokens,
} from '../../kit/format';
import { MetricCell } from '../../kit/MetricCell';
import { RuleChip } from '../../kit/RuleChip';
import { RulePopover } from '../../kit/RulePopover';
import type { RuleId } from '../../kit/rules';
import { SectionCard } from '../../kit/SectionCard';
import { StatTile } from '../../kit/StatTile';
import type { TimeRange } from '../../kit/TopBar';
import { freshnessText, phaseText } from '../native-index-text';
import { useNativeIndexStatus } from '../useAppInfo';
import { ActivityLanes } from './ActivityLanes';
import { EnvironmentPanel } from './EnvironmentPanel';
import {
  captureGapText,
  captureText,
  excludedSurfaceText,
  favoriteUnknownText,
  fromPoints,
  inventoryState,
  plural,
  ruleId,
  surfaceLabel,
  tileDelta,
  unavailableReason,
  unpricedText,
  usageGapText,
  usd,
} from './present';
import { useDashboardReport, useSelectedRange } from './range';
import { TokenDayChart } from './TokenDayChart';
import '../../styles/dashboard.css';

const TITLE = 'What your agents did';

/** Which supplementary sections are expanded; kept while another range loads. */
const ExpandedContext = createContext<{
  expanded: ReadonlySet<string>;
  toggle: (title: string, open: boolean) => void;
}>({ expanded: new Set(), toggle: () => {} });

export function DashboardPage() {
  const { source } = useData();
  const range = useSelectedRange();
  const report = useDashboardReport(range);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const toggle = (title: string, open: boolean) =>
    setExpanded((current) => {
      if (current.has(title) === open) return current;
      const next = new Set(current);
      if (open) next.add(title);
      else next.delete(title);
      return next;
    });
  let body: ReactNode;
  if (source.kind === 'preview')
    body = <p className="xt-dash-status">Open the desktop app to read local Dashboard metrics.</p>;
  else if (report.isPending)
    body = (
      <p role="status" className="xt-dash-status">
        Reading Dashboard metrics for the last {range}…
      </p>
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
  else body = <DashboardReport report={report.data} range={range} />;
  return (
    <section className="xt-dashboard" aria-busy={report.isFetching || undefined}>
      <h1>{TITLE}</h1>
      <ExpandedContext.Provider value={{ expanded, toggle }}>{body}</ExpandedContext.Provider>
    </section>
  );
}

const sampleText = (tile: MetricTile) =>
  tile.current_n === null
    ? null
    : `n = ${tile.current_n.toLocaleString('en-US')} ${tile.sample_unit}; previous period ${tile.previous_n?.toLocaleString('en-US') ?? '—'}.`;
function tileTip(tile: MetricTile, ...extra: (string | null | undefined)[]) {
  const parts = [tile.reason, tile.note, ...extra, sampleText(tile)];
  if (tile.current_n !== null && tile.delta.suppressed)
    parts.push(
      'Change vs the previous period is hidden: a window has fewer than 5 samples or the previous value is zero.',
    );
  return parts.filter(Boolean).join(' ');
}

function DashboardReport({ report, range }: { report: DashboardMetrics; range: TimeRange }) {
  const { tiles, favorite } = report;
  const favoriteModel = favorite.current.model;
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

      <div className="xt-dash-row xt-dash-hero-row">
        <HeroCard report={report} />
        <DashCard
          title="Caught by your rules"
          rule={ruleId(tiles.rule_fires.rule_id, 'R-05')}
          className="xt-rules-card"
        >
          <UnavailableLine
            reason={
              tiles.rule_fires.reason ??
              unavailableReason(report.unavailable, 'rule_fires', 'Rule-fire data is unavailable')
            }
          />
        </DashCard>
      </div>

      <div className="xt-dash-tiles">
        <StatTile
          label="Agent h/day"
          icon="clock"
          ruleId={ruleId(tiles.agent_hours_per_day.rule_id, 'M-05')}
          value={tiles.agent_hours_per_day.value}
          format={hours}
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
          format={hours}
          unit="mean"
          reason={tiles.concurrency_mean.reason ?? undefined}
          delta={tileDelta(tiles.concurrency_mean)}
          aside={
            tiles.concurrency_max.value === null
              ? undefined
              : `max ${count(tiles.concurrency_max.value)}`
          }
          tip={tileTip(tiles.concurrency_mean)}
        />
        <StatTile
          label="Merged PRs"
          icon="merge"
          ruleId={ruleId(tiles.merged_prs.rule_id, 'M-14')}
          value={tiles.merged_prs.value}
          format={count}
          reason={tiles.merged_prs.reason ?? undefined}
          delta={tileDelta(tiles.merged_prs)}
          tip={tileTip(tiles.merged_prs)}
        />
        <StatTile
          label="Hands-off median"
          icon="shield"
          ruleId={ruleId(tiles.hands_off_median.rule_id, 'M-09')}
          value={tiles.hands_off_median.value}
          format={minutes}
          unit="min"
          reason={tiles.hands_off_median.reason ?? undefined}
          delta={tileDelta(tiles.hands_off_median)}
          aside={
            tiles.hands_off_p90.value === null
              ? undefined
              : `p90 ${minutes(tiles.hands_off_p90.value)}`
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
        <DashCard title="Effort by type" rule="M-19" className="xt-reserved-card">
          <UnavailableLine
            reason={unavailableReason(
              report.unavailable,
              'work_type',
              'Work-type data is unavailable',
            )}
          />
        </DashCard>
        <DashCard title="Environment" rule="M-17" className="xt-env-card">
          <EnvironmentPanel range={range} />
        </DashCard>
      </div>

      <SectionCard
        title="Sessions"
        ruleId="M-05"
        meta="Active spans · last 48 hours"
        right={
          <Link className="xt-dash-link" to="/sessions">
            View sessions
          </Link>
        }
        padding="12px 14px"
      >
        <ActivityLanes report={report} />
      </SectionCard>

      <div className="xt-dash-extras" role="group" aria-label="Measurement details">
        <ExtraSection
          title="Tokens per day"
          rule="M-04"
          meta={tokenMeta(report)}
          summary="Show daily tokens"
        >
          <TokenDayChart days={report.days} />
        </ExtraSection>
        <CostSection report={report} />
        <CoverageSection report={report} />
        <IndexLine />
      </div>
    </>
  );
}

function SummaryCount({ tile, one }: { tile: MetricTile; one: string }) {
  return tile.value === null ? (
    <span>
      <MetricCell value={null} reason={tile.reason ?? undefined} /> {one}s
    </span>
  ) : (
    <span>{plural(tile.value, one)}</span>
  );
}

/** A compact titled card without the nested panel SectionCard draws. */
function DashCard({
  title,
  rule,
  className = '',
  children,
}: {
  title: string;
  rule: RuleId;
  className?: string;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <section className={`xt-dash-card ${className}`} aria-labelledby={id}>
      <header className="xt-dash-card-header">
        <h2 id={id}>{title}</h2>
        <RuleChip ruleId={rule} size="sm" />
      </header>
      {children}
    </section>
  );
}

function UnavailableLine({ reason }: { reason: string }) {
  return (
    <p className="xt-dash-unavailable">
      <MetricCell value={null} reason={reason} size={18} />
      <span>{reason}.</span>
    </p>
  );
}

const HUMAN_ESTIMATE =
  'Human-in-the-loop time is estimated from message timing and typing length, unioned on the wall clock.';

function HeroValue({
  tile,
  fallback,
  name,
  context,
  format = hours,
  size = 24,
  children,
}: {
  tile: MetricTile;
  fallback: RuleId;
  name: string;
  context?: string;
  format?: (value: number) => string;
  size?: 11 | 24;
  children?: ReactNode;
}) {
  return (
    <RulePopover ruleId={ruleId(tile.rule_id, fallback)} context={tileTip(tile, context)}>
      <button type="button" className="xt-hero-value">
        <span className="sr-only">{name} </span>
        <MetricCell
          value={tile.value}
          format={format}
          reason={tile.reason ?? undefined}
          size={size}
        />
        {children}
      </button>
    </RulePopover>
  );
}

function HeroCard({ report }: { report: DashboardMetrics }) {
  const { agent_hours: agent, human_hours_est: human, ratio } = report.tiles;
  const changes = [
    ['agent', tileDelta(agent)],
    ['human', tileDelta(human)],
  ].filter((entry): entry is [string, number] => entry[1] !== undefined);
  return (
    <DashCard title="Agent / human hours" rule="M-08" className="xt-hero-card">
      <p className="xt-hero-line">
        <HeroValue tile={agent} fallback="M-05" name="Agent hours" />
        <span className="xt-hero-sep" aria-hidden="true">
          /
        </span>
        <HeroValue
          tile={human}
          fallback="M-07"
          name="Human-in-the-loop hours, estimated"
          context={HUMAN_ESTIMATE}
        />
        <span className="xt-hero-unit">h</span>
        {human.value !== null && <span className="xt-hero-est">human est.</span>}
      </p>
      <p className="xt-hero-sub">
        <HeroValue tile={ratio} fallback="M-08" name="Agent to human ratio" size={11}>
          {ratio.value !== null && <span className="xt-hero-unit">× agent : human</span>}
        </HeroValue>
        {changes.map(([name, change]) => (
          <span key={name}>
            {name} {formatDelta(change)} vs previous
          </span>
        ))}
      </p>
    </DashCard>
  );
}

function ExtraSection({
  title,
  rule,
  meta,
  summary,
  children,
}: {
  title: string;
  rule: RuleId;
  meta?: string;
  summary: string;
  children: ReactNode;
}) {
  const id = useId();
  const { expanded, toggle } = useContext(ExpandedContext);
  return (
    <section className="xt-dash-extra" aria-labelledby={id}>
      <header className="xt-dash-card-header">
        <h2 id={id}>{title}</h2>
        <RuleChip ruleId={rule} size="sm" />
        {meta && (
          <span className="xt-dash-extra-meta" title={meta}>
            {meta}
          </span>
        )}
      </header>
      <details
        className="xt-dash-extra-details"
        open={expanded.has(title)}
        onToggle={(event) => toggle(title, event.currentTarget.open)}
      >
        <summary>{summary}</summary>
        <div className="xt-dash-extra-body">{children}</div>
      </details>
    </section>
  );
}

function tokenMeta(report: DashboardMetrics) {
  const total = report.tokens.counters.total_tokens;
  // Coverage denominator: all sessions in range, including those without selected usage.
  const { measured, sessions } = report.usage_coverage.total;
  return `${total === null ? 'total unmeasured' : `${formatTokens(total)} recorded`} · ${measured}/${plural(sessions, 'session')} measured`;
}

function CostSection({ report }: { report: DashboardMetrics }) {
  const { cost } = report;
  const partial = cost.total_usd === null && cost.priced_observations > 0;
  return (
    <ExtraSection
      title="API-equivalent cost"
      rule="M-04"
      meta={`catalog ${cost.price_version} · as of ${cost.as_of}`}
      summary="Show cost details"
    >
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
    </ExtraSection>
  );
}

const usageText = (usage: MetricUsageSummary) =>
  `${usage.measured} of ${plural(usage.sessions, 'session')}`;
const gapsText = (usage: MetricUsageSummary) =>
  usage.gaps.length ? usage.gaps.map((gap) => usageGapText[gap]).join(', ') : null;

function CoverageSection({ report }: { report: DashboardMetrics }) {
  const { usage_coverage: usage, usage_gate_14d: gate } = report;
  const inventory = inventoryState[report.capture_inventory];
  return (
    <ExtraSection
      title="Coverage"
      rule="M-18"
      meta={`${inventory.label} · tokens measured for ${usage.total.measured}/${plural(usage.total.sessions, 'session')}`}
      summary="Show coverage details"
    >
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
            Session capture{' '}
            <StatePill tone={inventory.tone} height={20}>
              {inventory.label}
            </StatePill>
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
                    {row.incomplete_reasons.length > 0 &&
                      ` · ${row.incomplete_reasons.map((gap) => captureGapText[gap]).join(', ')}`}
                  </span>
                </li>
              ))}
            </ul>
          )}
          <p className="xt-dash-note">
            Receipts are historical: a captured session does not mean a surface is capturing now.
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
    </ExtraSection>
  );
}

function IndexLine() {
  const index = useNativeIndexStatus();
  return (
    <p className="xt-dash-note xt-dash-index" data-testid="dashboard-index">
      Native index:{' '}
      {index.data
        ? `${phaseText(index.data)} · ${freshnessText(index.data)}.`
        : index.isError
          ? 'status could not be loaded.'
          : 'reading status…'}{' '}
      Indexing does not certify capture coverage.{' '}
      <Link className="xt-dash-link" to="/settings">
        Index details
      </Link>
    </p>
  );
}
