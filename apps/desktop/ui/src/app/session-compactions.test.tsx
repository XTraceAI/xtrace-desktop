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
import type { CompactionCount } from '../data/generated/CompactionCount';
import type { SessionCompactions } from '../data/generated/SessionCompactions';
import { events } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { CompactionBadge, useSessionCompactions } from './session-compactions';
import { AppRoutes } from './AppRoutes';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
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
    expect.stringContaining('not a quality score'),
  );
});
it('shows a fork as its own count plus the count before the fork, in plain words', async () => {
  render(
    <ThemeProvider>
      <CompactionBadge
        outcome={{ state: 'count', count: 3, events: [], inherited: { state: 'count', count: 2 } }}
      />
      <CompactionBadge
        outcome={{ state: 'count', count: 3, events: [], inherited: { state: 'count', count: 0 } }}
      />
      <CompactionBadge
        outcome={{
          state: 'count',
          count: 3,
          events: [],
          inherited: { state: 'unknown', reason: 'missing' },
        }}
      />
      <CompactionBadge outcome={{ state: 'count', count: 4, events: [] }} />
    </ThemeProvider>,
  );
  const words = (from: string) => `Recorded compactions: 3 + ${from}`;
  const both = screen.getByLabelText(words('2'));
  expect(both.textContent).toBe('↺ 3 + 2');
  // The colour follows everything this conversation's context went through.
  expect(both.getAttribute('data-step')).toBe('5');
  const zero = screen.getByLabelText(words('0'));
  expect(zero.textContent).toBe('↺ 3 + 0');
  expect(zero.getAttribute('data-step')).toBe('3');
  const unknown = screen.getByLabelText(words('unknown'));
  expect(unknown.textContent).toBe('↺ 3 + ?');
  // A session that is not a fork keeps one number.
  expect(screen.getByLabelText('Recorded compactions: 4').textContent).toBe('↺ 4');
  fireEvent.focus(zero);
  expect(await screen.findByRole('tooltip')).toHaveProperty(
    'textContent',
    expect.stringContaining(
      '3 in this conversation + 0 in the conversation it was forked from, before the fork.',
    ),
  );
  fireEvent.blur(zero);
  fireEvent.focus(unknown);
  await waitFor(() =>
    expect(screen.getByRole('tooltip').textContent).toContain(
      '3 in this conversation + an unknown number in the conversation it was forked from, before the fork. The saved conversation it was forked from could not be found.',
    ),
  );
});
it('says in plain words why a reviewer snapshot or an unsplit copy has no count', async () => {
  render(
    <ThemeProvider>
      <CompactionBadge outcome={{ state: 'unknown', reason: 'snapshot' }} />
      <CompactionBadge outcome={{ state: 'unknown', reason: 'copied_history' }} />
    </ThemeProvider>,
  );
  const [snapshot, copied] = screen.getAllByLabelText('Recorded compactions: unknown');
  expect(snapshot.textContent).toContain('—');
  fireEvent.focus(snapshot);
  expect(await screen.findByRole('tooltip')).toHaveProperty(
    'textContent',
    expect.stringContaining(
      'This approval reviewer saved only part of the conversation it reviewed, with earlier compactions left out, so they can’t be counted.',
    ),
  );
  fireEvent.blur(snapshot);
  fireEvent.focus(copied);
  await waitFor(() =>
    expect(screen.getByRole('tooltip').textContent).toContain(
      'This conversation was copied from another one, and its own compactions can’t be told apart from the copied ones.',
    ),
  );
});
it('shows a copied-history fork as its own count plus the copied count', async () => {
  render(
    <ThemeProvider>
      <CompactionBadge
        outcome={{
          state: 'count',
          count: 95,
          events: [],
          inherited: { state: 'count', count: 49, copied: true },
        }}
      />
    </ThemeProvider>,
  );
  const badge = screen.getByLabelText('Recorded compactions: 95 + 49');
  expect(badge.textContent).toBe('↺ 95 + 49');
  fireEvent.focus(badge);
  expect(await screen.findByRole('tooltip')).toHaveProperty(
    'textContent',
    expect.stringContaining(
      '95 in this conversation + 49 copied from the conversation it was forked from.',
    ),
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
it('keeps count and Running together in All sessions and hides child counts', async () => {
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(private callback: IntersectionObserverCallback) {}
      observe(target: Element) {
        queueMicrotask(() =>
          this.callback(
            [{ target, isIntersecting: true } as IntersectionObserverEntry],
            this as unknown as IntersectionObserver,
          ),
        );
      }
      unobserve() {}
      disconnect() {}
    },
  );
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
      child_check: 'checked' as const,
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
  await waitFor(() => expect(screen.getAllByLabelText('Recorded compactions: 7')).toHaveLength(1));
  // Each child is collapsed under its main session; opened, it
  // reads its live state as its own row.
  await waitFor(() => expect(screen.getAllByLabelText(/ · Running$/)).toHaveLength(1));
  fireEvent.click(screen.getByRole('button', { name: '1 loaded sub-session of Main' }));
  await waitFor(() => expect(screen.getAllByLabelText(/ · Running$/)).toHaveLength(2));
  for (const count of screen.getAllByLabelText('Recorded compactions: 7')) {
    const row = count.closest('[role="row"]') ?? count.closest('li');
    expect(within(row as HTMLElement).getByLabelText(/ · Running$/)).toBeTruthy();
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
  for (const [ids] of data.compactions.read.mock.calls) expect(ids).toEqual(['main']);
});
it('shows a spinner only while a row with no count yet is being read', async () => {
  render(
    <ThemeProvider>
      <CompactionBadge reading />
      <CompactionBadge reading outcome={{ state: 'count', count: 3, events: [] }} />
      <CompactionBadge outcome={{ state: 'unknown', reason: 'missing' }} />
    </ThemeProvider>,
  );
  const spinning = screen.getByLabelText('Recorded compactions: reading');
  expect(spinning.querySelector('.xt-compaction-spinner')).not.toBeNull();
  expect(spinning.textContent).not.toContain('—');
  // A known count stays on screen during a re-read; a settled unknown is a dash.
  expect(
    screen.getByLabelText('Recorded compactions: 3').querySelector('.xt-compaction-spinner'),
  ).toBeNull();
  expect(screen.getByLabelText('Recorded compactions: unknown').textContent).toContain('—');
});
it('reports reading for a requested row until its answer arrives, not after or during a refresh', async () => {
  const data = source();
  let outcome: SessionCompactions['counts'][number]['outcome'] = {
    state: 'unknown',
    reason: 'missing',
  };
  const pending: Array<() => void> = [];
  data.compactions.read.mockImplementation(
    (ids) =>
      new Promise((resolve) => {
        pending.push(() => resolve({ counts: ids.map((id) => ({ id, outcome })) }));
      }),
  );
  const answer = async () => {
    await act(async () => {
      for (const resolve of pending.splice(0)) resolve();
    });
  };
  const { result } = renderHook(() => useSessionCompactions('list', ['main']), {
    wrapper: wrapper(data),
  });
  await waitFor(() => expect(pending.length).toBeGreaterThan(0));
  expect(result.current.reading('main')).toBe(true);
  // A row this view did not ask for is never reading.
  expect(result.current.reading('other')).toBe(false);
  // Answer every read, including any follow-up the app queued at start.
  await waitFor(async () => {
    await answer();
    expect(result.current.reading('main')).toBe(false);
  });
  // Settled unknown: a dash, not a spinner.
  expect(result.current('main')).toEqual(outcome);
  outcome = { state: 'count', count: 1, events: [] };
  act(() => void window.dispatchEvent(new Event('focus')));
  await waitFor(() => expect(pending.length).toBeGreaterThan(0));
  // A refresh keeps the last answer on screen, so no spinner either.
  expect(result.current('main')).toEqual({ state: 'unknown', reason: 'missing' });
  expect(result.current.reading('main')).toBe(false);
  await answer();
  await waitFor(() => expect(result.current('main')).toEqual(outcome));
});
it('keeps known counts with no spinner when more rows load, the filters change or the page changes', async () => {
  const data = source();
  const pending: Array<{ ids: readonly string[]; resolve: (answer: SessionCompactions) => void }> =
    [];
  data.compactions.read.mockImplementation(
    (ids) => new Promise((resolve) => pending.push({ ids, resolve })),
  );
  const answer = async (count: number) => {
    await act(async () => {
      for (const { ids, resolve } of pending.splice(0))
        resolve({
          counts: ids.map((id) => ({ id, outcome: { state: 'count', count, events: [] } })),
        });
    });
  };
  const three = { state: 'count', count: 3, events: [] };
  const { result, rerender, unmount } = renderHook(
    ({ scope, ids }) => useSessionCompactions(scope, ids),
    { initialProps: { scope: 'sessions:7', ids: ['a'] }, wrapper: wrapper(data) },
  );
  await waitFor(() => expect(pending.length).toBeGreaterThan(0));
  await answer(3);
  await waitFor(() => expect(result.current('a')).toEqual(three));
  // More rows load: the known row keeps its number, only the new one spins.
  rerender({ scope: 'sessions:7', ids: ['a', 'b'] });
  await waitFor(() => expect(pending.length).toBeGreaterThan(0));
  expect(result.current('a')).toEqual(three);
  expect(result.current.reading('a')).toBe(false);
  expect(result.current.reading('b')).toBe(true);
  await answer(3);
  await waitFor(() => expect(result.current('b')).toEqual(three));
  // Another filter: counts do not depend on it.
  rerender({ scope: 'sessions:30', ids: ['b', 'a'] });
  await waitFor(() => expect(pending.length).toBeGreaterThan(0));
  expect(result.current('a')).toEqual(three);
  expect(result.current.reading('a')).toBe(false);
  await answer(4);
  await waitFor(() => expect(result.current('a')).toEqual({ ...three, count: 4 }));
  unmount();
  // Another page reading from the same source starts from the known counts.
  const other = renderHook(() => useSessionCompactions('dashboard', ['a', 'b']), {
    wrapper: wrapper(data),
  });
  await waitFor(() => expect(other.result.current).toBeTypeOf('function'));
  expect(other.result.current('a')).toEqual({ ...three, count: 4 });
  expect(other.result.current.reading('a')).toBe(false);
});
it('starts a new source with nothing known', async () => {
  const first = source();
  first.compactions.read.mockImplementation(async (ids) => ({
    counts: ids.map((id) => ({ id, outcome: { state: 'count', count: 5, events: [] } })),
  }));
  const before = renderHook(() => useSessionCompactions('list', ['a']), {
    wrapper: wrapper(first),
  });
  await waitFor(() =>
    expect(before.result.current('a')).toEqual({ state: 'count', count: 5, events: [] }),
  );
  before.unmount();
  const second = source();
  second.compactions.read.mockImplementation(() => new Promise(() => {}));
  const after = renderHook(() => useSessionCompactions('list', ['a']), {
    wrapper: wrapper(second),
  });
  await waitFor(() => expect(after.result.current.reading('a')).toBe(true));
  expect(after.result.current('a')).toBeUndefined();
});
/** A source whose reads answer `auto` until `manual` is set, then wait to be answered. */
function ordered(auto: (id: string) => CompactionCount | undefined) {
  const data = source();
  const state = { manual: false };
  const calls: Array<{
    ids: readonly string[];
    resolve: (answer: SessionCompactions) => void;
    reject: (error: Error) => void;
  }> = [];
  data.compactions.read.mockImplementation((ids) =>
    state.manual
      ? new Promise((resolve, reject) => calls.push({ ids, resolve, reject }))
      : Promise.resolve({
          counts: ids.flatMap((id) => {
            const outcome = auto(id);
            return outcome ? [{ id, outcome }] : [];
          }),
        }),
  );
  // The next unanswered read of exactly these rows.
  const call = async (ids: readonly string[]) => {
    let found: (typeof calls)[number] | undefined;
    await waitFor(() => {
      found = calls.find((c) => JSON.stringify(c.ids) === JSON.stringify(ids));
      expect(found).toBeDefined();
    });
    calls.splice(calls.indexOf(found!), 1);
    return found!;
  };
  const reply = (target: (typeof calls)[number], outcomes: Record<string, CompactionCount>) =>
    act(async () =>
      target.resolve({
        counts: Object.entries(outcomes).map(([id, outcome]) => ({ id, outcome })),
      }),
    );
  return { data, state, call, reply };
}
it.each(['transport', 'replaced'] as const)(
  'does not let a Dashboard fork retry ending in %s bring back an older own count',
  async (failure) => {
    const base: CompactionCount = { state: 'count', count: 1, events: [] };
    const { data, state, call, reply } = ordered(() => base);
    const dashboard = renderHook(
      ({ scope }) => useSessionCompactions(scope, ['fork'], { retryTransient: true }),
      { initialProps: { scope: 'dashboard:0' }, wrapper: wrapper(data) },
    );
    const sessions = renderHook(({ scope }) => useSessionCompactions(scope, ['fork', 'other']), {
      initialProps: { scope: 'sessions:0' },
      wrapper: wrapper(data),
    });
    await waitFor(() => expect(sessions.result.current('other')).toEqual(base));
    state.manual = true;
    // The Dashboard's read starts first, then the Sessions page's.
    dashboard.rerender({ scope: 'dashboard:1' });
    const first = await call(['fork']);
    sessions.rerender({ scope: 'sessions:1' });
    const later = await call(['fork', 'other']);
    const partial: CompactionCount = {
      state: 'count',
      count: 3,
      events: [{ at_ms: 3, trigger: 'auto' }],
      inherited: { state: 'unknown', reason: 'replaced' },
    };
    const whole: CompactionCount = {
      state: 'count',
      count: 4,
      events: [{ at_ms: 4, trigger: 'manual' }],
      inherited: { state: 'count', count: 2 },
    };
    await reply(first, { fork: partial });
    await reply(later, { fork: whole, other: base });
    await waitFor(() => expect(sessions.result.current('fork')).toEqual(whole));
    // The Dashboard's retry, 250 ms later, finds nothing new.
    const retry = await call(['fork']);
    if (failure === 'transport') await act(async () => retry.reject(new Error('refused')));
    else await reply(retry, { fork: { state: 'unknown', reason: 'replaced' } });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(dashboard.result.current('fork')).toEqual(whole);
    expect(sessions.result.current('fork')).toEqual(whole);
  },
);
it('keeps the answer of the read that started last when two views answer out of order', async () => {
  const base: CompactionCount = { state: 'count', count: 1, events: [] };
  const { data, state, call, reply } = ordered((id) => (id === 'y' ? undefined : base));
  const one = renderHook(({ scope }) => useSessionCompactions(scope, ['x', 'y']), {
    initialProps: { scope: 'one:0' },
    wrapper: wrapper(data),
  });
  const two = renderHook(({ scope }) => useSessionCompactions(scope, ['x', 'y', 'z']), {
    initialProps: { scope: 'two:0' },
    wrapper: wrapper(data),
  });
  await waitFor(() => expect(two.result.current('z')).toEqual(base));
  state.manual = true;
  const count = (n: number): CompactionCount => ({ state: 'count', count: n, events: [] });
  const limit: CompactionCount = { state: 'unknown', reason: 'limit' };
  one.rerender({ scope: 'one:1' });
  const older = await call(['x', 'y']);
  two.rerender({ scope: 'two:1' });
  const newer = await call(['x', 'y', 'z']);
  await reply(newer, { x: count(5), y: limit, z: base });
  await waitFor(() => expect(one.result.current('x')).toEqual(count(5)));
  expect(one.result.current('y')).toEqual(limit);
  // The older read answers last: its count for x is stale, but y had only a
  // temporary unknown, which an older real count still replaces.
  await reply(older, { x: count(4), y: count(7) });
  await waitFor(() => expect(two.result.current('y')).toEqual(count(7)));
  expect(two.result.current('x')).toEqual(count(5));
  expect(one.result.current('x')).toEqual(count(5));
  // A newer read that only kept the last value does not block an older
  // read's newer count.
  one.rerender({ scope: 'one:2' });
  const third = await call(['x', 'y']);
  two.rerender({ scope: 'two:2' });
  const fourth = await call(['x', 'y', 'z']);
  await reply(fourth, { x: limit, y: count(7), z: base });
  expect(one.result.current('x')).toEqual(count(5));
  await reply(third, { x: count(6), y: count(7) });
  await waitFor(() => expect(two.result.current('x')).toEqual(count(6)));
});
