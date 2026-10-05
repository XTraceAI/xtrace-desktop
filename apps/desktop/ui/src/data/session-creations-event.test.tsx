import { act, cleanup, render, screen } from '@testing-library/react';
import { useQuery } from '@tanstack/react-query';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from './DataProvider';
import { FixtureDataSource } from './FixtureDataSource';
import { events } from './ipc-names';
import { queryKeys } from './query-client';
import type { FixtureExport } from './generated/FixtureExport';
import type { SessionParentLink } from './generated/SessionParentLink';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const CHILD = 'codex-019a0000-0000-7000-8000-0000000000bb';
const PARENT: SessionParentLink = {
  session_id: 'codex-019a0000-0000-7000-8000-0000000000aa',
  host: 'codex',
  title: null,
  evidence: 'native_spawn',
};

/**
 * A background sub-session pass commits between reconciliations and the app
 * publishes `sessions://creations-changed` once. Already-mounted Sessions,
 * exact-row and Dashboard views read again and show (then withhold) the
 * parent without navigation, focus or an unrelated event, while the index
 * status, the database counts and every other report are not read again. With
 * no event, nothing is read.
 */
it('re-reads mounted Sessions, session row and Dashboard views on a committed creation change only', async () => {
  vi.useFakeTimers();
  const source = new FixtureDataSource(exported);
  // What the store holds for the child's relation, as each read would see it.
  let stored: SessionParentLink | null = null;
  const sessionsList = vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: exported.sessions[0].window,
    rows: [{ ...exported.sessions[0].rows[0], id: CHILD, parent: stored }],
    next: null,
  }));
  const sessionRow = vi.spyOn(source, 'sessionRow').mockImplementation(async () => ({
    ...exported.sessions[0].rows[0],
    id: CHILD,
    parent: stored,
  }));
  const dashboard = vi.spyOn(source, 'dashboard').mockImplementation(async (days) => {
    const report = exported.dashboards.find((each) => each.window.days === days)!;
    return {
      ...report,
      lane_sessions: [{ ...report.lane_sessions[0], session_id: CHILD, parent: stored }],
    };
  });
  const humanTotals = vi.spyOn(source, 'today');
  const unrelated = [
    vi.spyOn(source, 'environment'),
    vi.spyOn(source, 'tokensByHost'),
    vi.spyOn(source, 'nativeIndexStatus'),
    vi.spyOn(source, 'dbCounts'),
  ];
  const filter = { search: '', hosts: null, withPrs: false };
  function Views() {
    const list = useQuery({
      queryKey: [...queryKeys.sessions(7), '', 'all', false],
      queryFn: () => source.sessionsList(filter, null, 7),
    });
    const row = useQuery({
      queryKey: queryKeys.session(7, CHILD),
      queryFn: () => source.sessionRow(CHILD, 7),
    });
    const week = useQuery({
      queryKey: queryKeys.dashboard(7),
      queryFn: () => source.dashboard(7),
    });
    const month = useQuery({
      queryKey: queryKeys.dashboard(30),
      queryFn: () => source.dashboard(30),
    });
    useQuery({ queryKey: queryKeys.environment(7), queryFn: () => source.environment(7) });
    useQuery({ queryKey: queryKeys.tokensByHost(7), queryFn: () => source.tokensByHost(7) });
    useQuery({ queryKey: queryKeys.nativeIndex, queryFn: () => source.nativeIndexStatus() });
    useQuery({ queryKey: queryKeys.dbCounts, queryFn: () => source.dbCounts() });
    useQuery({ queryKey: queryKeys.today, queryFn: () => source.today() });
    const shown = (parent: SessionParentLink | null | undefined) =>
      parent ? `Sub-session of ${parent.session_id}` : 'no parent';
    return (
      <>
        <output aria-label="Sessions list">{shown(list.data?.rows[0]?.parent)}</output>
        <output aria-label="Session row">{shown(row.data?.parent)}</output>
        <output aria-label="Dashboard 7d">{shown(week.data?.lane_sessions[0]?.parent)}</output>
        <output aria-label="Dashboard 30d">{shown(month.data?.lane_sessions[0]?.parent)}</output>
      </>
    );
  }
  render(
    <DataProvider source={source}>
      <Views />
    </DataProvider>,
  );
  // One settle opens the runtime once its listeners register; the next lands the first reads.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  const views = ['Sessions list', 'Session row', 'Dashboard 7d', 'Dashboard 30d'];
  const expectShown = (text: string) => {
    for (const name of views) expect(screen.getByRole('status', { name }).textContent).toBe(text);
  };
  expectShown('no parent');
  const all = [sessionsList, sessionRow, dashboard, humanTotals, ...unrelated];
  const clear = () => all.forEach((spy) => spy.mockClear());

  // The relation commits. Focus, time and an unrelated event do not show it.
  stored = PARENT;
  clear();
  await act(async () => {
    window.dispatchEvent(new Event('focus'));
    document.dispatchEvent(new Event('visibilitychange'));
    source.emit(events.fireReceived);
    await vi.advanceTimersByTimeAsync(5_000);
  });
  expectShown('no parent');
  for (const spy of [sessionsList, sessionRow, dashboard]) expect(spy).not.toHaveBeenCalled();

  // The committed change's one event re-reads exactly the views that carry it.
  await act(async () => {
    source.emit(events.sessionCreationsChanged);
    await vi.advanceTimersByTimeAsync(501);
  });
  expectShown(`Sub-session of ${PARENT.session_id}`);
  expect(humanTotals).toHaveBeenCalledOnce();
  expect(sessionsList).toHaveBeenCalledOnce();
  expect(sessionRow).toHaveBeenCalledExactlyOnceWith(CHILD, 7);
  expect(dashboard.mock.calls.map(([days]) => days).sort((a, b) => a - b)).toEqual([7, 30]);
  for (const spy of unrelated) expect(spy).not.toHaveBeenCalled();

  // A first conflict withholds the parent the same way.
  stored = null;
  clear();
  await act(async () => {
    source.emit(events.sessionCreationsChanged);
    await vi.advanceTimersByTimeAsync(501);
  });
  expectShown('no parent');
  expect(sessionsList).toHaveBeenCalledOnce();
  for (const spy of unrelated) expect(spy).not.toHaveBeenCalled();

  // A pass that changed nothing sends nothing, and nothing is read.
  clear();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(10_000);
  });
  for (const spy of all) expect(spy).not.toHaveBeenCalled();
  expectShown('no parent');
});
