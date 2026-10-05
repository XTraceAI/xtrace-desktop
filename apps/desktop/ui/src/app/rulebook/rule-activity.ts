/**
 * One Rulebook view's read of recorded rule activity, for exactly as long as
 * the view is open.
 *
 * Deliberately not a cached query: the answer is one bounded snapshot of a
 * local source, anchored when the native side admitted it, and a cache would
 * show an old window under a new visit. So this is a plain effect with an
 * explicit lifecycle:
 *
 * - **One read per activation.** The view that mounts this asks once; nothing
 *   reads again on an event, focus, visibility or timer.
 * - **Refresh only after the last read ended.** A read in flight is never
 *   joined by another from this view; a `busy` answer from the native side is
 *   shown with a manual Refresh and never retried here.
 * - **Leaving cancels.** Unmounting cancels the read by its own name, and an
 *   answer arriving after that is dropped. The local generation decides what
 *   is current, not the answer's echoed name, which a fixture need not echo.
 * - **Cancel is a request.** The view shows it stopping until the read itself
 *   ends, then says it was cancelled; nothing it may have counted is shown.
 * - **Only a loaded answer holds data.** Every other state replaces it.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { useData } from '../../data/DataProvider';
import type { RuleActivityLoaded } from '../../data/generated/RuleActivityLoaded';
import type { RuleActivityModeCounts } from '../../data/generated/RuleActivityModeCounts';
import type { RuleActivityPart } from '../../data/generated/RuleActivityPart';
import type { RuleActivityResult } from '../../data/generated/RuleActivityResult';
import type { RuleActivityUnavailableReason } from '../../data/generated/RuleActivityUnavailableReason';

export type RuleActivity =
  /** This window has no source to read, so no read is made. */
  | { readonly phase: 'unsupported' }
  | { readonly phase: 'reading'; readonly stopping: boolean }
  /** The reader asked to stop, and the read has ended. */
  | { readonly phase: 'cancelled' }
  /** The command itself failed, not the source. */
  | { readonly phase: 'error' }
  | { readonly phase: 'answered'; readonly result: RuleActivityResult };

export type Loaded = Extract<RuleActivityResult, { state: 'loaded' }>;

let reads = 0;
/** A name for one read, never reused within this window. */
function nextReadId(): string {
  reads += 1;
  const unique =
    typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `rule-activity-${reads}-${unique}`;
}

type Flight = { readonly id: string; started: boolean; settled: boolean; stopping: boolean };

export function useRuleActivity(): {
  activity: RuleActivity;
  /** Start a new read; ignored while one is still in flight. */
  refresh: () => void;
  /** Ask the read in flight to stop; ignored when none is, or it is already stopping. */
  cancel: () => void;
} {
  const { source } = useData();
  const controls = source.ruleActivity;
  const [activity, setActivity] = useState<RuleActivity>(
    controls ? { phase: 'reading', stopping: false } : { phase: 'unsupported' },
  );
  const [attempt, setAttempt] = useState(0);
  const flight = useRef<Flight | null>(null);
  /** A Refresh asked for and not yet begun, so a second press asks nothing. */
  const requested = useRef(false);

  useEffect(() => {
    if (!controls) return;
    const read: Flight = { id: nextReadId(), started: false, settled: false, stopping: false };
    let open = true;
    requested.current = false;
    flight.current = read;
    setActivity({ phase: 'reading', stopping: false });
    const settle = (next: RuleActivity) => {
      read.settled = true;
      if (!open) return;
      flight.current = null;
      setActivity(read.stopping ? { phase: 'cancelled' } : next);
    };
    // Started a task later, so a mount that ends in the same task (a
    // development double mount) sends nothing to cancel.
    void Promise.resolve().then(async () => {
      if (!open) return;
      if (read.stopping) {
        settle({ phase: 'cancelled' });
        return;
      }
      read.started = true;
      let answer: RuleActivity;
      try {
        answer = { phase: 'answered', result: await controls.read(read.id) };
      } catch {
        answer = { phase: 'error' };
      }
      settle(answer);
    });
    return () => {
      open = false;
      if (flight.current === read) flight.current = null;
      if (read.started && !read.settled && !read.stopping)
        void controls.cancel(read.id).catch(() => {
          // An undelivered cancel leaves a bounded read running and an answer
          // nobody reads; there is nothing to tell the reader.
        });
    };
  }, [controls, attempt]);

  const refresh = useCallback(() => {
    if (!controls || flight.current || requested.current) return;
    requested.current = true;
    setAttempt((value) => value + 1);
  }, [controls]);
  const cancel = useCallback(() => {
    const read = flight.current;
    if (!controls || !read || read.stopping) return;
    read.stopping = true;
    setActivity({ phase: 'reading', stopping: true });
    if (read.started)
      void controls.cancel(read.id).catch(() => {
        // The read still ends on its own bound; the view keeps waiting for it.
      });
  }, [controls]);
  return { activity, refresh, cancel };
}

