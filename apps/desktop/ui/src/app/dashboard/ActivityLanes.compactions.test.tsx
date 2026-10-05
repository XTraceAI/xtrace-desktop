import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { CompactionCount } from '../../data/generated/CompactionCount';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { SessionCompactions } from '../../data/generated/SessionCompactions';
import { events } from '../../data/ipc-names';
import type { TimeRange } from '../../kit/TopBar';
import { ActivityLanes } from './ActivityLanes';
import { recordedTime } from './present';

let observed: IntersectionObserverCallback | undefined;
const targets = new Set<Element>();
class Observer {
  private readonly watching = new Set<Element>();
  constructor(callback: IntersectionObserverCallback) {
    observed = callback;
  }
  observe(target: Element) {
    this.watching.add(target);
    targets.add(target);
  }
  unobserve(target: Element) {
    this.watching.delete(target);
    targets.delete(target);
  }
  disconnect() {
    for (const target of this.watching) targets.delete(target);
    this.watching.clear();
  }
}
const show = (ids?: readonly string[]) =>
  act(() =>
    observed!(
      [...targets].map(
        (target) =>
          ({
            target,
            isIntersecting: !ids || ids.includes((target as HTMLElement).dataset.visibleId!),
          }) as IntersectionObserverEntry,
      ),
      {} as IntersectionObserver,
    ),
  );
beforeEach(() => vi.stubGlobal('IntersectionObserver', Observer));
afterEach(() => {
  cleanup();
  targets.clear();
  vi.unstubAllGlobals();
});

function setup(outcomes: Record<string, CompactionCount>) {
  const source = new FixtureDataSource(structuredClone(fixture as FixtureExport));
  const report = structuredClone((fixture as FixtureExport).dashboards[0]);
  const ids = Object.keys(outcomes);
  const { lane_start_ms: start, lane_end_ms: end } = report;
  // Two spans per row, leaving an idle gap in the middle of the window.
  report.lanes = ids.flatMap((session_id) => [
    { session_id, host: 'claude', start_ms: start + 1000, end_ms: start + 2000 },
    { session_id, host: 'claude', start_ms: end - 2000, end_ms: end - 1000 },
  ]);
  report.lane_sessions = ids.map((session_id) => ({
    ...report.lane_sessions[0],
    session_id,
    host: 'claude',
    title: session_id,
    parent: null,
  }));
  report.lanes_total = report.lanes.length;
  const read = vi.fn(async (requested: readonly string[]): Promise<SessionCompactions> => ({
    counts: requested.map((id) => ({ id, outcome: outcomes[id] })),
  }));
  const cancel = vi.fn(async () => {});
  Object.assign(source, { compactions: { read, cancel } });
  const page = (range: TimeRange, data = source) => (
    <MemoryRouter>
      <DataProvider source={data}>
        <ActivityLanes report={report} range={range} />
      </DataProvider>
    </MemoryRouter>
  );
  const view = render(page('7d'));
  const row = (id: string) =>
    screen
      .getByRole('link', { name: `Open session ${id}, ${id}` })
      .closest('[role="row"]') as HTMLElement;
  return { report, start, end, row, source, read, cancel, view, page };
}

it('draws a focusable tick at each recorded compaction inside the lane window', async () => {
  const fixtureReport = (fixture as FixtureExport).dashboards[0];
  const start = fixtureReport.lane_start_ms;
  const end = fixtureReport.lane_end_ms;
  const middle = start + (end - start) / 2;
  const { report, row } = setup({
    'timed-chat': {
      state: 'count',
      count: 4,
      events: [
        { at_ms: start - 60_000, trigger: 'auto' },
        { at_ms: start + 1500, trigger: 'auto' },
        { at_ms: middle, trigger: 'manual' },
        { at_ms: end - 1500, trigger: 'unknown' },
      ],
    },
    'untimed-chat': { state: 'count', count: 2, events: [] },
    'unknown-chat': { state: 'unknown', reason: 'ownership' },
  });
  await screen.findByRole('link', { name: 'Open session timed-chat, timed-chat' });
  show();
  const timedRow = row('timed-chat');
  const timed = within(timedRow);
  const when = (ms: number) => recordedTime(ms, report.window);
  const auto = await timed.findByLabelText(`Compaction recorded ${when(start + 1500)}. Automatic.`);
  const manual = timed.getByRole('img', {
    name: `Compaction recorded ${when(middle)}. Manual.`,
  });
  const unknown = timed.getByLabelText(
    `Compaction recorded ${when(end - 1500)}. Automatic or manual: not recorded.`,
  );
  // The one before the window is not drawn; the count still says four.
  expect(timedRow.querySelectorAll('.xt-compaction-tick')).toHaveLength(3);
  expect(timed.getByLabelText('Recorded compactions: 4')).toBeTruthy();
  // Placed on the same axis as the spans; the manual one sits in the idle gap.
  const span = end - start;
  expect(parseFloat(auto.style.left)).toBeCloseTo((1500 / span) * 100, 6);
  expect(parseFloat(manual.style.left)).toBeCloseTo(50, 6);
  expect(parseFloat(unknown.style.left)).toBeCloseTo(((span - 1500) / span) * 100, 6);
  expect(manual.dataset.trigger).toBe('manual');
  for (const tick of [auto, manual, unknown]) {
    expect(tick.tabIndex).toBe(0);
    expect(tick.getAttribute('role')).toBe('img');
    expect(tick.closest('[aria-hidden="true"]')).toBeNull();
  }
  // Spans stay distinct marks: named graphics of their own, never ticks.
  for (const bar of timedRow.querySelectorAll('.xt-lane-span')) {
    expect(bar.getAttribute('role')).toBe('img');
    expect(bar.getAttribute('aria-label')).toMatch(/^Active span /);
  }

  fireEvent.focus(manual);
  const tooltip = await screen.findByRole('tooltip');
  expect(tooltip.textContent).toContain(`Compaction · ${when(middle)}`);
  expect(tooltip.textContent).toContain('Manual');

  // A count with no recorded times, and an unknown count, draw no tick.
  await waitFor(() =>
    expect(within(row('untimed-chat')).getByLabelText('Recorded compactions: 2')).toBeTruthy(),
  );
  expect(row('untimed-chat').querySelector('.xt-compaction-tick')).toBeNull();
  expect(row('unknown-chat').querySelector('.xt-compaction-tick')).toBeNull();
});

