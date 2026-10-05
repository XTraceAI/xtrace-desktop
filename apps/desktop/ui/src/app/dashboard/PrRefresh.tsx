import { useQuery } from '@tanstack/react-query';
import { useId, useRef, useState } from 'react';
import { useData, usePrRefresh } from '../../data/DataProvider';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricPrFreshnessSummary } from '../../data/generated/MetricPrFreshnessSummary';
import type { PrRefreshReport } from '../../data/generated/PrRefreshReport';
import type { PrRow } from '../../data/generated/PrRow';
import { queryKeys } from '../../data/query-client';
import { Button } from '../../kit/Button';
import { Modal, ModalClose } from '../../kit/Modal';
import { REFRESH_LIMIT, outcomeText, prFreshnessText, rowStatusText } from './pr-effort';
import { clockTime, plural } from './present';

const DISCLOSURE = [
  'Refresh asks GitHub for each selected pull request’s title, state, merge time, size and head branch by running gh pr view with the GitHub CLI sign-in you already have. It only reads: nothing is written to GitHub, and no transcript content leaves this computer.',
  `Nothing refreshes on its own; only the Refresh button starts a batch. A batch takes up to ${REFRESH_LIMIT} pull requests, one at a time, and stops after 120 seconds, each request after 30. A failed refresh keeps the facts already stored and marks them stale.`,
];

const stateText = (row: PrRow, window: DashboardWindow) =>
  row.state === null
    ? 'state not fetched'
    : row.state === 'merged'
      ? row.merged_at
        ? `merged ${clockTime(Date.parse(row.merged_at), window)}`
        : 'merged, time unknown'
      : row.state;

function reportText(report: PrRefreshReport) {
  const parts = [
    `${report.succeeded} refreshed`,
    `${report.failed} failed`,
    `${report.skipped} skipped`,
  ];
  if (report.unrecorded > 0) parts.push(`${report.unrecorded} not stored`);
  return `Requested ${report.requested}: ${parts.join(', ')}.${
    report.cancelled ? ' The batch was cancelled.' : ''
  } ${report.committed ? 'Stored facts changed; the Dashboard reads them again.' : 'No stored facts changed.'}`;
}

/**
 * The refresh status the card keeps in view under its rows: which confirmed
 * links have cached facts, and that a batch is under way when one is. The
 * batch belongs to the source runtime, so the line reads it wherever the
 * control that started it sits.
 */
export function PrRefreshStatus({
  freshness,
  window,
}: {
  freshness: MetricPrFreshnessSummary;
  window: DashboardWindow;
}) {
  const [operation] = usePrRefresh();
  return (
    <p className="xt-dash-note xt-pr-refresh-line" data-testid="pr-refresh">
      <span>{prFreshnessText(freshness, window)}</span>
      {operation.phase === 'running' && (
        <>
          {' '}
          <span role="status">Refreshing pull-request facts…</span>
        </>
      )}
    </p>
  );
}

/**
 * A compact manual refresh of indexed pull requests' cached GitHub facts. It
 * lists what storage holds only when opened, and only the Refresh button
 * starts a batch: nothing refreshes on load, on focus or on a timer. The batch
 * itself belongs to the source runtime (`usePrRefresh`), so its progress,
 * Cancel and result survive this control unmounting; only the dialog and the
 * selection are this control's. The control sits in the card's header, so
 * the card keeps its height for its rows.
 */