/** Rows recorded as advise or gate: what this page calls recorded fires. */
export const recordedFires = (modes: RuleActivityModeCounts) => modes.advise + modes.gate;
/** Every retained in-window row, of any recorded mode. */
export const windowRows = (modes: RuleActivityModeCounts) =>
  modes.advise + modes.gate + modes.suppressed + modes.unrecognized;

/** A count as this snapshot's precision allows it to be read. */
export function bounded(value: number, loaded: RuleActivityLoaded): string {
  const text = value.toLocaleString('en-US');
  return loaded.counts.precision === 'exact' ? text : `≥ ${text}`;
}

/**
 * Why the snapshot's counts are what they are, from its coverage and quality
 * flags alone. A lower bound always carries at least one reason.
 */
export function incompleteReasons(loaded: RuleActivityLoaded): string[] {
  const { coverage, counts } = loaded;
  const { snapshot } = counts;
  const malformed = Object.values(snapshot.malformed).reduce((sum, value) => sum + value, 0);
  const reasons = [
    coverage.byte_bound_reached && 'the read stopped at its size bound',
    coverage.line_bound_reached && 'the read stopped at its line bound',
    coverage.leading_partial_dropped && 'a partial first line was skipped',
    coverage.trailing_partial_dropped && 'an unfinished last line was skipped',
    malformed > 0 &&
      `${malformed.toLocaleString('en-US')} malformed ${malformed === 1 ? 'line was' : 'lines were'} rejected`,
    snapshot.conflicted_ids > 0 &&
      `${snapshot.conflicted_ids.toLocaleString('en-US')} fire ${snapshot.conflicted_ids === 1 ? 'ID was' : 'IDs were'} recorded with differing details and left out`,
  ].filter((reason): reason is string => typeof reason === 'string');
  if (counts.precision === 'lower_bound' && reasons.length === 0)
    reasons.push('the scan did not complete cleanly');
  return reasons;
}

/** What a snapshot's counts are and are not, and how precise they are and why. */
export function snapshotNote(loaded: RuleActivityLoaded): string {
  const { window_modes: modes, precision } = loaded.counts;
  const sentences = [
    `Recorded fires are rows recorded with mode advise or gate in this window; suppressed (${bounded(modes.suppressed, loaded)}) and unrecognized-mode (${bounded(modes.unrecognized, loaded)}) rows are counted apart.`,
    'Every count is of retained recorded rows in this source snapshot, not of all rule activity, active rules or outcomes.',
    precision === 'exact'
      ? 'Counts are exact for this snapshot.'
      : `Counts are lower bounds (≥): ${incompleteReasons(loaded).join('; ')}.`,
  ];
  if (loaded.coverage.grew_after_capture)
    sentences.push('The source grew after it was captured; newer rows are not included.');
  return sentences.join(' ');
}
/** What the page says before any snapshot is held. */
export const UNREAD_NOTE =
  'Counts come only from a completed read, as retained recorded rows in one snapshot of this source: not every rule event, not active rules and not outcomes. Rule proposals and active rules are not read by this app.';

