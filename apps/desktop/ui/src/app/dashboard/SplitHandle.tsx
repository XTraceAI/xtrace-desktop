import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from 'react';
import { clampFirst, readSplit, roundShare, writeSplit, type SplitAxis } from './layout-split';

/**
 * A boundary between two Dashboard panes that the pointer drags and the
 * keyboard moves: a window splitter (`role="separator"`) placed between its
 * two panes, inside the element that lays them out. `rows` stacks them (the
 * line is horizontal and moves up and down), `columns` sets them side by side.
 *
 * The layout is the stylesheet's. This handle only writes the first pane's
 * share onto that container — `data-split-<axis>` with `--<axis>-first` and
 * `--<axis>-second` — so each pane's minimum stays a CSS rule, and a window
 * resize keeps the proportion. A drag writes the share as it moves and stores
 * it when released; a key stores it at once. Double-click or Enter returns to
 * the default layout and forgets the stored share.
 */
export function SplitHandle({
  axis,
  label,
  names,
  controls,
}: {
  axis: SplitAxis;
  label: string;
  /** The two panes, first then second, as the value is spoken. */
  names: readonly [string, string];
  /** The first pane's id. */
  controls?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [dragging, setDragging] = useState(false);
  const [value, setValue] = useState<Value>({ now: 50, min: 0, max: 100, custom: false });

  /**
   * Re-reads the drawn split. `bounds` also re-measures how far each pane can
   * shrink, which takes two extra layouts, so a window resize asks for it only
   * once the size settles. Returns whether anything could be measured: a
   * hidden page draws nothing.
   */
  const refresh = (bounds = false): boolean => {
    const handle = ref.current;
    if (!handle) return false;
    const drawn = measure(handle, axis);
    if (!drawn || drawn.total <= 0) return false;
    const custom = handle.parentElement?.hasAttribute(attribute(axis)) ?? false;
    const now = percent(drawn.first / drawn.total);
    const limits = bounds ? probe(handle, axis) : null;
    setValue((last) => {
      const next = {
        now,
        custom,
        min: limits ? percent(limits.minFirst / drawn.total) : last.min,
        max: limits ? percent(1 - limits.minSecond / drawn.total) : last.max,
      };
      return next.now === last.now &&
        next.custom === last.custom &&
        next.min === last.min &&
        next.max === last.max
        ? last
        : next;
    });
    return true;
  };

  const refreshRef = useRef(refresh);
  useLayoutEffect(() => {
    refreshRef.current = refresh;
  });

  // The stored share is drawn before the first paint; the default layout
  // otherwise. Leaving the page clears it from the container.
  useLayoutEffect(() => {
    const panes = panesOf(ref.current);
    if (!panes) return;
    apply(panes, axis, readSplit()[axis]);
    return () => apply(panes, axis, null);
  }, [axis]);

  // The value follows the panes' drawn sizes each frame: a window resize, a
  // list that grows, the narrow layout. How far each pane can shrink depends
  // on what the panes hold — Environment's list arriving after the first
  // paint, a group opening — so it is re-measured whenever a pane's size or
  // content changes, once things have been still for a moment, and again
  // until a measure succeeds.
  useEffect(() => {
    const handle = ref.current;
    const panes = panesOf(handle);
    if (!handle || !panes) return;
    let bounds = true;
    let frame = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = () => {
      frame = 0;
      if (refreshRef.current(bounds)) bounds = false;
    };
    const update = () => {
      if (!frame) frame = requestAnimationFrame(run);
    };
    const settle = () => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        bounds = true;
        update();
      }, SETTLE_MS);
    };
    update();
    const resized =
      typeof ResizeObserver === 'function'
        ? new ResizeObserver(() => {
            update();
            settle();
          })
        : null;
    for (const element of [panes.container, panes.first, panes.second]) resized?.observe(element);
    const changed = typeof MutationObserver === 'function' ? new MutationObserver(settle) : null;
    for (const element of [panes.first, panes.second])
      changed?.observe(element, { childList: true, subtree: true });
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
      resized?.disconnect();
      changed?.disconnect();
    };
  }, [axis]);

  // A drag cut short by leaving the page does not leave its cursor behind.
  useEffect(
    () => () => {
      if (drag.current) delete document.documentElement.dataset.xtResizing;
    },
    [],
  );

  /** Draws and stores a share, or the default layout for `null`. */
  const commit = (share: number | null) => {
    const panes = panesOf(ref.current);
    if (!panes) return;
    const next = share === null ? null : roundShare(share);
    apply(panes, axis, next);
    writeSplit(axis, next);
    refresh(true);
  };

  /**
   * Ends a drag: a release stores where it ended; Escape, a cancelled pointer
   * or a lost capture puts back the layout from before it, stored or default.
   */
  const finish = (keep: boolean) => {
    const current = drag.current;
    if (!current) return;
    drag.current = null;
    setDragging(false);
    delete document.documentElement.dataset.xtResizing;
    const handle = ref.current;
    if (handle?.hasPointerCapture?.(current.pointer)) handle.releasePointerCapture(current.pointer);
    if (keep) {
      if (current.moved) commit(current.share);
      return;
    }
    const panes = panesOf(handle);
    if (panes) restore(panes, axis, current.before);
    refresh(true);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const handle = ref.current;
    if (!handle || event.button !== 0 || !event.isPrimary) return;
    const drawn = measure(handle, axis);
    if (!drawn || drawn.total <= 0) return;
    event.preventDefault();
    const limits = probe(handle, axis);
    drag.current = {
      pointer: event.pointerId,
      before: snapshot(handle.parentElement!, axis),
      start: axis === 'rows' ? event.clientY : event.clientX,
      first: drawn.first,
      total: drawn.total,
      ...limits,
      share: drawn.first / drawn.total,
      moved: false,
    };
    handle.setPointerCapture?.(event.pointerId);
    handle.focus({ preventScroll: true });
    setDragging(true);
    document.documentElement.dataset.xtResizing = axis;
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    const panes = panesOf(ref.current);
    if (!current || !panes) return;
    const delta = (axis === 'rows' ? event.clientY : event.clientX) - current.start;
    if (!current.moved && Math.abs(delta) < 1) return;
    current.moved = true;
    const size = clampFirst(
      current.first + delta,
      current.total,
      current.minFirst,
      current.minSecond,
    );
    current.share = size / current.total;
    apply(panes, axis, current.share);
    setValue((last) => ({ ...last, now: percent(current.share), custom: true }));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const handle = ref.current;
    if (!handle) return;
    if (drag.current) {
      if (event.key === 'Escape') {
        event.preventDefault();
        finish(false);
      }
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      commit(null);
      return;
    }
    const step = event.shiftKey ? 64 : 16;
    const moves: Record<string, number> =
      axis === 'rows'
        ? { ArrowUp: -step, ArrowDown: step }
        : { ArrowLeft: -step, ArrowRight: step };
    if (!(event.key in moves) && event.key !== 'Home' && event.key !== 'End') return;
    const drawn = measure(handle, axis);
    if (!drawn || drawn.total <= 0) return;
    event.preventDefault();
    const { minFirst, minSecond } = probe(handle, axis);
    const target =
      event.key === 'Home'
        ? minFirst
        : event.key === 'End'
          ? drawn.total - minSecond
          : drawn.first + moves[event.key]!;
    commit(clampFirst(target, drawn.total, minFirst, minSecond) / drawn.total);
  };

  const [first, second] = names;
  // Measured at different moments, the value is never spoken outside its range.
  const now = Math.min(value.max, Math.max(value.min, value.now));
  return (
    <div
      ref={ref}
      className="xt-split"
      data-axis={axis}
      data-dragging={dragging || undefined}
      role="separator"
      tabIndex={0}
      aria-label={label}
      aria-orientation={axis === 'rows' ? 'horizontal' : 'vertical'}
      aria-controls={controls}
      aria-valuenow={now}
      aria-valuemin={value.min}
      aria-valuemax={value.max}
      aria-valuetext={`${first} ${now}%, ${second} ${100 - now}%${
        value.custom ? '' : ', default layout'
      }`}
      title="Drag to resize. Double-click or press Enter to reset."
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={() => finish(true)}
      onPointerCancel={() => finish(false)}
      onLostPointerCapture={() => finish(false)}
      onDoubleClick={() => commit(null)}
      onKeyDown={onKeyDown}
      onFocus={() => refresh(true)}
    />
  );
}

