import { useEffect, useId, useState, type CSSProperties, type ReactNode } from 'react';
import type { EnvIdentityRow } from '../../data/generated/EnvIdentityRow';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import type { HookNames } from '../../data/generated/HookNames';
import { useData } from '../../data/DataProvider';
import { Button } from '../../kit/Button';
import { DayStrip } from '../../kit/DayStrip';
import type { TimeRange } from '../../kit/TopBar';
import { DashCard } from './DashCard';
import { DetailDialog } from './DetailDialog';
import { IssueFlag } from './IssueFlag';
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
import { plural, windowLabel, zoneLabel } from './present';
import { useEnvironmentReport } from './range';

/**
 * Rows the card's identity list always shows. The list takes the card's share
 * of a taller window and shows more of its top identities; beyond what it
 * shows, it scrolls.
 */
export const MIN_ENV_ROWS = 3;

/**
 * The Environment card: observed M-17 usage, with configured facts one step
 * away. The card draws its own header so the unresolved flag sits beside the
 * title, where it costs the card no line; its body — loading, failed and
 * observed alike — sits in the layered card's inset panel.
 */
export function EnvironmentPanel({ range }: { range: TimeRange }) {
  const report = useEnvironmentReport(range);
  return (
    <DashCard
      title="Environment"
      rule="M-17"
      className="xt-env-card"
      layered
      actions={report.data && <Unresolved report={report.data} range={range} />}
    >
      {report.isPending ? (
        <p role="status" className="xt-dash-status">
          Reading environment usage for the last {range}…
        </p>
      ) : report.isError ? (
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
      ) : (
        <div className="xt-env" aria-busy={report.isFetching || undefined}>
          <EnvironmentBody report={report.data} range={range} />
        </div>
      )}
    </DashCard>
  );
}

function EnvironmentBody({ report, range }: { report: EnvironmentMetrics; range: TimeRange }) {
  const { identities } = report;
  const top = identities.slice(0, TOP_IDENTITIES);
  return (
    <>
      {top.length === 0 ? (
        <p className="xt-dash-line" data-testid="environment-empty">
          No non-built-in tool calls were observed in the last {range} or the fixed 14 local days.
        </p>
      ) : (
        <IdentityTable report={report} rows={top} total={identities.length} range={range} />
      )}
      <div className="xt-env-actions">
        {/* With no strip to draw, the strips' scope and legend keep their own line. */}
        {top.length === 0 && <StripLegend report={report} />}
        <p className="xt-env-links">
          {identities.length > 0 && <AllObserved report={report} range={range} />}
          <ConfiguredDetails report={report} />
        </p>
      </div>
    </>
  );
}

/**
 * The card's identity list under one column header that shares its tracks:
 * the tool (name, kind and host), the fixed 14-day strip, then the
 * selected-range calls at the right edge. The header stays above the rows
 * that scroll and is not one of them; the calls track is as wide as the
 * longest count shown, in the row font's own character width, so the header
 * and every row lay out the same columns without a table's roles around the
 * list. The list is described by the header for assistive technology.
 */
function IdentityTable({
  report,
  rows,
  total,
  range,
}: {
  report: EnvironmentMetrics;
  rows: readonly EnvIdentityRow[];
  total: number;
  range: TimeRange;
}) {
  const headId = useId();
  const calls = rows.reduce(
    (widest, row) => Math.max(widest, callsText(row.identity, row.calls).length),
    0,
  );
  return (
    <div className="xt-env-table" style={{ '--calls': calls } as CSSProperties}>
      <div className="xt-env-head" id={headId} data-testid="environment-columns">
        <span className="xt-dash-label" title="Tool name, kind and host">
          Tool · kind<span className="sr-only"> · host</span>
        </span>
        <StripLegend report={report} header />
        <span
          className="xt-dash-label xt-env-head-calls"
          title={`Calls in the last ${range}, the selected range`}
        >
          Calls<span className="sr-only"> · last {range}</span>
        </span>
      </div>
      <IdentityList
        report={report}
        rows={rows}
        label={`Most-called identities, top ${rows.length} of ${total}`}
        describedBy={headId}
        compact
      />
    </div>
  );
}

/**
 * The strips' key: every strip is the same fixed 14 local days whatever range is selected, so
 * that scope stays visible as the strip column's own header, which is the control that opens
 * its dates, zone, shade steps and the observed-only limit over the page, so the card keeps its
 * height. With no strip to draw the same control sits on its own line above the dialog controls.
 */
