import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { HumanTimeline } from './HumanTimeline';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(cleanup);

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
      <HumanTimeline report={r} />
    </ThemeProvider>,
  );

it('draws one 24-hour row per day with each stretch, ticks for single messages and the day total', () => {
  mount(report());
  expect(screen.getByTestId('effort-total').textContent).toBe('1.5 your h');
  expect(screen.getByTestId('effort-headline').textContent).toContain('last 7 days');
  expect(screen.getByTestId('effort-headline').textContent).toContain(
    'gaps over 45 min count as breaks',
  );
  const rows = screen.getAllByTestId('human-day');
  expect(rows).toHaveLength(7);
  expect(rows.map((row) => row.dataset.date)).toEqual(
    report().human_hours.current.by_day.map((day) => day.date),
  );
  const [sixth, last] = rows.slice(-2) as [HTMLElement, HTMLElement];
  // A stretch cut at midnight runs to 24:00, and the next day's piece starts at 00:00.
  expect(sixth.getAttribute('aria-label')).toBe('Sep 6: 1 h; 23:00–24:00');
  expect(last.getAttribute('aria-label')).toBe('Sep 7: 0.5 h; 00:00–00:30, one message at 02:00');
  const bars = within(last).getAllByRole('img');
  expect(bars.map((bar) => bar.getAttribute('aria-label'))).toEqual([
    'Sep 7, 00:00–00:30',
    'Sep 7, one message at 02:00',
  ]);
  expect(bars[0]!.style.left).toBe('0%');
  expect(bars[1]!.dataset.tick).toBe('true');
  expect(bars[1]!.style.width).toBe('');
  expect(rows[0]!.textContent).toBe('Sep 1–');
  expect(last.textContent).toBe('Sep 70.5 h');
  // The hour axis is drawn once, above the rows.
  expect(screen.getByTestId('human-timeline').querySelector('.xt-human-axis')!.textContent).toBe(
    '0006121824',
  );
});

it('says your hours are unknown instead of drawing a smaller number', () => {
  mount(
    report((r) => {
      r.human_hours.current.active_ms = null;
      for (const day of r.human_hours.current.by_day) {
        day.active_ms = null;
        day.stretches = [];
      }
    }),
  );
  expect(screen.getByTestId('effort-total').textContent).toBe('your h unknown');
  expect(screen.getByTestId('effort-headline').textContent).toContain(
    'Could not tell which messages are yours.',
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
  expect(images[0]!.getAttribute('aria-label')).toBe('Nov 1, 01:50 PDT → 01:10 PST — 20 min');
  const pieces = images[0]!.querySelectorAll<HTMLElement>('.xt-human-bar');
  expect(pieces).toHaveLength(2);
  expect(pieces[0]!.dataset.lane).toBe('0');
  expect(pieces[1]!.dataset.lane).toBe('1');
  for (const piece of pieces)
    expect(parseFloat(piece.style.width)).toBeCloseTo((10 / (24 * 60)) * 100);
  expect(screen.getByTestId('effort-total').textContent).toBe('0.3 your h');
  fireEvent.focus(screen.getByTestId('human-day'));
  await screen.findByText('01:50 PDT → 01:10 PST — 20 min');
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
        <HumanTimeline report={refreshed} />
      </ThemeProvider>,
    );
    expect(body.scrollTop).toBe(200);
    // Another range opens on today again.
    view.rerender(
      <ThemeProvider>
        <HumanTimeline report={range(7)} />
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
    'Sep 7: 0.5 h; 00:00–00:30, one message at 02:00',
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