interface Value {
  now: number;
  min: number;
  max: number;
  /** Whether a chosen share is drawn rather than the default layout. */
  custom: boolean;
}
interface Drag {
  pointer: number;
  /** The container's split before the drag, put back if it is cancelled. */
  before: Snapshot;
  start: number;
  first: number;
  total: number;
  minFirst: number;
  minSecond: number;
  share: number;
  moved: boolean;
}

const attribute = (axis: SplitAxis) => `data-split-${axis}`;
/** How long a pane's size or content must be still before its bounds are re-measured. */
const SETTLE_MS = 150;

interface Snapshot {
  on: boolean;
  first: string;
  second: string;
}
const snapshot = (container: HTMLElement, axis: SplitAxis): Snapshot => ({
  on: container.hasAttribute(attribute(axis)),
  first: container.style.getPropertyValue(`--${axis}-first`),
  second: container.style.getPropertyValue(`--${axis}-second`),
});
function restore(panes: Panes, axis: SplitAxis, saved: Snapshot) {
  if (!saved.on) return apply(panes, axis, null);
  panes.container.setAttribute(attribute(axis), '');
  panes.container.style.setProperty(`--${axis}-first`, saved.first);
  panes.container.style.setProperty(`--${axis}-second`, saved.second);
}
const percent = (share: number) => Math.min(100, Math.max(0, Math.round(share * 100)));

