import { Popover } from '@base-ui/react/popover';
import { Tooltip } from '@base-ui/react/tooltip';
import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from 'react';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import { UNMEASURED } from '../../kit/format';
import { Icon } from '../../kit/icons';
import type { TimeRange } from '../../kit/TopBar';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { agentTime } from '../agent-duration';
import { leverageText } from '../metric-format';
import { DaySessions } from './DaySessions';
import {
  QUARTERS,
  LINE_HEIGHT,
  LINE_WIDTH,
  AT_OR_BELOW_EVEN,
  agentLines,
  timelineRows,
  type AgentLine,
  type TimelineRow,
} from './overview';
import '../../styles/overview.css';

/** The 7-day range's eight days fill the plot; longer ranges keep that row size and scroll. */
export const VISIBLE_ROWS = 8;
/** Below this a row's bar and its gap stop reading as separate rows. */
const MIN_ROW_PX = 28;
/** The totals columns whose widths follow their widest text. */
const TOTAL_COLUMNS = ['you', 'agents', 'leverage'] as const;
/** With the agents line: a taller track, so the line's height reads. */
const MIN_PAIRED_ROW_PX = 34;
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
 * a focused day opens its card, as hovering does. In "human + agent" a day's
 * date also opens a box of its sessions, each a link: hovering the date opens
 * it, and Enter on the focused day moves focus into it; Escape returns to the
 * day.
 */
export function HumanTimeline({
  report,
  range,
  withAgents = false,
}: {
  report: DashboardMetrics;
  /** The Dashboard's range, which a day card's session links carry. */
  range: TimeRange;
  /**
   * The "human + agent" view: each day's agent time as a thin line across
   * its track, as high as the agents running on average in each wall-clock
   * quarter-hour on the report's one scale, over soft bands where you were
   * working.
   */
  withAgents?: boolean;
}) {
  const human = report.human_hours.current;
  const rows = useMemo(() => timelineRows(human.by_day, report.window), [human, report.window]);
  const lines = useMemo(
    () =>
      withAgents ? agentLines(report.leverage.by_day, report.leverage.agents_scale_top) : null,
    [withAgents, report.leverage],
  );
  const minRow = withAgents ? MIN_PAIRED_ROW_PX : MIN_ROW_PX;
  const root = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  // No total is capped: a busy day's agent hours or a tiny day's leverage
  // can be far wider than usual. Each totals column is as wide as its
  // widest text in any row, never narrower than its usual width (in CSS);
  // the axis and every row read the same widths, so they stay aligned.
  useLayoutEffect(() => {
    const element = root.current;
    if (!withAgents || !element) return;
    let live = true;
    const fit = () => {
      if (!live) return;
      for (const key of TOTAL_COLUMNS) {
        const widths = [
          ...element.querySelectorAll<HTMLElement>(
            `.xt-human-total[data-key="${key}"] > .xt-human-total-text`,
          ),
        ].map((text) => text.getBoundingClientRect().width);
        element.style.setProperty(`--${key}-col`, `${Math.ceil(Math.max(0, ...widths))}px`);
      }
    };
    fit();
    // Measured again once the mono font has loaded.
    void document.fonts?.ready.then(fit);
    return () => {
      live = false;
    };
  }, [withAgents, rows, lines]);
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
    // Keys pressed inside a day's sessions box (it is portalled, but its
    // events still pass through here) are the box's own.
    if (!(event.target instanceof HTMLElement) || !event.target.classList.contains('xt-human-row'))
      return;
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
      setRowHeight(Math.max(minRow, Math.floor(element.clientHeight / VISIBLE_ROWS)));
    fit();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, [minRow]);
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
    <div
      ref={root}
      className="xt-effort xt-human"
      data-testid="human-timeline"
      data-variant={withAgents ? 'paired' : undefined}
    >
      {withAgents ? (
        <PairedHeadline report={report} />
      ) : (
        <p className="xt-effort-headline" data-testid="effort-headline">
          <strong data-testid="effort-total">
            {unknown ? 'human time unknown' : `${agentTime(human.active_ms!)} human`}
          </strong>
          {unknown && <small>Could not tell which messages a person sent.</small>}
          <small className="xt-effort-headline-end">
            break threshold: {human.break_minutes} min
          </small>
        </p>
      )}
      <div className="xt-human-plot">
        <div className="xt-human-axis" aria-hidden="true">
          <span />
          <span className="xt-human-hours">
            {HOURS.map((hour) => (
              <span key={hour}>{hour}</span>
            ))}
          </span>
          {withAgents ? (
            <>
              <span className="xt-human-total-head" data-testid="human-total-head">
                <Icon name="person" size={13} />
              </span>
              <span
                className="xt-human-total-head"
                data-key="agents"
                data-testid="agent-total-head"
              >
                <Icon name="bot" size={13} />
              </span>
              <span
                className="xt-human-total-head"
                data-key="leverage"
                data-testid="leverage-total-head"
              >
                <Icon name="bolt" size={13} />
              </span>
            </>
          ) : (
            <span />
          )}
        </div>
        <div
          ref={body}
          className="xt-human-body"
          role="list"
          aria-label={`${withAgents ? 'Human and agent time' : 'Human time'} by day, ${rows.length} days, each from 00:00 to 24:00`}
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
              line={lines?.get(row.date) ?? (lines ? null : undefined)}
              lower={index >= rows.length / 2}
              current={index === current}
              onFocus={() => setChosen(row.date)}
              window={report.window}
              range={range}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