it('keeps resolved counts and ticks across scrolling, then applies genuine refreshes', async () => {
  const at = (fixture as FixtureExport).dashboards[0].lane_start_ms + 1500;
  const outcomes: Record<string, CompactionCount> = {
    first: { state: 'count', count: 100, events: [{ at_ms: at, trigger: 'auto' }] },
    zero: { state: 'count', count: 0, events: [] },
    unknown: { state: 'unknown', reason: 'ownership' },
    later: { state: 'count', count: 2, events: [] },
  };
  const { row, source, read } = setup(outcomes);
  const pending: Array<{ ids: readonly string[]; resolve: (answer: SessionCompactions) => void }> =
    [];
  read.mockImplementation((ids) => new Promise((resolve) => pending.push({ ids, resolve })));
  const answer = async (index: number) => {
    await act(async () => {
      const { ids, resolve } = pending[index];
      resolve({ counts: ids.map((id) => ({ id, outcome: outcomes[id] })) });
    });
  };
  await screen.findByRole('link', { name: 'Open session first, first' });
  const scroll = screen.getByRole('region', { name: 'Session lanes scroll area' });
  scroll.scrollTop = 23;
  show(['first', 'zero', 'unknown']);
  await waitFor(() => expect(pending).toHaveLength(1));
  await answer(0);
  await waitFor(() =>
    expect(within(row('first')).getByLabelText('Recorded compactions: 100')).toBeTruthy(),
  );
  const tick = row('first').querySelector('.xt-compaction-tick');
  const stable = () => {
    expect(within(row('first')).getByLabelText('Recorded compactions: 100')).toBeTruthy();
    expect(row('first').querySelector('.xt-compaction-tick')).toBe(tick);
    expect(within(row('zero')).getByLabelText('Recorded compactions: 0')).toBeTruthy();
    expect(within(row('unknown')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
    expect(screen.getByRole('region', { name: 'Session lanes scroll area' })).toBe(scroll);
    expect(scroll.scrollTop).toBe(23);
  };
  // Adding one visible row must not erase already resolved neighbours.
  show(['first', 'zero', 'unknown', 'later']);
  await waitFor(() => expect(pending).toHaveLength(2));
  stable();
  expect(within(row('later')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
  await answer(1);
  // Scrolling away and back keeps the answer while a fresh read is pending.
  show(['later']);
  await waitFor(() => expect(pending).toHaveLength(3));
  stable();
  await answer(2);
  show(['first', 'zero', 'unknown']);
  await waitFor(() => expect(pending).toHaveLength(4));
  stable();
  await answer(3);
  // A committed event still rechecks the source and applies a changed answer.
  outcomes.first = { state: 'count', count: 101, events: [] };
  act(() => source.emit(events.turnCompleted));
  await waitFor(() => expect(pending).toHaveLength(5));
  stable();
  await answer(4);
  await waitFor(() =>
    expect(within(row('first')).getByLabelText('Recorded compactions: 101')).toBeTruthy(),
  );
  expect(row('first').querySelector('.xt-compaction-tick')).toBeNull();
  expect(read.mock.calls.every(([ids]) => ids.length <= 50)).toBe(true);
});

it('replaces retained counts with explicit unknown or failure instead of restoring stale ticks', async () => {
  const at = (fixture as FixtureExport).dashboards[0].lane_start_ms + 1500;
  const { row, read, source } = setup({
    first: { state: 'count', count: 100, events: [{ at_ms: at, trigger: 'auto' }] },
    later: { state: 'count', count: 0, events: [] },
  });
  await screen.findByRole('link', { name: 'Open session first, first' });
  show(['first']);
  await within(row('first')).findByLabelText('Recorded compactions: 100');
  read.mockResolvedValueOnce({
    counts: [{ id: 'first', outcome: { state: 'unknown', reason: 'missing' } }],
  });
  act(() => source.emit(events.turnCompleted));
  await within(row('first')).findByLabelText('Recorded compactions: unknown');
  expect(row('first').querySelector('.xt-compaction-tick')).toBeNull();
  show(['later']);
  await within(row('later')).findByLabelText('Recorded compactions: 0');
  show(['first']);
  await within(row('first')).findByLabelText('Recorded compactions: 100');
  read
    .mockRejectedValueOnce(new Error('source unreadable'))
    .mockRejectedValueOnce(new Error('still unreadable'));
  act(() => source.emit(events.turnCompleted));
  await within(row('first')).findByLabelText('Recorded compactions: unknown');
  show(['later']);
  await within(row('later')).findByLabelText('Recorded compactions: 0');
  expect(within(row('first')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
  expect(row('first').querySelector('.xt-compaction-tick')).toBeNull();
});

it('keeps four ticks through a temporary source failure, then applies the fifth with count 101', async () => {
  const start = (fixture as FixtureExport).dashboards[0].lane_start_ms;
  const ticks = Array.from({ length: 5 }, (_, index) => ({
    at_ms: start + (index + 1) * 1000,
    trigger: 'auto' as const,
  }));
  const { row, read, source } = setup({
    first: { state: 'count', count: 100, events: ticks.slice(0, 4) },
  });
  await screen.findByRole('link', { name: 'Open session first, first' });
  show(['first']);
  await within(row('first')).findByLabelText('Recorded compactions: 100');
  const before = read.mock.calls.length;
  let finish!: (answer: SessionCompactions) => void;
  read.mockRejectedValueOnce(new Error('read refused')).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  act(() => source.emit(events.turnCompleted));
  await waitFor(() => expect(read).toHaveBeenCalledTimes(before + 2));
  expect(within(row('first')).getByLabelText('Recorded compactions: 100')).toBeTruthy();
  expect(row('first').querySelectorAll('.xt-compaction-tick')).toHaveLength(4);
  await act(async () =>
    finish({ counts: [{ id: 'first', outcome: { state: 'count', count: 101, events: ticks } }] }),
  );
  await within(row('first')).findByLabelText('Recorded compactions: 101');
  expect(row('first').querySelectorAll('.xt-compaction-tick')).toHaveLength(5);
});

it('clears retained counts on range/source changes and drops cancelled late answers', async () => {
  const { row, read, cancel, view, page, source } = setup({
    first: { state: 'count', count: 100, events: [] },
  });
  await screen.findByRole('link', { name: 'Open session first, first' });
  show(['first']);
  await within(row('first')).findByLabelText('Recorded compactions: 100');
  const pending: Array<(answer: SessionCompactions) => void> = [];
  read.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
  act(() => source.emit(events.turnCompleted));
  await waitFor(() => expect(pending).toHaveLength(1));
  view.rerender(page('14d'));
  await waitFor(() => expect(pending).toHaveLength(2));
  expect(cancel).toHaveBeenCalled();
  expect(within(row('first')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
  await act(async () =>
    pending[0]({ counts: [{ id: 'first', outcome: { state: 'count', count: 999, events: [] } }] }),
  );
  expect(within(row('first')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
  await act(async () =>
    pending[1]({ counts: [{ id: 'first', outcome: { state: 'count', count: 2, events: [] } }] }),
  );
  await within(row('first')).findByLabelText('Recorded compactions: 2');
  act(() => source.emit(events.turnCompleted));
  await waitFor(() => expect(pending).toHaveLength(3));
  const replacement = Object.assign(
    new FixtureDataSource(structuredClone(fixture as FixtureExport)),
    {
      compactions: {
        read: vi.fn(() => new Promise<SessionCompactions>((resolve) => pending.push(resolve))),
        cancel: vi.fn(async () => {}),
      },
    },
  );
  view.rerender(page('14d', replacement));
  await screen.findByRole('link', { name: 'Open session first, first' });
  show(['first']);
  await waitFor(() => expect(pending).toHaveLength(4));
  expect(within(row('first')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
  await act(async () =>
    pending[2]({ counts: [{ id: 'first', outcome: { state: 'count', count: 888, events: [] } }] }),
  );
  expect(within(row('first')).getByLabelText('Recorded compactions: unknown')).toBeTruthy();
  await act(async () =>
    pending[3]({ counts: [{ id: 'first', outcome: { state: 'count', count: 3, events: [] } }] }),
  );
  await within(row('first')).findByLabelText('Recorded compactions: 3');
});
