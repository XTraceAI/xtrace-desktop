import { useId, useRef, useState, type ReactNode } from 'react';
import type { EnvIdentityRow } from '../../data/generated/EnvIdentityRow';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import { StatePill } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import { DayStrip } from '../../kit/DayStrip';
import { Modal, ModalClose } from '../../kit/Modal';
import type { TimeRange } from '../../kit/TopBar';
import {
  cacheSourceText,
  callsText,
  componentKindText,
  configSourceText,
  dayLabel,
  enabledText,
  configuredEmptyText,
  identityText,
  isHookSummary,
  isUntimed,
  kindBadge,
  rootStateText,
  rowSurfaces,
  scopeText,
  statusText,
  STRIP_THRESHOLDS,
  TOP_IDENTITIES,
  unresolvedText,
} from './environment';
import { inventoryState, plural, windowLabel, zoneLabel } from './present';
import { useEnvironmentReport } from './range';

/** The Environment card body: observed component usage, with configured facts one step away. */
export function EnvironmentPanel({ range }: { range: TimeRange }) {
  const report = useEnvironmentReport(range);
  if (report.isPending)
    return (
      <p role="status" className="xt-dash-status">
        Reading environment usage for the last {range}…
      </p>
    );
  if (report.isError)
    return (
      <p role="alert" className="xt-dash-status">
        Environment usage could not be loaded.{' '}
        <Button
          variant="outline"
          height={28}
          aria-label="Retry environment usage"
          disabled={report.isFetching}
          onClick={() => void report.refetch()}
        >
          Retry
        </Button>
      </p>
    );
  return (
    <div className="xt-env" aria-busy={report.isFetching || undefined}>
      <EnvironmentBody report={report.data} range={range} />
    </div>
  );
}

function EnvironmentBody({ report, range }: { report: EnvironmentMetrics; range: TimeRange }) {
  const { totals, identities } = report;
  const top = identities.slice(0, TOP_IDENTITIES);
  const inventory = inventoryState[report.inventory];
  return (
    <>
      <p className="xt-env-summary" data-testid="environment-summary">
        <span>
          {plural(totals.selected_calls, 'observed call')} · last {range} ·{' '}
          {plural(totals.identities, 'identity', 'identities')} in either window
        </span>
        <StatePill tone={inventory.tone} height={20}>
          {inventory.label}
        </StatePill>
      </p>
      <Unresolved report={report} range={range} />
      {top.length === 0 ? (
        <p className="xt-dash-line" data-testid="environment-empty">
          No tool calls were observed in the last {range} or the fixed 14 local days.
        </p>
      ) : (
        <IdentityList
          report={report}
          rows={top}
          label={`Most-called identities, top ${top.length} of ${identities.length}`}
          compact
        />
      )}
      <p className="xt-dash-note xt-env-note" data-testid="environment-strip-note">
        Strips: fixed 14 local days {windowLabel(report.strip_window)},{' '}
        {zoneLabel(report.strip_window)}, any range; shades at 1, {STRIP_THRESHOLDS[0]},{' '}
        {STRIP_THRESHOLDS[1]}. Observed use only: installed components are unknown.
      </p>
      <div className="xt-env-actions">
        {identities.length > 0 && <AllObserved report={report} range={range} />}
        <ConfiguredDetails report={report} />
      </div>
    </>
  );
}

/**
 * One line in the card; each host and reason is one step away. The count is stated on its own:
 * it includes untimed observations that no selected range holds, so it is never shown as a part of
 * the selected range's observed calls.
 */