/**
 * The "human + agent" headline: your time and the agents' over the same
 * whole days, each behind a key drawn like its line, then the leverage tile's
 * own value. Unknown human time says so, as the human h headline does.
 */
function PairedHeadline({ report }: { report: DashboardMetrics }) {
  const human = report.human_hours.current;
  const unknown = human.active_ms === null;
  const leverage = report.tiles.leverage.value;
  return (
    <p className="xt-effort-headline xt-paired-headline" data-testid="effort-headline">
      <span className="xt-paired-stat">
        <span className="xt-paired-key" data-key="you" role="img" aria-label="Your time">
          <Icon name="person" size={16} />
          <span className="xt-paired-swatch" data-key="band" />
        </span>
        <strong data-testid="effort-total">
          {unknown ? 'human time unknown' : agentTime(human.active_ms!)}
        </strong>
      </span>
      <span className="xt-paired-stat">
        <span className="xt-paired-key" data-key="agents" role="img" aria-label="Agent time">
          <Icon name="bot" size={16} />
          <svg className="xt-paired-swatch" data-key="line" viewBox="0 0 22 12" focusable="false">
            <path d="M0,10 L5,9 L8,4 L12,6 L16,2 L22,7" />
          </svg>
        </span>
        <strong data-testid="effort-agent-total">{agentTime(report.leverage.agent_ms)}</strong>
      </span>
      {unknown && <small>Could not tell which messages a person sent.</small>}
      <span className="xt-paired-stat" data-stat="leverage">
        <strong data-testid="effort-leverage">
          {leverage === null ? UNMEASURED : leverageText(leverage)}
        </strong>
        <small>agent time per hour of yours</small>
      </span>
    </p>
  );
}

/** Within two pixels of the end: the newest row is in view. */
const isAtBottom = (element: HTMLElement) =>
  element.scrollTop + element.clientHeight >= element.scrollHeight - 2;

/**
 * One day: a list item whose name lists every stretch, and whose card opens
 * on hover or focus. Only the current day is in the tab order; arrow keys
 * move between days. Each bar also names its own times.
 *
 * In "human + agent" the date also opens the day's sessions box, a list of
 * links, on hover; the pointer can move from the date into it. Enter or
 * Space on the focused day opens it with focus on its first session, and
 * Escape closes it and returns focus to the day. A click on the date does
 * nothing, so it never leaves a box pinned open. Only one of the day's two
 * boxes shows at a time: the card waits until the sessions box has gone.
 */
