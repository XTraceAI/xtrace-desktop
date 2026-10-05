import { useCallback, useEffect, useRef, useState } from 'react';
import { useData } from '../data/DataProvider';

/**
 * Host titles for the rows a list is showing, read from the sessions' own
 * local sources on demand and held only while the list is on screen.
 *
 * Nothing here is stored, cached across views or searched: the index holds no
 * such title, so a row keeps its identifier until its title arrives, and keeps
 * it for good when there is none. A read names only rows the list is showing,
 * at most one page of them, and one read runs at a time. A read belongs to the
 * list state it was made for — its range and filters, the `scope` — and to the
 * exact rows it named: when the scope changes or the list goes away it is
 * cancelled, and an answer that arrives for another scope, or for a read that
 * was replaced, is dropped rather than shown against rows it was not about. A
 * title belongs to the row version it was read for: a row whose version
 * changes shows its fallback until it is read again, or, with `keepResolved`,
 * keeps its last answered title until the new read answers. A running read
 * that named a row the list no longer shows, or shows at another version, is
 * cancelled and replaced; one whose rows are all still shown as named keeps
 * running.
 */

/** The most rows one read names: one Sessions page, the app's own cap. */
export const TITLE_BATCH = 50;

export interface TitleRow {
  id: string;
  /**
   * Changes when the row's indexed source has, so a title that may have
   * changed with it is read again. Until then the row shows its fallback.
   */
  version: string | number;
}

interface Found {
  version: string | number;
  title: string | null;
}

interface InFlight {
  readId: string;
  rows: readonly TitleRow[];
}

interface TitleState {
  scope: string;
  found: ReadonlyMap<string, Found>;
  inflight: InFlight | null;
}

const NOTHING: ReadonlyMap<string, Found> = new Map();

let reads = 0;
/** An opaque name for one read, unique in this window. */
const nextReadId = () => `titles-${Date.now().toString(36)}-${(reads += 1).toString(36)}`;

/** The rows a read would name next: unread or changed, each once, in order. */
function nextBatch(rows: readonly TitleRow[], found: ReadonlyMap<string, Found>): TitleRow[] {
  const batch: TitleRow[] = [];
  const named = new Set<string>();
  for (const row of rows) {
    if (batch.length === TITLE_BATCH) break;
    if (named.has(row.id) || found.get(row.id)?.version === row.version) continue;
    named.add(row.id);
    batch.push(row);
  }
  return batch;
}

const requestKey = (rows: readonly TitleRow[]) =>
  rows.map((row) => `${row.id}\u0000${row.version}`).join('\n');

/** Whether a read named some row the list no longer shows as it named it. */
function outdated(named: readonly TitleRow[], rows: readonly TitleRow[]): boolean {
  const now = new Map<string, TitleRow['version']>();
  for (const row of rows) if (!now.has(row.id)) now.set(row.id, row.version);
  // A version is never undefined, so a row no longer shown counts as changed.
  return named.some((row) => now.get(row.id) !== row.version);
}

/**
 * The host title of each row that has one, for `rows` as they are now under
 * `scope`. Returns a lookup by a row's identifier and its current version;
 * `null` means the row shows its fallback. With `keepResolved`, a row whose
 * version moved keeps the title last answered for it under this scope until
 * a read of its new version answers, which then decides: a new title, or the
 * fallback when it found none or failed. A cancelled read decides nothing.
 */
