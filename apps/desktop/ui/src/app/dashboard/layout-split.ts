/**
 * The Dashboard's two adjustable boundaries: between the Effort + Environment
 * row and Sessions (`rows`), and between Effort and Environment (`columns`).
 * Each is either the default layout (`null`) or the share, between 0 and 1,
 * that the first pane — the upper row, or Effort — takes of the room the two
 * panes have. A share rather than pixels, so a resized window keeps the
 * proportion; each pane's minimum is the stylesheet's, never stored here.
 *
 * The choice is kept in this browser profile under its own versioned key, as
 * the theme is. Unreadable or malformed storage reads as the default layout,
 * and a failed write only loses the choice for the next start.
 */
export type SplitAxis = 'rows' | 'columns';
export type DashboardSplit = Record<SplitAxis, number | null>;

export const splitStorageKey = 'xt.dashboard.layout.v1';
const DEFAULT_SPLIT: DashboardSplit = { rows: null, columns: null };

/** A stored share is usable only strictly between 0 and 1. */
const share = (value: unknown): number | null =>
  typeof value === 'number' && Number.isFinite(value) && value > 0 && value < 1 ? value : null;

export function readSplit(): DashboardSplit {
  try {
    const raw = localStorage.getItem(splitStorageKey);
    if (!raw) return { ...DEFAULT_SPLIT };
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object') return { ...DEFAULT_SPLIT };
    const record = parsed as Partial<Record<SplitAxis, unknown>>;
    return { rows: share(record.rows), columns: share(record.columns) };
  } catch {
    return { ...DEFAULT_SPLIT };
  }
}

/** Stores one axis, keeping the other; returns whether the choice was stored. */
export function writeSplit(axis: SplitAxis, value: number | null): boolean {
  try {
    const next = { ...readSplit(), [axis]: value === null ? null : share(value) };
    if (next.rows === null && next.columns === null) localStorage.removeItem(splitStorageKey);
    else localStorage.setItem(splitStorageKey, JSON.stringify(next));
    return true;
  } catch {
    return false;
  }
}

/**
 * The first pane's size after a move, kept within both panes' minimums.
 * `total` is the two panes' combined size; `minFirst` and `minSecond` are the
 * least each can be drawn at.
 */
export function clampFirst(
  size: number,
  total: number,
  minFirst: number,
  minSecond: number,
): number {
  const most = Math.max(minFirst, total - minSecond);
  return Math.min(Math.max(size, minFirst), most);
}

/** A share rounded to four places, so a stored value stays short and stable. */
export const roundShare = (value: number) => Math.round(value * 10_000) / 10_000;