export function PrRefresh({ window }: { window: DashboardWindow }) {
  const { source } = useData();
  const [operation, refresh] = usePrRefresh();
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const button = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const list = useQuery({
    queryKey: queryKeys.pullRequests,
    queryFn: () => source.pullRequests(),
    enabled: open && source.kind !== 'preview',
  });
  const running = operation.phase === 'running';
  const cancel = operation.phase === 'idle' ? 'none' : operation.cancel;
  const rows = list.data?.rows ?? [];
  // Only identifiers the current list still holds are counted or sent.
  const chosen = rows.map((row) => row.pull_request.id).filter((id) => selected.has(id));
  const toggle = (id: number) =>
    setSelected((current) => {
      // Drop anything the current list no longer holds before applying the bound.
      const listed = new Set(rows.map((row) => row.pull_request.id));
      const next = new Set([...current].filter((held) => listed.has(held)));
      if (next.has(id)) next.delete(id);
      else if (next.size < REFRESH_LIMIT) next.add(id);
      return next;
    });
  const openDialog = () => {
    // A reopened dialog starts from a fresh selection; the last batch's report
    // stays until another batch runs.
    setSelected(new Set());
    setOpen(true);
  };
  const report = operation.phase === 'done' ? operation.report : undefined;
  const outcomes = new Map(report?.rows.map((row) => [row.id, row]) ?? []);
  return (
    <>
      <Button ref={button} variant="outline" height={28} onClick={openDialog}>
        Refresh PR facts…
      </Button>
      <Modal
        open={open}
        onOpenChange={setOpen}
        returnFocusRef={button}
        aria-labelledby={titleId}
        className="xt-env-modal xt-pr-refresh-modal"
      >
        <header className="xt-env-modal-header">
          <h2 id={titleId}>Refresh pull-request facts</h2>
          <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
            Close
          </ModalClose>
        </header>
        {DISCLOSURE.map((text) => (
          <p key={text} className="xt-dash-note xt-env-lead">
            {text}
          </p>
        ))}
        {list.isPending ? (
          <p role="status" className="xt-dash-line">
            Reading indexed pull requests…
          </p>
        ) : list.isError ? (
          <p role="alert" className="xt-dash-line">
            Indexed pull requests could not be read.{' '}
            <Button variant="outline" height={28} onClick={() => void list.refetch()}>
              Retry
            </Button>
          </p>
        ) : rows.length === 0 ? (
          <p className="xt-dash-line">
            No session links a pull request yet, so there is nothing to refresh.
          </p>
        ) : (
          <fieldset className="xt-pr-refresh-list" disabled={running}>
            <legend className="xt-dash-label">
              Indexed pull requests · {chosen.length} of {REFRESH_LIMIT} selected
            </legend>
            <ul className="xt-dash-list" aria-label="Indexed pull requests">
              {rows.map((row) => {
                const id = row.pull_request.id;
                const checked = selected.has(id);
                const outcome = outcomes.get(id);
                return (
                  <li key={id}>
                    <label className="xt-pr-refresh-option">
                      <input
                        type="checkbox"
                        tabIndex={0}
                        checked={checked}
                        disabled={!checked && chosen.length >= REFRESH_LIMIT}
                        onChange={() => toggle(id)}
                      />
                      <span>
                        {row.pull_request.repository}#{row.pull_request.number}
                        {row.title ? ` · ${row.title}` : ' · title not fetched'}
                      </span>
                    </label>
                    <span>
                      {stateText(row, window)} · {rowStatusText(row, window)} ·{' '}
                      {plural(row.linked_sessions, 'linked session')}
                      {outcome && (
                        <span className="xt-pr-refresh-outcome">
                          {' '}
                          · {outcomeText(outcome.outcome)}
                        </span>
                      )}
                    </span>
                  </li>
                );
              })}
            </ul>
            {chosen.length >= REFRESH_LIMIT && (
              <p className="xt-dash-note">
                {REFRESH_LIMIT} is the most one batch refreshes; clear one to choose another.
              </p>
            )}
          </fieldset>
        )}
        <div className="xt-pr-refresh-actions">
          <Button
            variant="accent"
            height={28}
            disabled={running || chosen.length === 0}
            onClick={() => refresh.start(chosen)}
          >
            {running ? 'Refreshing…' : `Refresh ${plural(chosen.length, 'pull request')}`}
          </Button>
          {running && (
            <Button
              variant="outline"
              height={28}
              disabled={cancel === 'asked'}
              onClick={() => refresh.cancel()}
            >
              {cancel === 'asked' ? 'Cancelling…' : 'Cancel'}
            </Button>
          )}
        </div>
        {cancel === 'none-running' && (
          <p className="xt-dash-note" role="status">
            No refresh was running when the cancel arrived.
          </p>
        )}
        {cancel === 'failed' && (
          <p className="xt-dash-note" role="alert">
            The cancel request could not be delivered.
          </p>
        )}
        {operation.phase === 'failed' && (
          <p role="alert" className="xt-dash-line" data-testid="pr-refresh-error">
            The refresh did not run: {operation.error}
          </p>
        )}
        {report && !running && (
          <p role="status" className="xt-dash-line" data-testid="pr-refresh-report">
            {reportText(report)}
          </p>
        )}
      </Modal>
    </>
  );
}