/**
 * Writes a share onto the container, or clears it for the default layout.
 * Each pane's weight is the size it should take less its own borders and
 * padding, which a flex item keeps even at a zero basis — the Sessions card's
 * 1px borders — so the drawn share is the one asked for. The weights are
 * pixel-sized, so their sum is never below one, which a flex container would
 * otherwise leave partly unfilled.
 */
function apply(panes: Panes, axis: SplitAxis, share: number | null) {
  const { container, first, second } = panes;
  const style = container.style;
  if (share === null) {
    container.removeAttribute(attribute(axis));
    style.removeProperty(`--${axis}-first`);
    style.removeProperty(`--${axis}-second`);
    return;
  }
  const total = extent(first, axis) + extent(second, axis);
  const [edgeFirst, edgeSecond] =
    axis === 'rows' && total > 0 ? [edges(first), edges(second)] : [0, 0];
  const size = total > 0 ? total : 1000;
  const weight = (value: number) => `${Math.max(value, 0.001)}${axis === 'columns' ? 'fr' : ''}`;
  container.setAttribute(attribute(axis), '');
  style.setProperty(`--${axis}-first`, weight(share * size - edgeFirst));
  style.setProperty(`--${axis}-second`, weight((1 - share) * size - edgeSecond));
}

interface Panes {
  container: HTMLElement;
  first: Element;
  second: Element;
}
function panesOf(handle: HTMLElement | null): Panes | null {
  const container = handle?.parentElement;
  const first = handle?.previousElementSibling;
  const second = handle?.nextElementSibling;
  return container && first && second ? { container, first, second } : null;
}

/** A row pane's vertical borders and padding: what its box keeps at a zero flex basis. */
function edges(element: Element) {
  const style = getComputedStyle(element);
  return [style.borderTopWidth, style.borderBottomWidth, style.paddingTop, style.paddingBottom]
    .map((value) => parseFloat(value) || 0)
    .reduce((sum, value) => sum + value, 0);
}

const extent = (element: Element, axis: SplitAxis) => {
  const box = element.getBoundingClientRect();
  return axis === 'rows' ? box.height : box.width;
};

/** The two panes' drawn sizes along the axis. */
function measure(handle: HTMLElement, axis: SplitAxis) {
  const panes = panesOf(handle);
  if (!panes) return null;
  const first = extent(panes.first, axis);
  return { first, total: first + extent(panes.second, axis) };
}

/**
 * How small each pane can be drawn, read from the stylesheet's own minimums:
 * the split is pushed to each end in turn, measured and put back before the
 * next paint, so nothing on screen moves. A list scrolled inside a pane would
 * have its position clamped while the pane is small, so every scrolled
 * position is put back too.
 */
function probe(handle: HTMLElement, axis: SplitAxis) {
  const panes = panesOf(handle)!;
  const saved = snapshot(panes.container, axis);
  const scrolled = [panes.first, panes.second]
    .flatMap((pane) => [pane, ...pane.querySelectorAll('*')])
    .filter((element) => element.scrollTop !== 0 || element.scrollLeft !== 0)
    .map((element) => [element, element.scrollTop, element.scrollLeft] as const);
  apply(panes, axis, 0.0001);
  const minFirst = extent(panes.first, axis);
  apply(panes, axis, 0.9999);
  const minSecond = extent(panes.second, axis);
  restore(panes, axis, saved);
  for (const [element, top, left] of scrolled) {
    element.scrollTop = top;
    element.scrollLeft = left;
  }
  return { minFirst, minSecond };
}
