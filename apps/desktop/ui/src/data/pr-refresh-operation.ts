import type { QueryClient } from '@tanstack/react-query';
import type { DataSource } from './DataSource';
import type { PrRefreshReport } from './generated/PrRefreshReport';
import { queryKeys } from './query-client';
import { refreshQueries } from './subscribe-invalidation';

/** Whether a cancel request reached a running batch, as far as this app knows. */
export type CancelState = 'none' | 'asked' | 'none-running' | 'failed';

/** The one manual pull-request refresh a source runtime can have at a time. */
export type PrRefreshState =
  | { phase: 'idle' }
  | { phase: 'running'; run: number; ids: readonly number[]; cancel: CancelState }
  | {
      phase: 'done';
      run: number;
      ids: readonly number[];
      cancel: CancelState;
      report: PrRefreshReport;
    }
  | { phase: 'failed'; run: number; ids: readonly number[]; cancel: CancelState; error: string };

export interface PrRefreshOperation {
  getState(): PrRefreshState;
  subscribe(listener: () => void): () => void;
  /** Starts a batch; false, and nothing is sent, while one is already running. */
  start(ids: readonly number[]): boolean;
  /** Asks the running batch to stop; nothing is sent when none is running. */
  cancel(): void;
  /** While detached (after its runtime unmounts), late answers change nothing. */
  attach(): void;
  detach(): void;
}

/** Every range's Dashboard report, and nothing else under `metrics`. */
const dashboardPrefix = queryKeys.dashboard(7).slice(0, 2);

/**
 * The manual refresh as state of the source runtime rather than of a screen:
 * a range change or leaving the Dashboard unmounts the report, while the batch,
 * its Cancel and its result stay here until the next batch starts. A committed
 * batch re-reads the Dashboard reports through `refreshQueries`, so a read that
 * began before the commit is followed by one that began after it.
 */
export function createPrRefreshOperation(
  source: DataSource,
  client: QueryClient,
): PrRefreshOperation {
  let state: PrRefreshState = { phase: 'idle' };
  let runs = 0;
  let live = true;
  const listeners = new Set<() => void>();
  const set = (next: PrRefreshState) => {
    state = next;
    listeners.forEach((listener) => listener());
  };
  /** Applies an answer only to the run it belongs to, while attached. */
  const update = (
    run: number,
    next: (current: Exclude<PrRefreshState, { phase: 'idle' }>) => PrRefreshState,
  ) => {
    if (live && state.phase !== 'idle' && state.run === run) set(next(state));
  };
  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    start(ids) {
      if (state.phase === 'running' || ids.length === 0) return false;
      const run = ++runs;
      const selection = [...ids];
      set({ phase: 'running', run, ids: selection, cancel: 'none' });
      let request: Promise<PrRefreshReport>;
      try {
        request = source.refreshPullRequests(selection);
      } catch (error) {
        request = Promise.reject(error);
      }
      request.then(
        (report) => {
          update(run, (current) => ({
            phase: 'done',
            run,
            ids: selection,
            cancel: current.cancel,
            report,
          }));
          // Committed facts change M-19, which only the Dashboard report carries;
          // the refresh event itself invalidates `prs` and `gh`.
          if (report.committed && live)
            refreshQueries(
              client,
              [dashboardPrefix],
              () => {},
              () => live,
            );
        },
        (error: unknown) =>
          update(run, (current) => ({
            phase: 'failed',
            run,
            ids: selection,
            cancel: current.cancel,
            error: error instanceof Error ? error.message : String(error),
          })),
      );
      return true;
    },
    cancel() {
      if (state.phase !== 'running' || state.cancel === 'asked') return;
      const run = state.run;
      set({ ...state, cancel: 'asked' });
      source.cancelPullRequestRefresh().then(
        (wasRunning) =>
          update(run, (current) =>
            current.cancel === 'asked' && !wasRunning
              ? { ...current, cancel: 'none-running' }
              : current,
          ),
        () =>
          update(run, (current) =>
            current.cancel === 'asked' ? { ...current, cancel: 'failed' } : current,
          ),
      );
    },
    attach() {
      live = true;
    },
    detach() {
      live = false;
    },
  };
}