function Row({
  row,
  line,
  lower,
  current,
  onFocus,
  window,
  range,
}: {
  row: TimelineRow;
  /** The day's agents line in the "human + agent" view; `null` there when the report has none. */
  line?: AgentLine | null;
  lower: boolean;
  current: boolean;
  onFocus: () => void;
  /** The report's window, which names the day's sessions read. */
  window: DashboardWindow;
  /** The Dashboard's range, which the day's session links carry. */
  range: TimeRange;
}) {
  const theme = useSurfaceTheme();
  const paired = line !== undefined;
  /** The quarter under the pointer, and where along the track it is, in pixels. */
  const [pointer, setPointer] = useState<{ quarter: number; x: number } | null>(null);
  /** The pointer is on the track: the hairline shows. */
  const [onTrack, setOnTrack] = useState(false);
  /** Hover or focus asks for the day's card. */
  const [card, setCard] = useState(false);
  /** The sessions box is open. */
  const [sessions, setSessions] = useState(false);
  /** The sessions box is open or still closing; the card waits until it has gone. */
  const [sessionsShown, setSessionsShown] = useState(false);
  const day = useRef<HTMLElement | null>(null);
  const dayRef = useCallback((element: HTMLElement | null) => {
    day.current = element;
  }, []);
  const popup = useRef<HTMLDivElement | null>(null);
  /** Enter asked for focus on the first session, which may not be read yet. */
  const entering = useRef(false);
  const enter = (box: HTMLElement) => {
    const link = box.querySelector<HTMLElement>('a[href]');
    if (link) {
      entering.current = false;
      link.focus();
    } else box.focus();
  };
  const popupRef = useCallback((element: HTMLDivElement | null) => {
    popup.current = element;
    if (element && entering.current) enter(element);
  }, []);
  // The sessions arrive after the box opened from the keyboard: focus moves
  // from the box onto the first of them.
  const listRef = useCallback((list: HTMLElement | null) => {
    if (list && entering.current && document.activeElement === popup.current) enter(list);
  }, []);
  const track = (
    <span
      className="xt-human-track"
      data-fold={row.fold || undefined}
      onPointerMove={
        paired
          ? (event) => {
              const box = event.currentTarget.getBoundingClientRect();
              if (box.width <= 0) return;
              const x = Math.max(0, Math.min(box.width - 1, event.clientX - box.left));
              setPointer({ quarter: Math.floor((x / box.width) * QUARTERS), x });
              setOnTrack(true);
            }
          : undefined
      }
      onPointerLeave={
        paired
          ? (event) => {
              setOnTrack(false);
              // Onto the day's totals the card stays open and drops the
              // quarter now. Off the day, or onto its date, the card closes
              // and keeps it until it has gone: a card whose content changed
              // as it closed would resize under the positioner's own size
              // watcher (a ResizeObserver loop in WebKit).
              const to = event.relatedTarget;
              if (
                to instanceof Element &&
                event.currentTarget.parentElement?.contains(to) &&
                !to.closest('.xt-human-day')
              )
                setPointer(null);
            }
          : undefined
      }
    >
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
      {paired && line?.path && (
        <svg
          className="xt-agent-line"
          data-testid="agent-line"
          viewBox={`0 0 ${LINE_WIDTH} ${LINE_HEIGHT}`}
          preserveAspectRatio="none"
          aria-hidden="true"
          focusable="false"
        >
          <path d={line.path} vectorEffect="non-scaling-stroke" />
        </svg>
      )}
      {pointer && onTrack && (
        <span
          className="xt-human-hair"
          data-testid="agent-hairline"
          aria-hidden="true"
          style={{ left: `${pointer.x}px` }}
        />
      )}
    </span>
  );
  const name = paired
    ? `${row.name}; agents: ${line ? `${line.spoken}${line.busiest ? `, busiest 15 minutes at ${line.busiest}` : ''}` : 'unknown'}; agent time per hour of yours: ${line?.leverageWords ?? 'unknown'}; Enter opens the day’s sessions`
    : row.name;
  const sessionsName = `Sessions on ${row.label}`;
  const dayCard = (
    <Tooltip.Root
      disableHoverablePopup
      open={card && !sessionsShown}
      onOpenChange={(open) => setCard(open)}
      onOpenChangeComplete={(open) => {
        if (!open) setPointer(null);
      }}
    >
      <Tooltip.Trigger
        ref={dayRef}
        delay={0}
        closeOnClick={false}
        render={
          <div
            role="listitem"
            tabIndex={current ? 0 : -1}
            onFocus={onFocus}
            onKeyDown={
              paired
                ? (event) => {
                    if (event.target !== event.currentTarget) return;
                    if (event.key !== 'Enter' && event.key !== ' ') return;
                    event.preventDefault();
                    entering.current = true;
                    // An open box takes focus now; a closed (or closing) one
                    // opens and takes it once shown.
                    if (sessions && popup.current) enter(popup.current);
                    else {
                      setSessionsShown(true);
                      setSessions(true);
                    }
                  }
                : undefined
            }
            aria-label={name}
            className="xt-human-row"
            data-testid="human-day"
            data-date={row.date}
          />
        }
      >
        {paired ? (
          <Popover.Trigger
            nativeButton={false}
            openOnHover
            delay={0}
            // Not a tab stop: the day is, and Enter on it opens this box.
            tabIndex={-1}
            aria-label={sessionsName}
            // Focus that lands on the date (a click, or Shift+Tab out of the
            // box) belongs to the day.
            onFocus={() => day.current?.focus()}
            render={<span className="xt-human-day" data-weekend={row.weekend || undefined} />}
          >
            {row.label}
          </Popover.Trigger>
        ) : (
          <span className="xt-human-day" data-weekend={row.weekend || undefined}>
            {row.label}
          </span>
        )}
        {track}
        <span
          className="xt-human-total"
          data-key={paired ? 'you' : undefined}
          data-unknown={paired && row.total === null ? '' : undefined}
        >
          {paired ? (
            <span className="xt-human-total-text">{line?.yourHours ?? UNMEASURED}</span>
          ) : (
            (row.total ?? '—')
          )}
        </span>
        {paired && (
          <span className="xt-human-total" data-key="agents" data-testid="agent-row-total">
            <span className="xt-human-total-text">{line ? line.hours : UNMEASURED}</span>
          </span>
        )}
        {paired && (
          <span
            className="xt-human-total"
            data-key="leverage"
            data-unknown={line?.leverage ? undefined : ''}
            data-low={line?.atOrBelowEven ? '' : undefined}
            data-testid="leverage-row-total"
          >
            <span className="xt-human-total-text">{line?.leverage ?? UNMEASURED}</span>
          </span>
        )}
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
            {paired && (
              <>
                <span className="xt-effort-tip-divider" />
                <span className="xt-effort-tip-row" data-agent="">
                  <span>Agent time</span>
                  <b data-testid="agent-day-total">{line ? line.total : 'unknown'}</b>
                </span>
                {line?.busiest && (
                  <span className="xt-effort-tip-row">
                    <span>Busiest 15 min</span>
                    <em>{line.busiest}</em>
                  </span>
                )}
                <span
                  className="xt-effort-tip-row"
                  data-low={line?.atOrBelowEven ? '' : undefined}
                  data-testid="leverage-day"
                >
                  <span>Agent time per hour of yours</span>
                  <b>{line?.leverage ?? line?.leverageWords ?? 'unknown'}</b>
                  {line?.atOrBelowEven && <em>{AT_OR_BELOW_EVEN}</em>}
                </span>
                {line && pointer && (
                  <span className="xt-effort-tip-row" data-pointer="" data-testid="agent-quarter">
                    <span>{line.quarter(pointer.quarter).span}</span>
                    <em>{line.quarter(pointer.quarter).agents}</em>
                  </span>
                )}
              </>
            )}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
  if (!paired) return dayCard;
  return (
    <Popover.Root
      open={sessions}
      onOpenChange={(open, details) => {
        // A click (or tap) on the date neither opens, pins nor closes the
        // box: a pinned box would stay open beside the next day's. Hover
        // opens it, and Enter or Space on the day for the keyboard.
        if (details.reason === 'trigger-press') {
          details.cancel();
          return;
        }
        if (open) setSessionsShown(true);
        else {
          entering.current = false;
          // Escape from inside the box hands focus back to the day.
          if (details.reason === 'escape-key' && popup.current?.contains(document.activeElement))
            day.current?.focus();
        }
        setSessions(open);
      }}
      onOpenChangeComplete={(open) => {
        // A box reopened before it had gone keeps its element: focus moves
        // in here instead of when the element appears.
        if (open && entering.current && popup.current) enter(popup.current);
        if (!open) setSessionsShown(false);
      }}
    >
      {dayCard}
      <Popover.Portal data-theme={theme}>
        {/* Beside the date, over the track, and never flipped back over the
            dates: a box above or below its date would cover the next date
            and keep the pointer moving down the dates from reaching it. It
            grows away from the nearer end of the list and shifts only up or
            down to fit. */}
        <Popover.Positioner
          className="xt-rule-positioner xt-day-sessions-positioner"
          positionMethod="fixed"
          side="inline-end"
          align={lower ? 'end' : 'start'}
          sideOffset={4}
          collisionPadding={8}
          collisionAvoidance={{ side: 'none', align: 'shift', fallbackAxisSide: 'none' }}
        >
          <Popover.Popup
            ref={popupRef}
            className="xt-rule-popover xt-effort-tip"
            aria-label={sessionsName}
            initialFocus={false}
            finalFocus={false}
          >
            <DaySessions
              window={window}
              date={row.date}
              label={row.label}
              range={range}
              listRef={listRef}
            />
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