const PART: Record<RuleActivityPart, string> = {
  root: 'Its folder',
  ledger_dir: 'Its ledger folder',
  schema_marker: 'Its schema marker',
  ledger: 'Its ledger',
};
const REASON: Record<RuleActivityUnavailableReason, string> = {
  missing: 'was not found',
  unreadable: 'could not be read',
  unsafe_path: 'failed a path safety check',
  malformed_schema: 'names a schema that could not be parsed',
  unsupported_schema: 'names a schema this app does not read',
  not_configured: 'is not configured on this device',
};

/** What a view says about a state: a pill, a lead and why. */
export function describe(activity: RuleActivity): {
  pill: string;
  tone: 'meta' | 'info' | 'warning' | 'danger' | 'success';
  lead: string;
  detail: string;
} {
  if (activity.phase === 'unsupported')
    return {
      pill: 'unavailable',
      tone: 'meta',
      lead: 'Recorded rule activity is read only in the desktop app',
      detail: 'This preview has no local rulebook source, so nothing was read or counted.',
    };
  if (activity.phase === 'reading')
    return activity.stopping
      ? {
          pill: 'stopping',
          tone: 'info',
          lead: 'Stopping the read…',
          detail: 'Nothing is shown until it has stopped.',
        }
      : {
          pill: 'reading',
          tone: 'info',
          lead: 'Reading the default local rulebook source…',
          detail: 'One bounded read of the last 14 days. Nothing is shown until it completes.',
        };
  if (activity.phase === 'cancelled')
    return {
      pill: 'cancelled',
      tone: 'meta',
      lead: 'Read cancelled',
      detail: 'Nothing it may have counted is shown. Refresh to read again.',
    };
  if (activity.phase === 'error')
    return {
      pill: 'failed',
      tone: 'danger',
      lead: 'Rule activity could not be read',
      detail: 'The read did not return an answer. Refresh to try again.',
    };
  const { result } = activity;
  switch (result.state) {
    case 'loaded':
      return {
        pill: 'read',
        tone: 'success',
        lead: 'Read complete',
        detail: 'Retained recorded rows in this source snapshot.',
      };
    case 'unavailable': {
      const schema =
        result.reason === 'unsupported_schema' && result.found_version !== null
          ? ` (schema ${result.found_version})`
          : '';
      return {
        pill: 'unavailable',
        tone: 'warning',
        lead: 'Source unavailable',
        detail: `${PART[result.part]} ${REASON[result.reason]}${schema}. Nothing was counted.`,
      };
    }
    case 'source_changed':
      return {
        pill: 'changed',
        tone: 'warning',
        lead: 'The source changed during the read',
        detail: `It was ${result.change} while being read, so nothing from it is shown. Refresh to read it again.`,
      };
    case 'interrupted':
      return result.reason === 'deadline'
        ? {
            pill: 'timed out',
            tone: 'warning',
            lead: 'The read ran out of time',
            detail: 'No partial counts are shown. Refresh to try again.',
          }
        : {
            pill: 'cancelled',
            tone: 'meta',
            lead: 'Read cancelled',
            detail: 'Nothing it may have counted is shown. Refresh to read again.',
          };
    case 'busy':
      return {
        pill: 'busy',
        tone: 'warning',
        lead: 'Another rule activity read is still running',
        detail: 'Nothing is counted from this attempt. Refresh once that read has finished.',
      };
    case 'closed':
      return {
        pill: 'closed',
        tone: 'danger',
        lead: 'The rule activity reader has closed',
        detail: 'Nothing was read. Refresh to try again.',
      };
    case 'invalid_read_id':
      return {
        pill: 'refused',
        tone: 'danger',
        lead: 'The read was refused',
        detail: 'Nothing was read. Refresh to try again.',
      };
    case 'failed':
      return {
        pill: 'failed',
        tone: 'danger',
        lead: 'Rule activity could not be read',
        detail: 'The read failed before a snapshot was taken. Refresh to try again.',
      };
  }
}

type LoadedActivity = { readonly phase: 'answered'; readonly result: Loaded };
export function isLoaded(activity: RuleActivity): activity is LoadedActivity {
  return activity.phase === 'answered' && activity.result.state === 'loaded';
}
