import { Tooltip } from '@base-ui/react/tooltip';
import { useQuery } from '@tanstack/react-query';
import { useId, useMemo, useRef, useState } from 'react';
import { useData, usePrRefresh } from '../../data/DataProvider';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { PrRefreshReport } from '../../data/generated/PrRefreshReport';
import type { PrRow } from '../../data/generated/PrRow';
import { queryKeys } from '../../data/query-client';
import { Button } from '../../kit/Button';
import { Modal, ModalClose } from '../../kit/Modal';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import {
  REFRESH_LIMIT,
  outcomeText,
  prAttention,
  prAttentionText,
  prFreshnessText,
  rowStatusText,
  type AutoCheck,
} from './pr-effort';
import { clockTime, plural } from './present';

const DISCLOSURE = [
  'Refresh asks GitHub for each selected pull request’s title, state, merge time, size and head branch by running gh pr view with the GitHub CLI sign-in you already have. It only reads: nothing is written to GitHub, and no transcript content leaves this computer.',
  `XTrace also checks on its own, the same way: pull requests it has not checked yet and ones that were still open, when you switch to XTrace or open the Dashboard, and about once an hour. Merged and closed ones are not checked again. Use this to check particular ones now. A batch takes up to ${REFRESH_LIMIT} pull requests, one at a time, and stops after 120 seconds, each request after 30. A failed check keeps the facts already stored and marks them stale.`,
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

const RUNNING_NAME = 'Refreshing pull-request facts…';
const RUNNING_TIP =
  'XTrace is checking the pull requests you chose on GitHub. Click to see its progress or cancel it.';

/**
 * A manual refresh of indexed pull requests' cached GitHub facts, reached
 * only when it is needed: its trigger is a small red ! in the Effort card's
 * header, drawn only while {@link prAttention} says the checks need the user,
 * and nothing at all otherwise. While a batch runs the trigger stays, quiet
 * rather than red, so its progress and Cancel stay one click away; it also
 * stays while its dialog is open. The dialog lists what storage holds only
 * when opened, and only its Refresh button starts a batch from here; the
 * native side's automatic check, which runs on its own, gives way to it. The
 * batch itself belongs to the source runtime (`usePrRefresh`), so its
 * progress, Cancel and result survive this control unmounting; only the
 * dialog and the selection are this control's. The trigger sits in the
 * card's header, so the card keeps its height for its chart.
 */
export function PrRefresh({
  window,
  tile,
  auto,
}: {
  window: DashboardWindow;
  /** The Merged PRs tile's counts, which the red ! reads as the tile does. */
  tile: MetricMergedPrs;
  auto: AutoCheck;
}) {
  const { source } = useData();
  const theme = useSurfaceTheme();
  const tipId = useId();
  const [operation, refresh] = usePrRefresh();
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const button = useRef<HTMLButtonElement>(null);
  // Always in the header, so focus has somewhere to go when the red ! is gone.
  const slot = useRef<HTMLSpanElement>(null);
  // Read when the dialog closes: the red ! when it is still there; when a
  // refresh cleared what it was for, it is gone, and focus returns to the
  // control before it in the header (Details) instead of being dropped.
  const returnFocus = useMemo(
    () => ({
      get current(): HTMLElement | null {
        if (button.current?.isConnected) return button.current;
        let before = slot.current?.previousElementSibling ?? null;
        while (before && !(before instanceof HTMLButtonElement))
          before = before.previousElementSibling;
        return before;
      },
    }),
    [],
  );
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
  const attention = prAttention(tile, auto);
  const shown = attention !== null || running || open;
  return (
    <>
      <span ref={slot} className="xt-pr-attention-slot">
        {shown && (
          <Tooltip.Root disableHoverablePopup>
            <Tooltip.Trigger
              render={
                <button
                  ref={button}
                  type="button"
                  // Explicit, as the kit's buttons are: WebKit tabs past a button without it.
                  tabIndex={0}
                  className="xt-pr-attention"
                  data-state={running ? 'running' : attention ? 'attention' : 'open'}
                  data-testid="pr-attention"
                  aria-label={
                    running
                      ? RUNNING_NAME
                      : attention
                        ? 'Pull-request checks need your attention'
                        : 'Refresh pull-request facts'
                  }
                  aria-haspopup="dialog"
                  aria-expanded={open}
                  aria-describedby={tipId}
                  onClick={openDialog}
                >
                  {running ? <RunningMark /> : <AttentionMark />}
                </button>
              }
              delay={0}
            />
            <Tooltip.Portal data-theme={theme}>
              <Tooltip.Positioner
                className="xt-rule-positioner"
                positionMethod="fixed"
                side="bottom"
                align="end"
                sideOffset={8}
                collisionPadding={8}
              >
                <Tooltip.Popup
                  id={tipId}
                  role="tooltip"
                  className="xt-rule-popover"
                  data-testid="pr-attention-tip"
                >
                  {running
                    ? RUNNING_TIP
                    : attention
                      ? prAttentionText(attention)
                      : 'Nothing needs your attention now.'}
                </Tooltip.Popup>
              </Tooltip.Positioner>
            </Tooltip.Portal>
          </Tooltip.Root>
        )}
      </span>
      <Modal
        open={open}
        onOpenChange={setOpen}
        returnFocusRef={returnFocus}
        aria-labelledby={titleId}
        className="xt-env-modal xt-pr-refresh-modal"
      >
        <header className="xt-env-modal-header">
          <h2 id={titleId}>Refresh pull-request facts</h2>
          <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
            Close
          </ModalClose>
        </header>
        {/* What used to be the card's last line: the confirmed links' status,
            then why the checks need the user, when they do. */}
        <p className="xt-dash-line" data-testid="pr-refresh">
          {prFreshnessText(tile.freshness, window)}
          {attention && !running && ` ${prAttentionText(attention, 'dialog')}`}
        </p>
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

/** A small !: a circle in the danger ink with the mark drawn through it. */
function AttentionMark() {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.2}
      strokeLinecap="round"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="12" cy="12" r="9.5" />
      <path d="M12 7v6.5M12 17v.01" />
    </svg>
  );
}

/** A quiet three-quarter ring while a batch runs: not a warning, just busy. */
function RunningMark() {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.2}
      strokeLinecap="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M12 2.5a9.5 9.5 0 1 0 9.5 9.5" />
    </svg>
  );
}
