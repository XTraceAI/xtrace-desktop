import { Tooltip } from '@base-ui/react/tooltip';
import { useQuery } from '@tanstack/react-query';
import { useCallback, useId, useMemo, useSyncExternalStore, type CSSProperties } from 'react';
import { useData } from '../data/DataProvider';
import type { SessionCompactionControls } from '../data/DataSource';
import type { CompactionCount } from '../data/generated/CompactionCount';
import type { CompactionEvent } from '../data/generated/CompactionEvent';
import type { CompactionTrigger } from '../data/generated/CompactionTrigger';
import type { InheritedCompactions } from '../data/generated/InheritedCompactions';
import { RULE_OPEN_DELAY_MS } from '../kit/RulePopover';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { useNativeIndexStatus } from './useAppInfo';
import '../styles/compactions.css';

const BATCH = 50;
const RETRY_DELAY_MS = 250;
const temporary = new Set(['cancelled', 'limit', 'replaced', 'incomplete']);
// A fork's inherited part that may be read next time. The native reader keeps
// no unknown inherited answer, but only these are worth one quick retry.
const temporaryInherited = new Set(['cancelled', 'limit', 'replaced', 'unreadable']);
// Numbers each read when it starts, so an older answer never replaces a newer one.
let reads = 0;
/** A row's count, plus whether a read for that row is running now. */
export type CompactionLookup = ((id: string) => CompactionCount | undefined) & {
  reading: (id: string) => boolean;
};

/**
 * What a row shows after a read. `next` is undefined when the read failed or
 * left the row out. A temporary miss keeps the last known outcome, so an
 * active session's number does not flicker to a dash; a lasting reason
 * (such as `missing`) replaces it. A fork whose inherited part missed for a
 * temporary reason keeps the inherited count it last had.
 */
function keep(
  last: CompactionCount | undefined,
  next: CompactionCount | undefined,
): CompactionCount | undefined {
  if (!next) return last;
  if (next.state === 'unknown') return temporary.has(next.reason) ? (last ?? next) : next;
  if (
    next.inherited?.state === 'unknown' &&
    temporaryInherited.has(next.inherited.reason) &&
    last?.state === 'count' &&
    last.inherited?.state === 'count'
  )
    return { ...next, inherited: last.inherited };
  return next;
}

/** Each row's last known outcome for one source, shared by every view, and
 * the number of the read it came from. */
type Shown = {
  found: ReadonlyMap<string, CompactionCount>;
  from: Map<string, number>;
  listeners: Set<() => void>;
};
/** A read's answer (undefined when it failed) and when that read started. */
type Answer = { started: number; outcomes?: ReadonlyMap<string, CompactionCount> };
// Counts are per session and independent of range or filters, so they outlive
// a scope change and a page switch. A new source starts empty.
const shownBySource = new WeakMap<SessionCompactionControls, Shown>();
const nothingShown: Shown = { found: new Map(), from: new Map(), listeners: new Set() };
function shownFor(controls: SessionCompactionControls | undefined): Shown {
  if (!controls) return nothingShown;
  let shown = shownBySource.get(controls);
  if (!shown) {
    shown = { found: new Map(), from: new Map(), listeners: new Set() };
    shownBySource.set(controls, shown);
  }
  return shown;
}

function retryDelay(signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    signal.throwIfAborted();
    const abort = () => {
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      reject(signal.reason);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', abort);
      resolve();
    }, RETRY_DELAY_MS);
    signal.addEventListener('abort', abort, { once: true });
  });
}

/** One queue for visible/loaded main sessions; recent and list IDs dedupe.
 * Source revisions are checked natively on every invalidation/focus, never
 * guessed from record_count or activity timestamps. Answers of a cancelled
 * read (rows, scope or source replaced) are dropped.
 */