function Unresolved({ report, range }: { report: EnvironmentMetrics; range: TimeRange }) {
  const calls = report.totals.selected_unresolved_calls;
  const items = report.selected.unresolved.filter((item) => item.calls > 0);
  if (calls === 0 && items.length === 0) return null;
  const ranged = items.filter((item) => !isUntimed(item.reason));
  const untimed = items.filter((item) => isUntimed(item.reason));
  return (
    <div
      className="xt-env-unresolved xt-control-tone"
      data-tone="warning"
      role="note"
      aria-label="Unresolved attribution"
      data-testid="environment-unresolved"
    >
      <p title="Unresolved observations cannot be assigned to a named component">
        <strong>{plural(calls, 'unresolved observation')}</strong>
        {untimed.length > 0 && ', including untimed'}
      </p>
      <Details
        trigger="Details"
        label="Unresolved attribution details"
        title="Unresolved attribution"
      >
        <p className="xt-dash-note xt-env-lead">
          {plural(calls, 'observation')} cannot be assigned to a named component. These are counts
          only; none is a share of the observed calls.
        </p>
        {ranged.length > 0 && (
          <section aria-labelledby="xt-env-unresolved-ranged">
            <h3 id="xt-env-unresolved-ranged" className="xt-dash-label">
              In the last {range}
            </h3>
            <ReasonList items={ranged} label={`Unresolved in the last ${range}`} />
          </section>
        )}
        {untimed.length > 0 && (
          <section aria-labelledby="xt-env-unresolved-untimed">
            <h3 id="xt-env-unresolved-untimed" className="xt-dash-label">
              Untimed
            </h3>
            <p className="xt-dash-note">
              Observations without a timestamp cannot be assigned to any selected range. They are
              not part of the last {range}&rsquo;s observed calls.
            </p>
            <ReasonList items={untimed} label="Untimed observations" />
          </section>
        )}
      </Details>
    </div>
  );
}

function ReasonList({
  items,
  label,
}: {
  items: EnvironmentMetrics['selected']['unresolved'];
  label: string;
}) {
  return (
    <ul className="xt-dash-list" aria-label={label}>
      {items.map((item) => (
        <li key={`${item.host}:${item.reason}`}>
          {item.host} · {unresolvedText[item.reason](item.calls)}
        </li>
      ))}
    </ul>
  );
}

function IdentityList({
  report,
  rows,
  label,
  compact = false,
}: {
  report: EnvironmentMetrics;
  rows: readonly EnvIdentityRow[];
  label: string;
  /** One 24px line per identity in the card; the dialog adds kind, host and surface context. */
  compact?: boolean;
}) {
  // Both lists are bounded and scroll, so each takes focus for keyboard scrolling.
  return (
    <ol className="xt-env-list" data-compact={compact || undefined} aria-label={label} tabIndex={0}>
      {rows.map((row) => (
        <IdentityItem key={row.order} report={report} row={row} compact={compact} />
      ))}
    </ol>
  );
}

function IdentityItem({
  report,
  row,
  compact,
}: {
  report: EnvironmentMetrics;
  row: EnvIdentityRow;
  compact: boolean;
}) {
  const { identity } = row;
  const badge = kindBadge(identity.kind);
  const text = identityText(identity);
  const surfaces = rowSurfaces(report, row);
  const context = surfaces.length > 0 ? surfaces.join(', ') : `${row.host} · no surface in range`;
  const kind = (
    <span className="xt-kind xt-control-tone" data-tone={badge.tone}>
      {badge.text}
    </span>
  );
  const full = [text.name, text.note, badge.text, context].filter(Boolean).join(' · ');
  return (
    <li className="xt-env-row" data-hook-summary={isHookSummary(identity) || undefined}>
      {compact && kind}
      <span className="xt-env-name" title={compact ? full : undefined}>
        {text.name}
        {text.note && <small> · {text.note}</small>}
        {compact && <span className="xt-env-host"> · {row.host}</span>}
      </span>
      <span className="xt-env-calls">{callsText(identity, row.calls)}</span>
      {!compact && (
        <span className="xt-env-context">
          {kind} {context}
        </span>
      )}
      <DayStrip
        label={`${text.name}, ${row.host}: ${callsText(identity, row.strip_calls)} across 14 local days; daily counts`}
        thresholds={STRIP_THRESHOLDS}
        days={row.strip.map((day) => ({ label: dayLabel(day.date), value: day.calls }))}
      />
    </li>
  );
}

