import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import type { ReactNode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionCompactions } from '../data/generated/SessionCompactions';
import { events } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { CompactionBadge, useSessionCompactions } from './session-compactions';
import { AppRoutes } from './AppRoutes';

afterEach(cleanup);
const exported = fixture as FixtureExport;
function source() {
  return Object.assign(new FixtureDataSource(structuredClone(exported)), {
    compactions: {
      read: vi.fn(async (ids: readonly string[], readId: string): Promise<SessionCompactions> => {
        void readId;
        return {
          counts: ids.map((id) => ({ id, outcome: { state: 'count', count: 0, events: [] } })),
        };
      }),
      cancel: vi.fn(async (readId: string) => {
        void readId;
      }),
    },
  });
}
function wrapper(data: ReturnType<typeof source>) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <ThemeProvider>
        <DataProvider source={data}>{children}</DataProvider>
      </ThemeProvider>
    );
  };
}
it('keeps zero, unknown and values above five distinct; focus explains the caution', async () => {
  render(
    <ThemeProvider>
      <CompactionBadge outcome={{ state: 'count', count: 0, events: [] }} />
      <CompactionBadge outcome={{ state: 'unknown', reason: 'ownership' }} />
      <CompactionBadge outcome={{ state: 'count', count: 9, events: [] }} />
    </ThemeProvider>,
  );
  expect(screen.getByLabelText('Recorded compactions: 0').getAttribute('data-step')).toBe('0');
  expect(screen.getByLabelText('Recorded compactions: unknown').textContent).toContain('—');
  const red = screen.getByLabelText('Recorded compactions: 9');
  expect(red.getAttribute('data-step')).toBe('5');
  fireEvent.focus(red);
  expect(await screen.findByRole('tooltip')).toHaveProperty(
    'textContent',
    expect.stringContaining('Repeated summarization is a caution'),
  );
});
it('deduplicates requested IDs, reads at most fifty per command and catches compaction-only events', async () => {
  const data = source();
  let count = 0;
  data.compactions.read.mockImplementation(async (ids) => ({
    counts: ids.map((id) => ({ id, outcome: { state: 'count', count, events: [] } })),
  }));
  const ids = Array.from({ length: 112 }, (_, i) => `session-${i}`);
  const { result } = renderHook(() => useSessionCompactions('list', [...ids, ids[0]]), {
    wrapper: wrapper(data),
  });
  await waitFor(() =>
    expect(result.current(ids[0])).toEqual({ state: 'count', count: 0, events: [] }),
  );
  for (const [named] of data.compactions.read.mock.calls) {
    expect(named.length).toBeLessThanOrEqual(50);
    expect(new Set(named).size).toBe(named.length);
  }
  const before = data.compactions.read.mock.calls.length;
  count = 4;
  act(() => data.emit(events.turnCompleted));
  await waitFor(() =>
    expect(result.current(ids[0])).toEqual({ state: 'count', count: 4, events: [] }),
  );
  expect(data.compactions.read.mock.calls.length).toBeGreaterThan(before);
});
it('cancels obsolete reads and drops their late answers', async () => {
  const data = source();
  const pending: Array<{ ids: readonly string[]; resolve: (answer: SessionCompactions) => void }> =
    [];
  data.compactions.read.mockImplementation(
    (ids) => new Promise((resolve) => pending.push({ ids, resolve })),
  );
  const { result, rerender } = renderHook(({ id }) => useSessionCompactions(id, [id]), {
    initialProps: { id: 'old' },
    wrapper: wrapper(data),
  });
  await waitFor(() => expect(pending.length).toBeGreaterThan(0));
  rerender({ id: 'new' });
  await waitFor(() => expect(data.compactions.cancel).toHaveBeenCalled());
  await act(async () => {
    for (const read of pending.filter((p) => p.ids.includes('old')))
      read.resolve({ counts: [{ id: 'old', outcome: { state: 'count', count: 9, events: [] } }] });
  });
  expect(result.current('old')).toBeUndefined();
  expect(result.current('new')).toBeUndefined();
  await act(async () => {
    for (const read of pending.filter((p) => p.ids.includes('new')))
      read.resolve({ counts: [{ id: 'new', outcome: { state: 'count', count: 2, events: [] } }] });
  });
  await waitFor(() =>
    expect(result.current('new')).toEqual({ state: 'count', count: 2, events: [] }),
  );
});
it.each(['focus', 'visibility', 'both'] as const)(
  'keeps the previous count during %s refresh and then catches a missed compaction',
  async (wake) => {
    const data = source();
    let count = 0;
    let holdRefresh = false;
    let resolveRefresh: (() => void) | undefined;
    data.compactions.read.mockImplementation(async (ids) => {
      const answer: SessionCompactions = {
        counts: ids.map((id) => ({ id, outcome: { state: 'count', count, events: [] } })),
      };
      return holdRefresh
        ? new Promise((resolve) => {
            resolveRefresh = () => resolve(answer);
          })
        : answer;
    });
    const { result, unmount } = renderHook(() => useSessionCompactions('list', ['main']), {
      wrapper: wrapper(data),
    });
    await waitFor(() =>
      expect(result.current('main')).toEqual({ state: 'count', count: 0, events: [] }),
    );
    const before = data.compactions.read.mock.calls.length;
    count = 2;
    holdRefresh = true;
    act(() => {
      if (wake !== 'focus') {
        Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
        document.dispatchEvent(new Event('visibilitychange'));
        Object.defineProperty(document, 'visibilityState', {
          configurable: true,
          value: 'visible',
        });
        document.dispatchEvent(new Event('visibilitychange'));
      }
      if (wake !== 'visibility') window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(resolveRefresh).toBeTypeOf('function'));
    expect(result.current('main')).toEqual({ state: 'count', count: 0, events: [] });
    await act(async () => resolveRefresh!());
    await waitFor(() =>
      expect(result.current('main')).toEqual({ state: 'count', count: 2, events: [] }),
    );
    expect(data.compactions.read.mock.calls.length).toBe(before + 1);
    unmount();
    act(() => {
      window.dispatchEvent(new Event('focus'));
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(data.compactions.read.mock.calls.length).toBe(before + 1);
    Reflect.deleteProperty(document, 'visibilityState');
  },
);
it('keeps count and Running together in All sessions and recent activity, hides child counts, and dedupes overlap', async () => {
  const data = source();
  const base = structuredClone(exported.sessions[0].rows[0]);
  const parent = {
    session_id: 'main',
    host: 'codex',
    title: 'Main',
    evidence: 'native_spawn' as const,
  };
  const rows = [
    { ...base, host: 'codex', id: 'main', parent: null, title: 'Main' },
    { ...base, host: 'codex', id: 'child', parent, title: 'Child' },
  ];
  data.sessionsList = async () => ({ ...exported.sessions[0], rows, next: null });
  data.dashboard = async (days) => {
    const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
    report.lanes = rows.map((row, i) => ({
      session_id: row.id,
      host: 'codex',
      start_ms: report.lane_start_ms + i * 60000,
      end_ms: report.lane_end_ms - i * 60000,
    }));
    report.lane_sessions = rows.map((row) => ({
      session_id: row.id,
      host: 'codex',
      title: row.title,
      repo: row.repo,
      branch: row.branch,
      parent: row.parent,
      automated_review: false,
      started_at_ms: null,
      cost: null,
      pr_links: 0,
      inferred_pr_links: 0,
    }));
    return report;
  };
  data.compactions.read.mockImplementation(async (ids) => ({
    counts: ids.map((id) => ({ id, outcome: { state: 'count', count: 7, events: [] } })),
  }));
  Object.assign(data, {
    liveSessions: {
      read: async (ids: readonly string[], viewId: string | null) => ({
        view_id: viewId ?? 'synthetic-live-lease',
        states: ids.map((id) => ({ id, status: 'running' as const })),
      }),
      release: async () => {},
    },
  });
  render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={['/sessions']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await waitFor(() => expect(screen.getAllByLabelText('Recorded compactions: 7')).toHaveLength(2));
  await waitFor(() => expect(screen.getAllByText('Running', { exact: true })).toHaveLength(4));
  for (const count of screen.getAllByLabelText('Recorded compactions: 7')) {
    const row = count.closest('[role="row"]') ?? count.closest('li');
    expect(within(row as HTMLElement).getByText('Running', { exact: true })).toBeTruthy();
  }
  const table = screen.getByRole('table', { name: 'Indexed sessions' });
  const headers = within(table).getAllByRole('columnheader');
  const column = headers.findIndex((header) => header.textContent === 'Compactions');
  expect(column).toBeGreaterThan(-1);
  const mainRow = within(table).getByLabelText('Recorded compactions: 7').closest('[role="row"]');
  expect(within(mainRow as HTMLElement).getAllByRole('cell')[column].textContent).toBe('↺ 7');
  expect(table.querySelector('.xt-session-name .xt-compaction')).toBeNull();
  const childRow = within(table)
    .getByRole('link', { name: 'Open session Child, child' })
    .closest('[role="row"]');
  expect(within(childRow as HTMLElement).getAllByRole('cell')[column].textContent).toBe('');
  expect(
    within(screen.getByRole('region', { name: 'Recent indexed activity' })).getAllByLabelText(
      'Recorded compactions: 7',
    ),
  ).toHaveLength(1);
  for (const [ids] of data.compactions.read.mock.calls) expect(ids).toEqual(['main']);
});