export function useSessionCompactions(
  scope: string,
  requested: readonly string[],
  { retryTransient = false }: { retryTransient?: boolean } = {},
): CompactionLookup {
  const { source } = useData();
  useNativeIndexStatus();
  const request = JSON.stringify([...new Set(requested)].sort());
  const ids = useMemo<string[]>(() => JSON.parse(request), [request]);
  const controls = source.compactions;
  const shown = shownFor(controls);
  const subscribe = useCallback(
    (listener: () => void) => {
      shown.listeners.add(listener);
      return () => {
        shown.listeners.delete(listener);
      };
    },
    [shown],
  );
  const found = useSyncExternalStore(subscribe, () => shown.found);
  const query = useQuery({
    queryKey: ['compactions', scope, ids],
    enabled: Boolean(controls) && ids.length > 0,
    gcTime: 0,
    refetchOnWindowFocus: false,
    queryFn: async ({ signal }) => {
      if (!controls) return null;
      const read = async (named: readonly string[]): Promise<Answer> => {
        signal.throwIfAborted();
        const started = ++reads;
        const readId = `compactions-${Date.now().toString(36)}-${started}`;
        const cancel = () => {
          void controls.cancel(readId).catch(() => {});
        };
        signal.addEventListener('abort', cancel, { once: true });
        try {
          const answer = await controls.read(named, readId);
          signal.throwIfAborted();
          return {
            started,
            outcomes: new Map(
              answer.counts
                .filter((entry) => named.includes(entry.id))
                .map((entry) => [entry.id, entry.outcome]),
            ),
          };
        } catch {
          // A failed read is no answer; an aborted one is dropped.
          signal.throwIfAborted();
          return { started };
        } finally {
          signal.removeEventListener('abort', cancel);
        }
      };
      // Publish settled rows immediately, so a lasting verdict in a mixed
      // batch does not wait behind another row's delayed retry. Another view
      // reads the same rows: an answer from a read that started before the
      // one a row shows is older and is skipped. Keeping the last value is
      // no news, so it leaves the row's read number alone.
      const settle = (named: readonly string[], ...answers: Answer[]) => {
        signal.throwIfAborted();
        let next: Map<string, CompactionCount> | undefined;
        for (const id of named)
          for (const { started, outcomes } of answers) {
            if (started <= (shown.from.get(id) ?? 0)) continue;
            const last = (next ?? shown.found).get(id);
            const outcome = keep(last, outcomes?.get(id));
            if (!outcome || outcome === last) continue;
            next ??= new Map(shown.found);
            next.set(id, outcome);
            // A temporary unknown is shown only for want of anything better;
            // it must not block an older read's real answer.
            if (outcome.state === 'count' || !temporary.has(outcome.reason))
              shown.from.set(id, started);
          }
        if (!next) return;
        shown.found = next;
        for (const listener of shown.listeners) listener();
      };
      for (let at = 0; at < ids.length; at += BATCH) {
        const named = ids.slice(at, at + BATCH);
        const first = await read(named);
        const retry = retryTransient
          ? named.filter((id) => {
              const outcome = first.outcomes?.get(id);
              return (
                !first.outcomes ||
                (outcome?.state === 'unknown' && temporary.has(outcome.reason)) ||
                (outcome?.state === 'count' &&
                  outcome.inherited?.state === 'unknown' &&
                  temporaryInherited.has(outcome.inherited.reason))
              );
            })
          : [];
        settle(
          named.filter((id) => !retry.includes(id)),
          first,
        );
        if (retry.length === 0) continue;
        // Dashboard only: one quick retry may turn a temporary miss into a
        // number sooner. Until its answer the row keeps what it showed; the
        // first answer still counts as the older read it was.
        await retryDelay(signal);
        settle(retry, first, await read(retry));
      }
      return null;
    },
  });
  // Reading only while this row's own read runs and it has nothing to show
  // yet: a row with a known outcome keeps it on screen during a re-read.
  const fetching = Boolean(controls) && query.isFetching;
  return useMemo(
    () =>
      // A new function on each change, so a memoized row sees it.
      Object.assign((id: string) => found.get(id), {
        reading: (id: string) => fetching && ids.includes(id) && !found.has(id),
      }),
    [found, fetching, ids],
  );
}

const reasons: Record<string, string> = {
  not_indexed: 'No indexed native source is available.',
  unsupported: 'The saved format is not supported.',
  missing: 'The saved source is missing.',
  unreadable: 'The saved source could not be read.',
  ambiguous: 'More than one source claims this session.',
  replaced: 'The source changed or was replaced during the read.',
  identity_mismatch: 'The source could not be verified as this session.',
  ownership: 'The saved events could not be verified as this session’s own compactions.',
  incomplete: 'The available history is incomplete.',
  limit: 'The read reached its size or time limit.',
  cancelled: 'The read was cancelled.',
  snapshot:
    'This approval reviewer saved only part of the conversation it reviewed, with earlier compactions left out, so they can’t be counted.',
  copied_history:
    'This conversation was copied from another one, and its own compactions can’t be told apart from the copied ones.',
};
// Why a fork's inherited count is unknown, in the words of that conversation.
const inheritedReasons: Record<string, string> = {
  not_indexed: 'The conversation it was forked from is not available.',
  unsupported: 'The conversation it was forked from is saved in a format this app does not read.',
  missing: 'The saved conversation it was forked from could not be found.',
  unreadable: 'The saved conversation it was forked from could not be read.',
  ambiguous: 'More than one saved file claims to be the conversation it was forked from.',
  replaced: 'The conversation it was forked from changed while it was being read.',
  identity_mismatch: 'The saved file does not match the conversation it was forked from.',
  ownership: 'The compactions saved in the conversation it was forked from could not be verified.',
  incomplete: 'The saved conversation it was forked from does not show where the fork happened.',
  limit: 'Reading the conversation it was forked from reached its size or time limit.',
  cancelled: 'The read was cancelled.',
  snapshot: 'The conversation it was forked from saved only part of its history.',
  copied_history:
    'The compactions copied from the conversation it was forked from could not be told apart.',
};

