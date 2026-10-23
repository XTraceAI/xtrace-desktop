import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router';
import { DataProvider } from '../../data/DataProvider';
import type { SessionTitleControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import { events } from '../../data/ipc-names';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { HumanTimeline } from './HumanTimeline';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
// Set the test Mac's default locale explicitly; production still reads its own.
const NativeDateTimeFormat = Intl.DateTimeFormat;
beforeEach(() => {
  class TestDateTimeFormat extends NativeDateTimeFormat {
    constructor(locales?: Intl.LocalesArgument, options?: Intl.DateTimeFormatOptions) {
      super(locales ?? 'en-US', options);
    }
  }
  vi.stubGlobal('Intl', Object.assign(Object.create(Intl), { DateTimeFormat: TestDateTimeFormat }));
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const exported = () => structuredClone(fixture as FixtureExport);
/** Where a session link landed. */
function SessionPath() {
  const location = useLocation();
  return <p data-testid="session-path">{`${location.pathname}${location.search}`}</p>;
}

const H = 3_600_000;
/** F1's 7-day report with synthetic stretches (not real history) on its last two days. */
function report(edit?: (report: DashboardMetrics) => void) {
  const out = structuredClone(
    (fixture as FixtureExport).dashboards.find((d) => d.window.days === 7)!,
  );
  const days = out.human_hours.current.by_day;
  for (const day of days) {
    day.active_ms = 0;
    day.stretches = [];
  }
  const [sixth, last] = days.slice(-2) as [(typeof days)[0], (typeof days)[0]];
  sixth.stretches = [{ start_ms: sixth.end_ms - H, end_ms: sixth.end_ms }];
  sixth.active_ms = H;
  last.stretches = [
    { start_ms: last.start_ms, end_ms: last.start_ms + 0.5 * H },
    { start_ms: last.start_ms + 2 * H, end_ms: last.start_ms + 2 * H },
  ];
  last.active_ms = 0.5 * H;
  out.human_hours.current.active_ms = 1.5 * H;
  out.human_hours.current.break_minutes = 45;
  edit?.(out);
  return out;
}
const mount = (r: DashboardMetrics) =>
  render(
    <ThemeProvider>
      <HumanTimeline report={r} range="7d" />
    </ThemeProvider>,
  );

it('draws one 24-hour row per day with each stretch, ticks for single messages and the day total', () => {
  mount(report());
  expect(screen.getByTestId('effort-total').textContent).toBe('1 h 30 m human');
  // The total, then the break setting it depends on at the row's right end.
  expect(screen.getByTestId('effort-headline').textContent).toBe(
    '1 h 30 m humanbreak threshold: 45 min',
  );
  expect(screen.getByText('break threshold: 45 min').className).toBe('xt-effort-headline-end');
  const rows = screen.getAllByTestId('human-day');
  expect(rows).toHaveLength(7);
  expect(rows.map((row) => row.dataset.date)).toEqual(
    report().human_hours.current.by_day.map((day) => day.date),
  );
  const [sixth, last] = rows.slice(-2) as [HTMLElement, HTMLElement];
  // A stretch cut at midnight runs to midnight, and the next day's piece starts at midnight.
  expect(sixth.getAttribute('aria-label')).toBe('Sep 6: 1 hour 0 minutes; 11:00 PM–12:00 AM');
  expect(last.getAttribute('aria-label')).toBe(
    'Sep 7: 30 minutes; 12:00 AM–12:30 AM, one message at 2:00 AM',
  );
  const bars = within(last).getAllByRole('img');
  expect(bars.map((bar) => bar.getAttribute('aria-label'))).toEqual([
    'Sep 7, 12:00 AM–12:30 AM',
    'Sep 7, one message at 2:00 AM',
  ]);
  expect(bars[0]!.style.left).toBe('0%');
  expect(bars[1]!.dataset.tick).toBe('true');
  expect(bars[1]!.style.width).toBe('');
  // A measured zero is a zero, never a dash: a dash means unknown.
  expect(rows[0]!.textContent).toBe('Sep 10 h 0 m');
  expect(last.textContent).toBe('Sep 70 h 30 m');
  // The hour axis is drawn once, above the rows.
  expect(screen.getByTestId('human-timeline').querySelector('.xt-human-axis')!.textContent).toBe(
    '0006121824',
  );
});

it('draws a row for every whole day human time cover, and names no range', () => {
  mount(
    report((r) => {
      const days = r.human_hours.current.by_day;
      days.push({ ...days.at(-1)!, date: '2026-09-08' });
    }),
  );
  expect(screen.getAllByTestId('human-day')).toHaveLength(8);
  // A 7-day range that starts mid-day: human time covers eight whole days,
  // one row each; the headline names no range.
  const headline = screen.getByTestId('effort-headline').textContent;
  expect(headline).not.toContain('whole days');
  expect(headline).not.toContain('last 7 days');
});

it('says human time is unknown instead of drawing a smaller number', () => {
  mount(
    report((r) => {
      r.human_hours.current.active_ms = null;
      for (const day of r.human_hours.current.by_day) {
        day.active_ms = null;
        day.stretches = [];
      }
    }),
  );
  expect(screen.getByTestId('effort-total').textContent).toBe('human time unknown');
  expect(screen.getByTestId('effort-headline').textContent).toContain(
    'Could not tell which messages a person sent.',
  );
  expect(screen.queryAllByTestId('human-stretch')).toHaveLength(0);
  expect(screen.getAllByTestId('human-day').map((row) => row.textContent?.slice(-1))).toEqual(
    Array(7).fill('—'),
  );
});

it('keeps a folded stretch as one accessible image and tooltip entry with two drawn pieces', async () => {
  const folded = report((r) => {
    r.window.timezone = 'America/Los_Angeles';
    r.human_hours.current.active_ms = 20 * 60_000;
    r.human_hours.current.by_day = [
      {
        date: '2026-11-01',
        start_ms: Date.parse('2026-11-01T00:00:00-07:00'),
        end_ms: Date.parse('2026-11-02T00:00:00-08:00'),
        active_ms: 20 * 60_000,
        stretches: [
          {
            start_ms: Date.parse('2026-11-01T01:50:00-07:00'),
            end_ms: Date.parse('2026-11-01T01:10:00-08:00'),
          },
        ],
      },
    ];
  });
  const before = structuredClone(folded);
  mount(folded);
  const images = screen.getAllByRole('img');
  expect(images).toHaveLength(1);
  expect(images[0]!.getAttribute('aria-label')).toBe('Nov 1, 1:50 AM PDT → 1:10 AM PST — 20 min');
  const pieces = images[0]!.querySelectorAll<HTMLElement>('.xt-human-bar');
  expect(pieces).toHaveLength(2);
  expect(pieces[0]!.dataset.lane).toBe('0');
  expect(pieces[1]!.dataset.lane).toBe('1');
  for (const piece of pieces)
    expect(parseFloat(piece.style.width)).toBeCloseTo((10 / (24 * 60)) * 100);
  expect(screen.getByTestId('effort-total').textContent).toBe('0 h 20 m human');
  fireEvent.focus(screen.getByTestId('human-day'));
  await screen.findByText('1:50 AM PDT → 1:10 AM PST — 20 min');
  expect(document.querySelectorAll('.xt-effort-tip-row')).toHaveLength(1);
  expect(folded).toEqual(before);
});

/** A range's report from F1, unchanged. */
const range = (days: number) =>
  structuredClone((fixture as FixtureExport).dashboards.find((d) => d.window.days === days)!);

/**
 * jsdom lays nothing out: give every element a fixed scroll area and a
 * scroll position that keeps what it is set to, within its bounds.
 */
function scrollArea(clientHeight: number, scrollHeight: number) {
  const top = new WeakMap<Element, number>();
  const proto = HTMLElement.prototype;
  const saved = ['clientHeight', 'scrollHeight', 'scrollTop'].map(
    (key) => [key, Object.getOwnPropertyDescriptor(proto, key)] as const,
  );
  Object.defineProperty(proto, 'clientHeight', { configurable: true, get: () => clientHeight });
  Object.defineProperty(proto, 'scrollHeight', { configurable: true, get: () => scrollHeight });
  Object.defineProperty(proto, 'scrollTop', {
    configurable: true,
    get(this: Element) {
      return top.get(this) ?? 0;
    },
    set(this: Element, value: number) {
      top.set(this, Math.max(0, Math.min(value, scrollHeight - clientHeight)));
    },
  });
  return () => {
    for (const [key, descriptor] of saved)
      if (descriptor) Object.defineProperty(proto, key, descriptor);
      else delete (proto as unknown as Record<string, unknown>)[key];
  };
}

it('keeps your scroll position when the same range refreshes, and opens a new range on today', () => {
  const restore = scrollArea(400, 1600);
  try {
    const month = range(30);
    const view = mount(month);
    const body = screen.getByTestId('human-timeline-body');
    // It opens on today, at the bottom.
    expect(body.scrollTop).toBe(1200);
    body.scrollTop = 200;
    fireEvent.scroll(body);
    // A live update: a new report object for the same range, a second later.
    const refreshed = structuredClone(month);
    refreshed.window.end_ms += 1000;
    view.rerender(
      <ThemeProvider>
        <HumanTimeline report={refreshed} range="30d" />
      </ThemeProvider>,
    );
    expect(body.scrollTop).toBe(200);
    // Another range opens on today again.
    view.rerender(
      <ThemeProvider>
        <HumanTimeline report={range(7)} range="7d" />
      </ThemeProvider>,
    );
    expect(screen.getByTestId('human-timeline-body').scrollTop).toBe(1200);
  } finally {
    restore();
  }
});

it('is one tab stop, today at first; arrow keys move between days and open their card', () => {
  mount(report());
  const timeline = screen.getByTestId('human-timeline');
  const stops = () =>
    [...timeline.querySelectorAll<HTMLElement>('[tabindex]')].filter(
      (element) => element.tabIndex >= 0,
    );
  const rows = screen.getAllByRole('listitem');
  expect(rows).toHaveLength(7);
  expect(stops()).toEqual([rows[6]]);
  // Every day and stretch is still named.
  expect(rows[6]!.getAttribute('aria-label')).toBe(
    'Sep 7: 30 minutes; 12:00 AM–12:30 AM, one message at 2:00 AM',
  );
  expect(within(rows[6]!).getAllByRole('img')).toHaveLength(2);
  rows[6]!.focus();
  fireEvent.keyDown(rows[6]!, { key: 'ArrowUp' });
  expect(document.activeElement).toBe(rows[5]);
  expect(stops()).toEqual([rows[5]]);
  fireEvent.keyDown(rows[5]!, { key: 'Home' });
  expect(document.activeElement).toBe(rows[0]);
  fireEvent.keyDown(rows[0]!, { key: 'ArrowUp' });
  expect(document.activeElement).toBe(rows[0]);
  fireEvent.keyDown(rows[0]!, { key: 'End' });
  expect(document.activeElement).toBe(rows[6]);
  expect(stops()).toEqual([rows[6]]);
});

const Q = 15 * 60_000;
/** The "human + agent" report: synthetic agent time (not real history) on the last day. */
function paired(edit?: (report: DashboardMetrics) => void) {
  return report((r) => {
    for (const day of r.leverage.by_day) {
      day.agent_ms = 0;
      day.quarter_agents = Array.from({ length: 96 }, () => 0);
      day.busiest_quarter = null;
      day.value = null;
      day.at_or_below_even = null;
      // Your time on the leverage day is the row's: measured as zero.
      day.human_ms = 0;
    }
    const last = r.leverage.by_day.at(-1)!;
    // 00:00–00:30 one agent, 00:30–00:45 three, 08:00 a fifth of one.
    for (const [index, agents] of [
      [0, 1],
      [1, 1],
      [2, 3],
      [32, 0.2],
    ] as const)
      last.quarter_agents[index] = agents;
    last.agent_ms = 5 * Q + Q / 5;
    last.busiest_quarter = 2;
    // The report's own leverage for the day: 78 agent minutes per 30 of yours.
    last.value = 2.6;
    last.at_or_below_even = false;
    last.human_ms = 0.5 * H;
    r.leverage.agent_ms = last.agent_ms;
    r.tiles.leverage = { ...r.tiles.leverage, value: 3.4 };
    edit?.(r);
  });
}
/**
 * The "human + agent" view reads each open day's sessions, so it is mounted
 * with F1's fixture source (or `source`) and a router for the session links.
 */
async function mountPaired(r: DashboardMetrics, source = new FixtureDataSource(exported())) {
  const view = render(
    <MemoryRouter initialEntries={['/dashboard']}>
      <ThemeProvider>
        <DataProvider source={source}>
          <Routes>
            <Route path="/dashboard" element={<HumanTimeline report={r} range="7d" withAgents />} />
            <Route path="/sessions/:sessionId" element={<SessionPath />} />
          </Routes>
        </DataProvider>
      </ThemeProvider>
    </MemoryRouter>,
  );
  await screen.findAllByTestId('human-day');
  return view;
}

it('human + agent: the headline sets your time beside the agents’ and the leverage', async () => {
  await mountPaired(paired());
  expect(screen.getByTestId('human-timeline').dataset.variant).toBe('paired');
  expect(screen.getByTestId('effort-headline').textContent).toBe(
    '1 h 30 m1 h 18 m3.4×agent time per hour of yours',
  );
  // No break threshold here, and no colour legend.
  expect(screen.queryByText(/break threshold/)).toBeNull();
});

it('human + agent: one track per day, your bands and the agents line in it, your total and the agents’ beside it', async () => {
  await mountPaired(paired());
  const rows = screen.getAllByTestId('human-day');
  expect(rows).toHaveLength(7);
  for (const row of rows) expect(row.querySelectorAll('.xt-human-track')).toHaveLength(1);
  // A day with no agent time draws no line.
  const lines = screen.getAllByTestId('agent-line');
  expect(lines).toHaveLength(1);
  expect(rows[6]!.contains(lines[0]!)).toBe(true);
  expect(lines[0]!.getAttribute('viewBox')).toBe('0 0 960 26');
  // 3 agents on the report's scale of 10: 30% of the way up (25 → 18.1);
  // the 08:00 fifth of one 2% (24.54). An open, unfilled line.
  const path = lines[0]!.querySelector('path')!;
  expect(path.getAttribute('d')).toContain(' 25,18.1');
  expect(path.getAttribute('d')).toContain(' 325,24.54');
  expect(path.getAttribute('d')).not.toContain('Z');
  expect(path.getAttribute('vector-effect')).toBe('non-scaling-stroke');
  // The line is drawn over your bands.
  const track = rows[6]!.querySelector('.xt-human-track')!;
  expect(track.lastElementChild).toBe(lines[0]);
  expect(track.querySelectorAll('[data-testid="human-stretch"]')).toHaveLength(2);
  // Your total, then the agents' (the report's whole-day agent time), in
  // decimal hours on the row; the card keeps hours and minutes.
  expect(
    [...rows[6]!.querySelectorAll('.xt-human-total')].map((total) => total.textContent),
  ).toEqual(['0.5 h', '1.3 h', '2.6×']);
  expect(rows[6]!.textContent).toBe('Sep 70.5 h1.3 h2.6×');
  // A day with no leverage says so in the meta colour, never a computed 0.
  const leverage = screen.getAllByTestId('leverage-row-total');
  expect(leverage.map((cell) => cell.textContent)).toEqual([...Array(6).fill('—'), '2.6×']);
  expect(leverage[0]!.dataset.unknown).toBe('');
  expect(leverage[6]!.dataset.unknown).toBeUndefined();
  expect(screen.getByTestId('leverage-total-head').querySelector('svg')).not.toBeNull();
  expect(screen.getAllByTestId('agent-row-total').map((total) => total.textContent)).toEqual([
    ...Array(6).fill('0 h'),
    '1.3 h',
  ]);
  expect(screen.queryAllByTestId('agent-day-total')).toHaveLength(0);
  expect(rows[6]!.getAttribute('aria-label')).toBe(
    'Sep 7: 30 minutes; 12:00 AM–12:30 AM, one message at 2:00 AM; agents: 1 hour 18 minutes, busiest 15 minutes at 12:30 AM · 3 agents; agent time per hour of yours: 2.6×; Enter opens the day’s sessions',
  );
  // The person icon heads your column, the robot the agents'.
  expect(screen.getByTestId('human-total-head').querySelector('svg')).not.toBeNull();
  expect(screen.getByTestId('agent-total-head').querySelector('svg')).not.toBeNull();
});

it('human + agent: the headline keys are named icons, without the words you and agents', async () => {
  await mountPaired(paired());
  expect(screen.getByRole('img', { name: 'Your time' }).querySelector('svg')).not.toBeNull();
  expect(screen.getByRole('img', { name: 'Agent time' }).querySelector('svg')).not.toBeNull();
  const headline = screen.getByTestId('effort-headline').textContent!;
  expect(headline).not.toMatch(/\byou\b/);
  expect(headline).not.toMatch(/\bagents\b/);
});

it('human + agent: a taller quarter than the scale is drawn at the top', async () => {
  await mountPaired(paired((r) => (r.leverage.by_day.at(-1)!.quarter_agents[2] = 14)));
  expect(screen.getByTestId('agent-line').querySelector('path')!.getAttribute('d')).toContain(
    ' 25,2',
  );
});

it('human + agent: the day card adds agent time, the busiest quarter and the quarter under the pointer', async () => {
  await mountPaired(paired());
  const last = screen.getAllByTestId('human-day').at(-1)!;
  // Keyboard focus: the day's card without a pointer line, and no sessions.
  fireEvent.focus(last);
  await screen.findByText('Busiest 15 min');
  expect(screen.getByText('12:30 AM · 3 agents')).toBeTruthy();
  expect(screen.getByText('Agent time').nextSibling!.textContent).toBe('1 h 18 m');
  expect(screen.getByTestId('agent-day-total').textContent).toBe('1 h 18 m');
  expect(screen.getByTestId('leverage-day').textContent).toBe('Agent time per hour of yours2.6×');
  expect(screen.queryByTestId('agent-quarter')).toBeNull();
  expect(screen.queryByTestId('agent-hairline')).toBeNull();
  expect(screen.queryByTestId('day-sessions-head')).toBeNull();
  expect(screen.queryByRole('dialog')).toBeNull();
  // The pointer over 08:00–08:15 (a third of the track) names that quarter.
  const lane = last.querySelector<HTMLElement>('.xt-human-track')!;
  lane.getBoundingClientRect = () => ({ left: 0, width: 960, top: 0 }) as DOMRect;
  fireEvent.pointerMove(lane, { clientX: 321 });
  expect(screen.getByTestId('agent-hairline').style.left).toBe('321px');
  expect(screen.getByTestId('agent-quarter').textContent).toBe('8:00 AM–8:15 AM0.2 agents');
  fireEvent.pointerMove(lane, { clientX: 500 });
  expect(screen.getByTestId('agent-quarter').textContent).toBe('12:30 PM–12:45 PMno agents');
  // Onto the day's totals the card stays open and drops the quarter.
  fireEvent.pointerLeave(lane, { relatedTarget: within(last).getByTestId('agent-row-total') });
  expect(screen.queryByTestId('agent-hairline')).toBeNull();
  expect(screen.queryByTestId('agent-quarter')).toBeNull();
  // Off the day the card closes: it keeps the quarter until it has gone.
  fireEvent.pointerMove(lane, { clientX: 321 });
  fireEvent.pointerLeave(lane, { relatedTarget: document.body });
  expect(screen.queryByTestId('agent-hairline')).toBeNull();
  expect(screen.getByTestId('agent-quarter').textContent).toBe('8:00 AM–8:15 AM0.2 agents');
  fireEvent.blur(last);
  await waitFor(() => expect(screen.queryByText('Busiest 15 min')).toBeNull());
  fireEvent.focus(last);
  await screen.findByText('Busiest 15 min');
  expect(screen.queryByTestId('agent-quarter')).toBeNull();
});

it('human + agent: a day without leverage says none when your time was zero, unknown when it is unknown', async () => {
  await mountPaired(
    paired((r) => {
      // Sep 1: your time unknown that day; the others measured as zero.
      r.human_hours.current.by_day[0]!.active_ms = null;
      r.leverage.by_day[0]!.human_ms = null;
    }),
  );
  const rows = screen.getAllByTestId('human-day');
  const cells = screen.getAllByTestId('leverage-row-total');
  // The column draws a dash for both.
  expect(cells[0]!.textContent).toBe('—');
  expect(cells[1]!.textContent).toBe('—');
  expect(rows[0]!.getAttribute('aria-label')).toMatch(
    /; agent time per hour of yours: unknown; Enter opens the day’s sessions$/,
  );
  expect(rows[1]!.getAttribute('aria-label')).toMatch(
    /; agent time per hour of yours: none; Enter opens the day’s sessions$/,
  );
  fireEvent.focus(rows[1]!);
  await waitFor(() =>
    expect(screen.getByTestId('leverage-day').textContent).toBe('Agent time per hour of yoursnone'),
  );
  fireEvent.blur(rows[1]!);
  fireEvent.focus(rows[0]!);
  await waitFor(() =>
    expect(
      screen
        .getAllByTestId('leverage-day')
        .some((line) => line.textContent === 'Agent time per hour of yoursunknown'),
    ).toBe(true),
  );
});

it('human + agent: a day the report marks at or below even is red, and says so in words', async () => {
  await mountPaired(
    paired((r) => {
      const day = r.leverage.by_day[2]!;
      day.value = 0.8;
      day.at_or_below_even = true;
      day.human_ms = 3_600_000;
    }),
  );
  const cells = screen.getAllByTestId('leverage-row-total');
  expect(cells[2]!.textContent).toBe('0.8×');
  expect(cells[2]!.dataset.low).toBe('');
  expect(cells[6]!.dataset.low).toBeUndefined();
  const rows = screen.getAllByTestId('human-day');
  expect(rows[2]!.getAttribute('aria-label')).toMatch(
    /; agent time per hour of yours: 0\.8×, at or below even; Enter opens the day’s sessions$/,
  );
  expect(rows[6]!.getAttribute('aria-label')).toMatch(
    /; agent time per hour of yours: 2\.6×; Enter opens the day’s sessions$/,
  );
  fireEvent.focus(rows[2]!);
  await waitFor(() =>
    expect(
      screen
        .getAllByTestId('leverage-day')
        .some(
          (line) =>
            line.dataset.low === '' &&
            line.textContent === 'Agent time per hour of yours0.8×at or below even',
        ),
    ).toBe(true),
  );
});

it('human + agent: unknown human time still says so', async () => {
  await mountPaired(
    paired((r) => {
      r.human_hours.current.active_ms = null;
      r.tiles.leverage = { ...r.tiles.leverage, value: null };
    }),
  );
  expect(screen.getByTestId('effort-total').textContent).toBe('human time unknown');
  expect(screen.getByTestId('effort-leverage').textContent).toBe('—');
  expect(screen.getByTestId('effort-headline').textContent).toContain(
    'Could not tell which messages a person sent.',
  );
});

it('the plain human h view draws no agents line, agent totals, person icon or pointer line', () => {
  mount(paired());
  expect(screen.getByTestId('human-timeline').dataset.variant).toBeUndefined();
  expect(screen.queryAllByTestId('agent-line')).toHaveLength(0);
  expect(screen.queryByTestId('human-total-head')).toBeNull();
  expect(screen.queryByTestId('agent-total-head')).toBeNull();
  expect(screen.queryAllByTestId('agent-row-total')).toHaveLength(0);
  expect(screen.queryAllByTestId('leverage-row-total')).toHaveLength(0);
  expect(screen.queryByTestId('leverage-total-head')).toBeNull();
  expect(screen.queryAllByTestId('agent-day-total')).toHaveLength(0);
  expect(
    screen.getAllByTestId('human-day').at(-1)!.querySelectorAll('.xt-human-total'),
  ).toHaveLength(1);
  const track = screen
    .getAllByTestId('human-day')
    .at(-1)!
    .querySelector<HTMLElement>('.xt-human-track')!;
  track.getBoundingClientRect = () => ({ left: 0, width: 960, top: 0 }) as DOMRect;
  fireEvent.pointerMove(track, { clientX: 321 });
  expect(screen.queryByTestId('agent-hairline')).toBeNull();
});

const F1_SESSION = '00000000-0000-4000-8000-000000000001';
/** A day's open sessions box. */
const sessionsBox = (day = 'Sep 7') => screen.findByRole('dialog', { name: `Sessions on ${day}` });
/** The pointer onto a day, then onto its date. */
function hoverDate(row: HTMLElement) {
  const date = within(row).getByRole('button', { name: /^Sessions on / });
  fireEvent.mouseEnter(row);
  fireEvent.mouseEnter(date);
  return date;
}

it('human + agent: hovering a date opens its sessions box instead of the day’s card, and a session opens from it', async () => {
  const source = new FixtureDataSource(exported());
  const read = vi.spyOn(source, 'daySessions');
  await mountPaired(paired(), source);
  const last = screen.getAllByTestId('human-day').at(-1)!;
  // The rest of the day shows its card, without sessions; nothing is read.
  fireEvent.mouseEnter(last);
  await screen.findByText('Busiest 15 min');
  expect(screen.queryByTestId('day-sessions-head')).toBeNull();
  expect(read).not.toHaveBeenCalled();
  // The date: a named popup trigger, never a tab stop.
  const date = within(last).getByRole('button', { name: 'Sessions on Sep 7' });
  expect(date.textContent).toBe('Sep 7');
  expect(date.tabIndex).toBe(-1);
  expect(date.getAttribute('aria-haspopup')).toBe('dialog');
  fireEvent.mouseEnter(date);
  const box = await sessionsBox();
  const link = await within(box).findByRole('link', {
    name: `Open session ${F1_SESSION}, 23 minutes this day`,
  });
  // One box at a time: the day's card has gone.
  await waitFor(() => expect(screen.queryByText('Busiest 15 min')).toBeNull());
  expect(document.querySelectorAll('.xt-effort-tip')).toHaveLength(1);
  expect(read).toHaveBeenCalledExactlyOnceWith(7, paired().window.end_ms, 'UTC', '2026-09-07');
  expect(within(box).getByTestId('day-sessions-head').textContent).toBe('Sessions on Sep 7');
  // Named as the lanes name it, with its time that day as all agent time is written.
  expect(link.textContent).toBe('Session 000000000 h 23 m');
  expect(screen.queryByTestId('day-sessions-more')).toBeNull();
  // The link carries the list that finds the session, over the report's range.
  expect(link.getAttribute('href')).toBe(
    `/sessions/${F1_SESSION}?q=${F1_SESSION}&host=claude&range=7d`,
  );
  fireEvent.click(link);
  expect((await screen.findByTestId('session-path')).textContent).toBe(
    `/sessions/${F1_SESSION}?q=${F1_SESSION}&host=claude&range=7d`,
  );
});

it('human + agent: a click on a date neither opens nor closes its sessions box', async () => {
  await mountPaired(paired());
  const last = screen.getAllByTestId('human-day').at(-1)!;
  const date = within(last).getByRole('button', { name: 'Sessions on Sep 7' });
  fireEvent.click(date);
  await act(async () => {});
  expect(screen.queryByRole('dialog')).toBeNull();
  // Opened by hover, a click leaves it as it was.
  hoverDate(last);
  await sessionsBox();
  fireEvent.click(date);
  await act(async () => {});
  expect(screen.getByRole('dialog', { name: 'Sessions on Sep 7' })).toBeTruthy();
});

it('human + agent: Enter on the focused day opens its sessions box onto its first session, Escape returns to the day', async () => {
  const source = new FixtureDataSource(exported());
  const read = vi.spyOn(source, 'daySessions');
  await mountPaired(paired(), source);
  const days = screen.getAllByTestId('human-day');
  const last = days.at(-1)!;
  // Focus shows the day's card, as before; the sessions box stays closed.
  last.focus();
  await screen.findByText('Busiest 15 min');
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(last.getAttribute('aria-label')).toMatch(/; Enter opens the day’s sessions$/);
  // Enter opens the box; focus lands on the first session once it is read.
  fireEvent.keyDown(last, { key: 'Enter' });
  const box = await sessionsBox();
  const link = await within(box).findByRole('link');
  await waitFor(() => expect(document.activeElement).toBe(link));
  await waitFor(() => expect(screen.queryByText('Busiest 15 min')).toBeNull());
  // Arrow keys inside the box are the box's own, not the days'.
  fireEvent.keyDown(link, { key: 'ArrowUp' });
  expect(document.activeElement).toBe(link);
  fireEvent.keyDown(link, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(document.activeElement).toBe(last);
  // The focused day's card is back once the box has gone.
  await screen.findByText('Busiest 15 min');
  // Space opens it again, straight onto the session; the same day of the
  // same report is not read again.
  fireEvent.keyDown(last, { key: ' ' });
  const again = await within(await sessionsBox()).findByRole('link');
  await waitFor(() => expect(document.activeElement).toBe(again));
  expect(read).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(again, { key: 'Escape' });
  await waitFor(() => expect(document.activeElement).toBe(last));
  // Arrow keys still move between days, each showing its card.
  fireEvent.keyDown(last, { key: 'ArrowUp' });
  expect(document.activeElement).toBe(days.at(-2));
  await waitFor(() =>
    expect(document.querySelector('.xt-effort-tip-head')?.textContent).toMatch(/^Sep 6/),
  );
  // A day without agent time says so, and focus waits on its box.
  fireEvent.keyDown(days.at(-2)!, { key: 'Enter' });
  const before = await sessionsBox('Sep 6');
  await waitFor(() =>
    expect(within(before).getByTestId('day-sessions-state').textContent).toBe('No sessions'),
  );
  expect(document.activeElement).toBe(before);
});

it('human + agent: a sessions box says when its sessions are being read or could not be read, and counts the rest', async () => {
  const source = new FixtureDataSource(exported());
  let answer: (value: Awaited<ReturnType<FixtureDataSource['daySessions']>>) => void = () => {};
  const day = paired().leverage.by_day.at(-1)!;
  const read = vi
    .spyOn(source, 'daySessions')
    .mockImplementationOnce(() => new Promise((resolve) => (answer = resolve)))
    .mockRejectedValueOnce(new Error('refused'));
  await mountPaired(paired(), source);
  const [first, last] = [
    screen.getAllByTestId('human-day').at(0)!,
    screen.getAllByTestId('human-day').at(-1)!,
  ];
  last.focus();
  fireEvent.keyDown(last, { key: 'Enter' });
  const open = await sessionsBox();
  expect((await within(open).findByTestId('day-sessions-state')).textContent).toBe(
    'Reading sessions…',
  );
  // Eight named, the most time first, and the rest counted with their time.
  answer({
    date: day.date,
    start_ms: day.start_ms,
    end_ms: day.end_ms,
    agent_ms: 10 * H,
    sessions: Array.from({ length: 8 }, (_, n) => ({
      session_id: `s-${n}`,
      host: n === 0 ? 'codex' : 'claude',
      title: n === 0 ? 'Fix the sidebar' : null,
      automated_review: n === 1,
      agent_ms: H,
      last_activity_at_ms: day.start_ms + 10 * H,
    })),
    more_sessions: 3,
    more_ms: 2 * H,
  });
  const links = await within(open).findAllByTestId('day-session');
  expect(links.map((link) => link.querySelector('span')!.textContent)).toEqual([
    'Fix the sidebar',
    'Automated review',
    ...Array.from({ length: 6 }, (_, n) => `Session s-${n + 2}`),
  ]);
  expect(links[0]!.getAttribute('href')).toBe('/sessions/s-0?q=s-0&host=codex&range=7d');
  expect(screen.getByTestId('day-sessions-more').textContent).toBe('and 3 more sessions2 h 0 m');
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  first.focus();
  fireEvent.keyDown(first, { key: 'Enter' });
  const failed = await sessionsBox('Sep 1');
  await waitFor(() =>
    expect(within(failed).getByTestId('day-sessions-state').textContent).toBe(
      'Sessions could not be read',
    ),
  );
  expect(read).toHaveBeenCalledTimes(2);
});

it('human + agent: a sessions box drops saved titles read before stored content was deleted', async () => {
  const source = new FixtureDataSource(exported());
  const day = paired().leverage.by_day.at(-1)!;
  const answer = (title: string | null) => ({
    date: day.date,
    start_ms: day.start_ms,
    end_ms: day.end_ms,
    agent_ms: H,
    sessions: [
      {
        session_id: 's-0',
        host: 'claude',
        title,
        automated_review: false,
        agent_ms: H,
        last_activity_at_ms: day.start_ms + H,
      },
    ],
    more_sessions: 0,
    more_ms: 0,
  });
  let after: (value: ReturnType<typeof answer>) => void = () => {};
  const read = vi
    .spyOn(source, 'daySessions')
    .mockResolvedValueOnce(answer('Saved before the delete'))
    .mockImplementationOnce(() => new Promise((resolve) => (after = resolve)));
  await mountPaired(paired(), source);
  hoverDate(screen.getAllByTestId('human-day').at(-1)!);
  const open = await sessionsBox();
  await within(open).findByText('Saved before the delete');
  // The open box reads again and never shows the old title meanwhile.
  act(() => source.emit(events.contentPurged));
  await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(within(open).queryByText('Saved before the delete')).toBeNull();
  expect(within(open).getByTestId('day-sessions-state').textContent).toBe('Reading sessions…');
  await act(async () => after(answer(null)));
  expect((await within(open).findByTestId('day-session')).textContent).toBe('Session s-01 h 0 m');
});

it('human + agent: a session’s host title is read again when the session has later activity, on any day', async () => {
  const source = new FixtureDataSource(exported());
  const day = paired().leverage.by_day.at(-1)!;
  // The same hour that day, and later activity the second time, after the day.
  const answer = (lastActivity: number) => ({
    date: day.date,
    start_ms: day.start_ms,
    end_ms: day.end_ms,
    agent_ms: H,
    sessions: [
      {
        session_id: 's-0',
        host: 'claude',
        title: null,
        automated_review: false,
        agent_ms: H,
        last_activity_at_ms: lastActivity,
      },
    ],
    more_sessions: 0,
    more_ms: 0,
  });
  vi.spyOn(source, 'daySessions')
    .mockResolvedValueOnce(answer(day.start_ms + H))
    .mockResolvedValue(answer(day.end_ms + 2 * H));
  const titles = vi
    .fn<SessionTitleControls['read']>()
    .mockResolvedValueOnce({ titles: [{ id: 's-0', title: 'Old name' }] })
    .mockResolvedValue({ titles: [{ id: 's-0', title: 'New name' }] });
  Object.assign(source, { titles: { read: titles, cancel: vi.fn(async () => {}) } });
  await mountPaired(paired(), source);
  hoverDate(screen.getAllByTestId('human-day').at(-1)!);
  const open = await sessionsBox();
  await within(open).findByText('Old name');
  // Committed data re-reads the box: the same time that day, later activity.
  act(() => source.emit(events.importReceived));
  await within(open).findByText('New name', undefined, { timeout: 3000 });
  expect(titles).toHaveBeenCalledTimes(2);
  expect(titles.mock.calls.map((call) => call[0])).toEqual([['s-0'], ['s-0']]);
});