export function useSessionTitles(
  scope: string,
  rows: readonly TitleRow[],
  { keepResolved = false }: { keepResolved?: boolean } = {},
): (id: string, version: TitleRow['version']) => string | null {
  const { source } = useData();
  const controls = source.titles;
  const [state, setState] = useState<TitleState>(() => ({
    scope,
    found: NOTHING,
    inflight: null,
  }));
  const current = state.scope === scope ? state : null;
  const found = current?.found ?? NOTHING;
  const inflight = current?.inflight ?? null;
  // While a read runs, its own rows are the request, so starting it or adding
  // rows does not restart it; afterwards the next unread rows are. A read that
  // named a row no longer shown, or shown at another version, is replaced,
  // which cancels it.
  const pinned = inflight && !outdated(inflight.rows, rows) ? inflight.rows : null;
  const batch = pinned ?? (controls ? nextBatch(rows, found) : []);
  const request = requestKey(batch);
  // Reads started and not yet answered. A read absent from here when its
  // answer arrives was cancelled, and its answer is not used.
  const running = useRef(new Set<string>());
  const latest = useRef(batch);
  useEffect(() => {
    latest.current = batch;
  });

  useEffect(() => {
    if (!controls || request === '') return;
    const named = latest.current;
    const readId = nextReadId();
    const started = running.current;
    started.add(readId);
    setState((previous) => ({
      scope,
      found: previous.scope === scope ? previous.found : NOTHING,
      inflight: { readId, rows: named },
    }));
    const settle = (titles: ReadonlyMap<string, string> | null) => {
      const answered = started.delete(readId);
      setState((previous) => {
        if (previous.scope !== scope || previous.inflight?.readId !== readId) return previous;
        // Cancelled: nothing is learned, and the rows stay unread.
        if (!answered) return { ...previous, inflight: null };
        const next = new Map(previous.found);
        for (const row of named)
          next.set(row.id, { version: row.version, title: titles?.get(row.id) ?? null });
        return { scope, found: next, inflight: null };
      });
    };
    controls
      .read(
        named.map((row) => row.id),
        readId,
      )
      .then(
        (answer) => {
          // Only the rows this read named, each once.
          const asked = new Set(named.map((row) => row.id));
          settle(
            new Map(
              answer.titles
                .filter((entry) => asked.has(entry.id))
                .map((entry) => [entry.id, entry.title]),
            ),
          );
        },
        // A refused or failed read leaves the identifiers, and is not retried
        // until the rows change.
        () => settle(null),
      );
    return () => {
      if (!started.delete(readId)) return;
      void controls.cancel(readId).catch(() => {});
    };
  }, [controls, scope, request]);

  return useCallback(
    (id: string, version: TitleRow['version']) => {
      const known = found.get(id);
      return known && (keepResolved || known.version === version) ? known.title : null;
    },
    [found, keepResolved],
  );
}

/**
 * Which of a scrolling list's rows are inside its scroll area now.
 *
 * Attach `observe` to one element per row carrying `data-visible-id`; the
 * nearest scroll region (`.xt-table-scroll`) is the viewport. Rows scrolled
 * out of view leave the set and rows scrolled in join it, so a list can ask
 * only about what is on screen. Without IntersectionObserver nothing is
 * reported visible.
 */
export function useVisibleIds(): {
  observe: (element: HTMLElement | null) => (() => void) | undefined;
  visible: ReadonlySet<string>;
} {
  const [visible, setVisible] = useState<ReadonlySet<string>>(() => new Set());
  const observer = useRef<IntersectionObserver | null>(null);
  useEffect(
    () => () => {
      observer.current?.disconnect();
      observer.current = null;
    },
    [],
  );
  const observe = useCallback((element: HTMLElement | null) => {
    if (!element || typeof IntersectionObserver === 'undefined') return undefined;
    if (!observer.current) {
      observer.current = new IntersectionObserver(
        (entries) =>
          setVisible((previous) => {
            let next: Set<string> | null = null;
            for (const entry of entries) {
              const id = (entry.target as HTMLElement).dataset.visibleId;
              if (!id || entry.isIntersecting === previous.has(id)) continue;
              next ??= new Set(previous);
              if (entry.isIntersecting) next.add(id);
              else next.delete(id);
            }
            return next ?? previous;
          }),
        { root: element.closest('.xt-table-scroll') },
      );
    }
    const watching = observer.current;
    watching.observe(element);
    return () => {
      watching.unobserve(element);
      const id = element.dataset.visibleId;
      if (id)
        setVisible((previous) => {
          if (!previous.has(id)) return previous;
          const next = new Set(previous);
          next.delete(id);
          return next;
        });
    };
  }, []);
  return { observe, visible };
}