function StripLegend({ report, header = false }: { report: EnvironmentMetrics; header?: boolean }) {
  const [low, high] = STRIP_THRESHOLDS;
  return (
    <DetailDialog
      trigger={header ? '14 days' : 'Activity strips · fixed 14 local days'}
      label={header ? '14 days · activity strips, fixed 14 local days' : undefined}
      title="Activity strips"
      className="xt-env-modal"
      triggerClassName={header ? 'xt-dash-label xt-env-head-legend' : 'xt-env-legend'}
      testId="environment-strip-legend"
    >
      <p className="xt-dash-note xt-env-note" data-testid="environment-strip-note">
        Strips: fixed 14 local days {windowLabel(report.strip_window)},{' '}
        {zoneLabel(report.strip_window)}, any range; shades at 1, {low}, {high}. Observed use only:
        installed components are unknown. Built-in tools are excluded.
      </p>
      {/* The shades a strip day can take, each beside the call count it starts at; read aloud as
          one phrase rather than as swatches. */}
      <p className="xt-dash-note xt-env-legend-scale">
        Shades
        <span aria-hidden="true">
          {([1, low, high] as const).map((step, index) => (
            <span key={step} className="xt-env-legend-step">
              <span className="xt-env-legend-swatch" data-level={index + 1} />
              {step}
              {index === 2 && '+'}
            </span>
          ))}
        </span>
        <span className="sr-only">
          {' '}
          at 1, {low} and {high} or more calls a day
        </span>
      </p>
    </DetailDialog>
  );
}

/**
 * The card's unresolved attribution, as the triangle at the end of its header: the count and each
 * host and reason read on hover and focus, and open whole from the triangle. The count is stated
 * on its own: it includes untimed observations that no selected range holds, so it is never shown
 * as a part of the selected range's observed calls.
 */
function Unresolved({ report, range }: { report: EnvironmentMetrics; range: TimeRange }) {
  const calls = report.totals.selected_unresolved_calls;
  const items = report.selected.unresolved.filter((item) => item.calls > 0);
  if (calls === 0 && items.length === 0) return null;
  const ranged = items.filter((item) => !isUntimed(item.reason));
  const untimed = items.filter((item) => isUntimed(item.reason));
  const headline = `${plural(calls, 'unresolved observation')}${untimed.length > 0 ? ', including untimed' : ''}`;
  return (
    <IssueFlag
      label={`Unresolved attribution: ${headline}`}
      title="Unresolved attribution"
      testId="environment-unresolved"
      gist={
        <>
          <span>
            <strong>{headline}</strong>: cannot be assigned to a named component. Counts only; none
            is a share of the observed calls.
          </span>
          {ranged.length > 0 && <ReasonGist title={`In the last ${range}`} items={ranged} />}
          {untimed.length > 0 && (
            <ReasonGist title="Untimed, in no selected range" items={untimed} />
          )}
        </>
      }
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
            Observations without a timestamp cannot be assigned to any selected range. They are not
            part of the last {range}&rsquo;s observed calls.
          </p>
          <ReasonList items={untimed} label="Untimed observations" />
        </section>
      )}
    </IssueFlag>
  );
}

/** One scope's hosts and reasons in the triangle's tooltip, in the dialog's wording. */
function ReasonGist({
  title,
  items,
}: {
  title: string;
  items: EnvironmentMetrics['selected']['unresolved'];
}) {
  return (
    <span className="xt-rule-context">
      {title}:{' '}
      {items.map((item) => `${item.host} · ${unresolvedText[item.reason](item.calls)}`).join('; ')}
    </span>
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
  describedBy,
  compact = false,
}: {
  report: EnvironmentMetrics;
  rows: readonly EnvIdentityRow[];
  label: string;
  /** The column header the card list sits under. */
  describedBy?: string;
  /** One 24px line per identity in the card; the dialog adds kind, host and surface context. */
  compact?: boolean;
}) {
  // Both lists are bounded and scroll, so each takes focus for keyboard scrolling.
  return (
    <ol
      className="xt-env-list"
      data-compact={compact || undefined}
      aria-label={label}
      aria-describedby={describedBy}
      tabIndex={0}
      style={
        compact
          ? ({
              '--rows': Math.min(rows.length, MIN_ENV_ROWS),
              '--count': rows.length,
            } as CSSProperties)
          : undefined
      }
    >
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
  const name = (
    <span className="xt-env-name">
      {text.name}
      {text.note && <small> · {text.note}</small>}
    </span>
  );
  const strip = (
    <DayStrip
      label={`${text.name}, ${row.host}: ${callsText(identity, row.strip_calls)} per local day, last 14 days`}
      thresholds={STRIP_THRESHOLDS}
      days={row.strip.map((day) => ({ label: dayLabel(day.date), value: day.calls }))}
    />
  );
  const calls = <span className="xt-env-calls">{callsText(identity, row.calls)}</span>;
  if (compact) {
    // The card row, in the header's column order: the tool — its name, then its kind, then its
    // host as a muted suffix that gives way first when the line is short — the strip, then the
    // calls at the right edge. The full name, note, kind and context are the tool's title and,
    // visibly, in the observed dialog.
    const full = [text.name, text.note, badge.text, context].filter(Boolean).join(' · ');
    return (
      <li className="xt-env-row" data-hook-summary={isHookSummary(identity) || undefined}>
        <span className="xt-env-tool" title={full}>
          {name}
          {kind}
          <span className="xt-env-host"> · {row.host}</span>
        </span>
        {strip}
        {calls}
      </li>
    );
  }
  return (
    <li className="xt-env-row" data-hook-summary={isHookSummary(identity) || undefined}>
      {name}
      {calls}
      <span className="xt-env-context">
        {kind} {context}
      </span>
      {strip}
    </li>
  );
}

/** A dialog over the kit Modal from a compact text control; the trigger gets focus back on close. */
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
  return (
    <DetailDialog trigger={trigger} label={label} title={title} className="xt-env-modal">
      {children}
    </DetailDialog>
  );
}