/**
 * A fork's two counts in words: its own, then what it inherits. An older
 * fork's file holds a copy of that history; a newer one only refers to it.
 */
function forkWords(count: number, inherited: InheritedCompactions): string {
  if (inherited.state === 'count' && inherited.copied)
    return `${count} in this conversation + ${inherited.count} copied from the conversation it was forked from`;
  const from = inherited.state === 'count' ? `${inherited.count}` : 'an unknown number';
  return `${count} in this conversation + ${from} in the conversation it was forked from, before the fork`;
}

export const COMPACTION_MEANING =
  'How many times this session’s context was summarized, across all its saved history, not just this range.';

/** The badge's note when its count is the session's own, with no fork. */
export const COMPACTION_CAUTION =
  'Older summaries may be missing. A high count is a reason to look at the session, not a quality score.';

export function CompactionBadge({
  outcome,
  reading = false,
}: {
  outcome?: CompactionCount;
  reading?: boolean;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  const count = outcome && outcome.state !== 'unknown' ? outcome.count : undefined;
  // Only a fork has an inherited part; it is shown even when it is zero.
  const inherited = outcome?.state === 'count' ? outcome.inherited : undefined;
  const from = inherited?.state === 'count' ? inherited.count : undefined;
  const value = count === undefined ? '—' : inherited ? `${count} + ${from ?? '?'}` : `${count}`;
  const step = count === undefined ? 0 : Math.min(5, count + (from ?? 0));
  // A spinner only while the count is being read and none is known yet.
  const loading = reading && !outcome;
  const detail = loading
    ? 'The count is being read now.'
    : !outcome
      ? 'A count has not been read for this session.'
      : outcome.state === 'unknown'
        ? reasons[outcome.reason]
        : inherited
          ? inherited.state === 'unknown'
            ? `${forkWords(outcome.count, inherited)}. ${inheritedReasons[inherited.reason]}`
            : `${forkWords(outcome.count, inherited)}.`
          : COMPACTION_CAUTION;
  // The tooltip, which describes the badge, says a fork's counts in words.
  const label = loading
    ? 'reading'
    : count === undefined
      ? 'unknown'
      : value.replace('?', 'unknown');
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={
          <span
            className="xt-compaction"
            data-step={step}
            data-reading={loading || undefined}
            style={{ '--compaction-step': `${step * 20}%` } as CSSProperties}
          />
        }
        tabIndex={0}
        aria-label={`Recorded compactions: ${label}`}
        aria-describedby={id}
        delay={RULE_OPEN_DELAY_MS}
      >
        <span aria-hidden="true">
          ↺ {loading ? <span className="xt-compaction-spinner" /> : value}
        </span>
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="bottom"
          align="start"
          sideOffset={8}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover xt-rule-definition">
            <span>{COMPACTION_MEANING}</span>
            <span className="xt-rule-context">{detail}</span>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/** How a compaction started, in words. Codex never records it. */
export const TRIGGER_TEXT: Record<CompactionTrigger, string> = {
  auto: 'Automatic',
  manual: 'Manual',
  unknown: 'Automatic or manual: not recorded',
};

/**
 * The recorded compactions of one count that fall inside `[start, end]`, the
 * window a lane draws. Unknown counts and Cursor counts have none.
 */
export function compactionEvents(
  outcome: CompactionCount | undefined,
  start: number,
  end: number,
): CompactionEvent[] {
  if (!outcome || outcome.state !== 'count') return [];
  return outcome.events.filter((event) => event.at_ms >= start && event.at_ms <= end);
}

/**
 * One recorded compaction drawn on an activity lane at `left` (0–1 of the
 * lane). It is a focusable mark whose tooltip says when it was recorded, in
 * `when`, and whether it was automatic or manual.
 */
export function CompactionTick({
  event,
  left,
  when,
}: {
  event: CompactionEvent;
  left: number;
  when: string;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  const trigger = TRIGGER_TEXT[event.trigger];
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={
          <span
            className="xt-compaction-tick"
            data-trigger={event.trigger}
            style={{ left: `${left * 100}%` }}
          />
        }
        tabIndex={0}
        // A named graphic: the mark itself has no text for a name to label.
        role="img"
        aria-label={`Compaction recorded ${when}. ${trigger}.`}
        aria-describedby={id}
        delay={0}
      />
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="top"
          align="center"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover">
            <span>Compaction · {when}</span>
            <span className="xt-rule-context">{trigger}</span>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
