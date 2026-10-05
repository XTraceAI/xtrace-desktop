import { Tooltip } from '@base-ui/react/tooltip';
import { useQuery } from '@tanstack/react-query';
import { useCallback, useEffect, useId, useMemo, useState, type CSSProperties } from 'react';
import { useData } from '../data/DataProvider';
import type { CompactionCount } from '../data/generated/CompactionCount';
import type { CompactionEvent } from '../data/generated/CompactionEvent';
import type { CompactionTrigger } from '../data/generated/CompactionTrigger';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { useNativeIndexStatus } from './useAppInfo';
import '../styles/compactions.css';

const BATCH = 50;
const RETRY_DELAY_MS = 250;
const temporary = new Set(['cancelled', 'limit', 'replaced', 'incomplete']);
let reads = 0;
export type CompactionLookup = (id: string) => CompactionCount | undefined;

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
 * guessed from record_count or activity timestamps. Late results are dropped.
 */
export function useSessionCompactions(
  scope: string,
  requested: readonly string[],
  {
    keepResolved = false,
    retryTransient = false,
  }: { keepResolved?: boolean; retryTransient?: boolean } = {},
): CompactionLookup {
  const { source } = useData();
  useNativeIndexStatus();
  const request = JSON.stringify([...new Set(requested)].sort());
  const ids = useMemo<string[]>(() => JSON.parse(request), [request]);
  const controls = source.compactions;
  const [resolved, setResolved] = useState(() => ({
    scope,
    controls,
    found: new Map<string, CompactionCount>(),
  }));
  const query = useQuery({
    queryKey: ['compactions', scope, ids],
    enabled: Boolean(controls) && ids.length > 0,
    gcTime: 0,
    refetchOnWindowFocus: false,
    queryFn: async ({ signal }) => {
      const found = new Map<string, CompactionCount>();
      if (!controls) return found;
      const read = async (named: readonly string[]) => {
        signal.throwIfAborted();
        const readId = `compactions-${Date.now().toString(36)}-${++reads}`;
        const cancel = () => {
          void controls.cancel(readId).catch(() => {});
        };
        signal.addEventListener('abort', cancel, { once: true });
        try {
          const answer = await controls.read(named, readId);
          signal.throwIfAborted();
          return new Map(
            answer.counts
              .filter((entry) => named.includes(entry.id))
              .map((entry) => [entry.id, entry.outcome]),
          );
        } finally {
          signal.removeEventListener('abort', cancel);
        }
      };
      const settle = (named: readonly string[], outcomes: ReadonlyMap<string, CompactionCount>) => {
        signal.throwIfAborted();
        for (const id of named) {
          found.delete(id);
          const outcome = outcomes.get(id);
          if (outcome) found.set(id, outcome);
        }
        // Publish settled rows immediately. A terminal verdict in a mixed
        // batch must not wait behind another row's delayed retry.
        if (retryTransient)
          setResolved((previous) => {
            if (signal.aborted) return previous;
            const next =
              previous.scope === scope && previous.controls === controls
                ? new Map(previous.found)
                : new Map<string, CompactionCount>();
            for (const id of named) {
              next.delete(id);
              const outcome = outcomes.get(id);
              if (outcome) next.set(id, outcome);
            }
            return { scope, controls, found: next };
          });
      };
      for (let at = 0; at < ids.length; at += BATCH) {
        const named = ids.slice(at, at + BATCH);
        let first: ReadonlyMap<string, CompactionCount> | undefined;
        try {
          first = await read(named);
        } catch (error) {
          signal.throwIfAborted();
          if (!retryTransient) throw error;
        }
        const retry = retryTransient
          ? named.filter((id) => {
              const outcome = first?.get(id);
              return !first || (outcome?.state === 'unknown' && temporary.has(outcome.reason));
            })
          : [];
        settle(
          named.filter((id) => !retry.includes(id)),
          first ?? new Map(),
        );
        if (retry.length === 0) continue;
        // Dashboard only: one retry, keeping last verified values until its
        // answer. A final unknown or rejection clears those old values.
        await retryDelay(signal);
        let second: ReadonlyMap<string, CompactionCount> = new Map();
        try {
          second = await read(retry);
        } catch {
          signal.throwIfAborted();
        }
        settle(retry, second);
      }
      return found;
    },
  });
  // The Dashboard keeps each row's last answer while scrolling changes the
  // visible batch. Reads remain limited to that batch; new scope/source starts
  // empty. Settled unknowns/failures replace the requested IDs' old values;
  // a dashboard transient gets one delayed retry before it is settled.
  useEffect(() => {
    if (!keepResolved) return;
    setResolved((previous) => {
      const found =
        previous.scope === scope && previous.controls === controls
          ? new Map(previous.found)
          : new Map<string, CompactionCount>();
      if (query.isError || query.data) {
        for (const id of ids) found.delete(id);
        if (!query.isError) for (const [id, outcome] of query.data!) found.set(id, outcome);
      }
      return { scope, controls, found };
    });
  }, [keepResolved, scope, controls, ids, query.data, query.isError]);
  // The current batch's settled result takes precedence, including a failure.
  // While a new batch is pending, retain rows already read in this scope.
  const current = query.isError ? undefined : query.data;
  return useCallback(
    (id) => {
      if (retryTransient && query.isFetching)
        return resolved.scope === scope && resolved.controls === controls
          ? resolved.found.get(id)
          : undefined;
      if (ids.includes(id) && (query.isError || current)) return current?.get(id);
      return (
        current?.get(id) ??
        (keepResolved && resolved.scope === scope && resolved.controls === controls
          ? resolved.found.get(id)
          : undefined)
      );
    },
    [
      current,
      ids,
      query.isError,
      query.isFetching,
      keepResolved,
      retryTransient,
      resolved,
      scope,
      controls,
    ],
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
};
export const COMPACTION_MEANING =
  'Recorded compactions count distinct times this session’s own context was summarized in its available saved history, across all available dates, independent of the selected range. Repeated summarization is a caution to check the session; it does not measure model quality or the probability of an unhealthy session.';

export function CompactionBadge({ outcome }: { outcome?: CompactionCount }) {
  const id = useId();
  const theme = useSurfaceTheme();
  const count = outcome && outcome.state !== 'unknown' ? outcome.count : undefined;
  const value = count === undefined ? '—' : `${count}`;
  const step = count === undefined ? 0 : Math.min(5, count);
  const detail = !outcome
    ? 'A count has not been read for this session.'
    : outcome.state === 'unknown'
      ? reasons[outcome.reason]
      : 'Recorded compactions in available own-session history; older events may be missing.';
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={
          <span
            className="xt-compaction"
            data-step={step}
            style={{ '--compaction-step': `${step * 20}%` } as CSSProperties}
          />
        }
        tabIndex={0}
        aria-label={`Recorded compactions: ${count === undefined ? 'unknown' : value}`}
        aria-describedby={id}
        delay={0}
      >
        <span aria-hidden="true">↺ {value}</span>
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
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover">
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