/** A dialog over the kit Modal; the trigger gets focus back on close. */
function Details({
  trigger,
  label,
  title,
  children,
}: {
  trigger: string;
  /** The button's accessible name when its visible text is shorter. */
  label?: string;
  title: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const id = useId();
  return (
    <>
      <Button
        ref={button}
        variant="outline"
        height={28}
        aria-label={label}
        onClick={() => setOpen(true)}
      >
        {trigger}
      </Button>
      <Modal
        open={open}
        onOpenChange={setOpen}
        returnFocusRef={button}
        aria-labelledby={id}
        className="xt-env-modal"
      >
        <header className="xt-env-modal-header">
          <h2 id={id}>{title}</h2>
          <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
            Close
          </ModalClose>
        </header>
        {children}
      </Modal>
    </>
  );
}

function AllObserved({ report, range }: { report: EnvironmentMetrics; range: TimeRange }) {
  const count = report.identities.length;
  return (
    <Details trigger={`Observed identities · ${count}`} title="Observed identities">
      <p className="xt-dash-note">
        Every identity with a call in the last {range} or the fixed 14 local days, in report order,
        with its kind, host and surface.
      </p>
      <IdentityList report={report} rows={report.identities} label={`All ${count} observed`} />
    </Details>
  );
}

function ConfiguredDetails({ report }: { report: EnvironmentMetrics }) {
  const { configured, sources, cache, roots } = report;
  return (
    <Details trigger={`Configured components · ${configured.length}`} title="Configured components">
      <p className="xt-dash-note xt-env-lead">
        Named in supported configuration files. Being configured does not show that a component is
        installed, can be called, or has been called.
      </p>
      <section aria-labelledby="xt-env-configured">
        <h3 id="xt-env-configured" className="xt-dash-label">
          Configured · {configured.length}
        </h3>
        {configured.length === 0 ? (
          <p className="xt-dash-line" data-testid="environment-configured-empty">
            {configuredEmptyText(sources)}
          </p>
        ) : (
          <ul className="xt-dash-list" aria-label="Configured components">
            {configured.map((item) => (
              <li key={`${item.host}:${item.source}:${item.name}`}>
                <span>
                  {item.host} · {componentKindText[item.kind]} · {item.name}
                </span>
                <span>
                  {configSourceText[item.source]} · {scopeText(item.scope)} ·{' '}
                  {enabledText(item.enabled)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
      <section aria-labelledby="xt-env-sources">
        <h3 id="xt-env-sources" className="xt-dash-label">
          Sources
        </h3>
        <ul className="xt-dash-list" aria-label="Configuration sources">
          {sources.map((item) => (
            <li key={`${item.source}:${item.scope}:${item.root_index}`}>
              <span>
                {item.host} · {configSourceText[item.source]} ·{' '}
                {scopeText(item.scope, item.root_index)}
              </span>
              <span>{statusText(item.status, item.source)}</span>
            </li>
          ))}
        </ul>
        {roots.length > 0 && (
          <p className="xt-dash-note">
            Roots:{' '}
            {roots
              .map((root) => `${scopeText(root.scope, root.index)} ${rootStateText[root.state]}`)
              .join(' · ')}
            .
          </p>
        )}
      </section>
      <section aria-labelledby="xt-env-cache">
        <h3 id="xt-env-cache" className="xt-dash-label">
          Cache only
        </h3>
        <p className="xt-dash-note">
          Plugin caches hold downloaded files. They are not configured components.
        </p>
        {cache.length > 0 && (
          <ul className="xt-dash-list" aria-label="Plugin cache observations">
            {cache.map((item) => (
              <li key={`${item.source}:${item.scope}:${item.root_index}`}>
                <span>
                  {item.host} · {cacheSourceText[item.source]} ·{' '}
                  {scopeText(item.scope, item.root_index)}
                </span>
                <span>{statusText(item.status, item.source)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </Details>
  );
}
