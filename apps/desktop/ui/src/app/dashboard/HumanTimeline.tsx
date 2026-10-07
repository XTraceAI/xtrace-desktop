import { Tooltip } from '@base-ui/react/tooltip';
import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from 'react';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { agentTime } from '../agent-duration';
import { timelineRows, wholeDaysText, type TimelineRow } from './overview';
import '../../styles/overview.css';

/** The 7-day range's eight days fill the plot; longer ranges keep that row size and scroll. */
export const VISIBLE_ROWS = 8;
/** Below this a row's bar and its gap stop reading as separate rows. */
const MIN_ROW_PX = 28;
const HOURS = ['00', '06', '12', '18', '24'];

/**
 * Human time as a 24-hour timeline: one row per reported local day, oldest
 * first, each a whole day with its stretches drawn across its hours and its
 * total at the right. Rows keep the height eight rows have in the plot, so a
 * 14- or 30-day range scrolls instead of shrinking; the hour axis stays above
 * the rows.
 *
 * It opens on today, at the bottom, when it first appears (switching to
 * "human h" mounts it) and when the range changes. A refresh of the same range
 * (live updates send a new report after every finished turn) leaves the
 * scroll position alone. When the rows change height with the card, a view
 * that was at the bottom stays there and any other keeps the same rows in view.
 *
 * The timeline is one tab stop: one day row at a time (today at first). Up and
 * Down move to the day before or after, Home and End to the first and last;
 * a focused day opens its card, as hovering does.
 */