function AllObserved({ report, range }: { report: EnvironmentMetrics; range: TimeRange }) {
  const count = report.identities.length;
  const { source } = useData();
  const [open, setOpen] = useState(false);
  const [names, setNames] = useState<HookNames | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const hook = report.identities.some(
    (row) => row.host === 'claude' && isHookSummary(row.identity) && row.calls > 0,
  );
  useEffect(() => {
    if (!open || !hook || !source.hookNames) {
      setNames(null);
      setLoading(false);
      return;
    }
    let current = true;
    const readId = `environment-hooks-${Date.now()}-${Math.random()}`;
    setNames(null);
    setFailed(false);
    setLoading(true);
    void source.hookNames
      .read(report.window.days, report.window.end_ms, readId)
      .then((result) => {
        if (current) setNames(result);
      })
      .catch(() => {
        if (current) setFailed(true);
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
      void source.hookNames?.cancel(readId);
    };
  }, [open, hook, source, report.window.days, report.window.end_ms]);
  return (
    <DetailDialog
      trigger={`Observed · ${count}`}
      label={`Observed identities · ${count}`}
      title={`Observed identities · last ${range}`}
      className="xt-env-modal"
      open={open}
      onOpenChange={setOpen}
    >
      <p className="xt-dash-note">
        Every identity with a call in the last {range} or the fixed 14 local days, in report order,
        with its kind, host and surface. Built-in tools are excluded.
      </p>
      <IdentityList report={report} rows={report.identities} label={`All ${count} observed`} />
      {hook && source.hookNames && (
        <section aria-label="Names found in saved summaries">
          <h3 className="xt-dash-label">Names found in saved summaries</h3>
          {loading && <p role="status">Reading saved hook summaries…</p>}
          {failed && <p role="alert">Saved hook names could not be read.</p>}
          {names && (
            <>
              <p className="xt-dash-note">
                Checked {names.checked_summaries} of {names.requested_summaries} saved summaries.
                {names.unavailable_summaries > 0 &&
                  ` Names unavailable for ${names.unavailable_summaries} summaries.`}
                {names.summaries_with_unnamed_commands > 0 &&
                  ` ${names.summaries_with_unnamed_commands} checked ${names.summaries_with_unnamed_commands === 1 ? 'summary also contains' : 'summaries also contain'} unnamed commands.`}
              </p>
              {names.labels.length > 0 ? (
                <ul className="xt-dash-list" aria-label="Script names in saved summaries">
                  {names.labels.map((label) => (
                    <li key={label.script_basename}>
                      <span>
                        {label.display_label} · {label.script_basename}
                      </span>
                      <span>{label.summaries_mentioning} summaries mentioning this name</span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="xt-dash-note">No approved names found in checked summaries.</p>
              )}
            </>
          )}
          <p className="xt-dash-note">
            A saved summary describes commands; it does not prove that a hook ran or match a
            configured component.
          </p>
        </section>
      )}
    </DetailDialog>
  );
}

function ConfiguredDetails({ report }: { report: EnvironmentMetrics }) {
  const { configured, sources, cache, roots } = report;
  return (
    <Details
      trigger={`Configured · ${configured.length}`}
      label={`Configured components · ${configured.length}`}
      title="Configured components"
    >
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