export function HumanTimeline({ report }: { report: DashboardMetrics }) {
  const human = report.human_hours.current;
  const rows = useMemo(() => timelineRows(human.by_day, report.window), [human, report.window]);
  const body = useRef<HTMLDivElement>(null);
  const [rowHeight, setRowHeight] = useState<number | null>(null);
  const [above, setAbove] = useState(false);
  /** The range the view last opened on today for; `null` before the first. */
  const openedFor = useRef<number | null>(null);
  const drawnRowHeight = useRef<number | null>(null);
  const atBottom = useRef(true);
  const days = report.window.days;
  /** The day that takes the timeline's tab stop; today until another is chosen. */
  const [chosen, setChosen] = useState<string | null>(null);
  const found = rows.findIndex((row) => row.date === chosen);
  const current = found >= 0 ? found : rows.length - 1;
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const next = {
      ArrowUp: current - 1,
      ArrowDown: current + 1,
      Home: 0,
      End: rows.length - 1,
    }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    const index = Math.max(0, Math.min(rows.length - 1, next));
    setChosen(rows[index]!.date);
    body.current?.querySelectorAll<HTMLElement>('[data-testid="human-day"]')[index]?.focus();
  };
  // Size the rows from the space eight of them have, and follow the card.
  useLayoutEffect(() => {
    const element = body.current;
    if (!element) return;
    const fit = () =>
      setRowHeight(Math.max(MIN_ROW_PX, Math.floor(element.clientHeight / VISIBLE_ROWS)));
    fit();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  // Open on today, the last row, for each new range; follow a change of row
  // height. Nothing here runs for a new report of the same range.
  useLayoutEffect(() => {
    const element = body.current;
    if (!element || rowHeight === null) return;
    const previous = drawnRowHeight.current;
    if (openedFor.current !== days) {
      openedFor.current = days;
      element.scrollTop = element.scrollHeight;
    } else if (previous !== null && previous !== rowHeight) {
      element.scrollTop = atBottom.current
        ? element.scrollHeight
        : Math.round((element.scrollTop * rowHeight) / previous);
    }
    drawnRowHeight.current = rowHeight;
    atBottom.current = isAtBottom(element);
    setAbove(element.scrollTop > 2);
  }, [days, rowHeight]);
  const unknown = human.active_ms === null;
  return (
    <div className="xt-effort xt-human" data-testid="human-timeline">
      <p className="xt-effort-headline" data-testid="effort-headline">
        <strong data-testid="effort-total">
          {unknown ? 'human time unknown' : `${agentTime(human.active_ms!)} human`}
        </strong>
        <span>{wholeDaysText(human.by_day)}</span>
        <small>
          {unknown
            ? 'Could not tell which messages a person sent.'
            : `gaps over ${human.break_minutes} min count as breaks`}
        </small>
      </p>
      <div className="xt-human-plot">
        <div className="xt-human-axis" aria-hidden="true">
          <span />
          <span className="xt-human-hours">
            {HOURS.map((hour) => (
              <span key={hour}>{hour}</span>
            ))}
          </span>
          <span />
        </div>
        <div
          ref={body}
          className="xt-human-body"
          role="list"
          aria-label={`Human time by day, ${rows.length} days, each from 00:00 to 24:00`}
          data-more-above={above || undefined}
          data-testid="human-timeline-body"
          onKeyDown={move}
          onScroll={(event) => {
            atBottom.current = isAtBottom(event.currentTarget);
            setAbove(event.currentTarget.scrollTop > 2);
          }}
          style={
            rowHeight === null ? undefined : ({ '--row-h': `${rowHeight}px` } as CSSProperties)
          }
        >
          {rows.map((row, index) => (
            <Row
              key={row.date}
              row={row}
              lower={index >= rows.length / 2}
              current={index === current}
              onFocus={() => setChosen(row.date)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

/** Within two pixels of the end: the newest row is in view. */
const isAtBottom = (element: HTMLElement) =>
  element.scrollTop + element.clientHeight >= element.scrollHeight - 2;

/**
 * One day: a list item whose name lists every stretch, and whose card opens
 * on hover or focus. Only the current day is in the tab order; arrow keys
 * move between days. Each bar also names its own times.
 */
function Row({
  row,
  lower,
  current,
  onFocus,
}: {
  row: TimelineRow;
  lower: boolean;
  current: boolean;
  onFocus: () => void;
}) {
  const theme = useSurfaceTheme();
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        delay={0}
        closeOnClick={false}
        render={
          <div
            role="listitem"
            tabIndex={current ? 0 : -1}
            onFocus={onFocus}
            aria-label={row.name}
            className="xt-human-row"
            data-testid="human-day"
            data-date={row.date}
          />
        }
      >
        <span className="xt-human-day" data-weekend={row.weekend || undefined}>
          {row.label}
        </span>
        <span className="xt-human-track" data-fold={row.fold || undefined}>
          {row.bars.map((bar, index) => (
            <span
              key={index}
              role="img"
              aria-label={`${row.label}, ${bar.tick ? `one message at ${bar.text}` : bar.text}`}
              className={bar.pieces ? 'xt-human-stretch' : 'xt-human-bar'}
              data-tick={bar.tick || undefined}
              data-testid="human-stretch"
              style={
                bar.pieces
                  ? undefined
                  : {
                      left: `${bar.left * 100}%`,
                      width: bar.tick ? undefined : `${bar.width * 100}%`,
                    }
              }
            >
              {bar.pieces?.map((piece, part) => (
                <span
                  key={part}
                  aria-hidden="true"
                  className="xt-human-bar"
                  data-lane={piece.lane}
                  data-tick={bar.tick || undefined}
                  style={{
                    left: `${piece.left * 100}%`,
                    width: bar.tick ? undefined : `${piece.width * 100}%`,
                  }}
                />
              ))}
            </span>
          ))}
        </span>
        <span className="xt-human-total">{row.total ?? '—'}</span>
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side={lower ? 'top' : 'bottom'}
          align="center"
          sideOffset={4}
          collisionPadding={8}
        >
          <Tooltip.Popup className="xt-rule-popover xt-effort-tip" aria-hidden="true">
            <span className="xt-effort-tip-head">
              <span>{row.label}</span>
              <b>{row.total ?? 'unknown'}</b>
            </span>
            {row.bars.length === 0 && (
              <span className="xt-effort-tip-note">
                {row.total === null ? 'Could not tell which messages a person sent' : 'No messages'}
              </span>
            )}
            {row.bars.map((bar, index) => (
              <span key={index} className="xt-effort-tip-row">
                <span>{bar.text}</span>
                <em>{bar.tick ? 'one message' : ''}</em>
              </span>
            ))}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
