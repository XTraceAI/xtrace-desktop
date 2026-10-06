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
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, SessionListFilter, SessionTitleControls } from '../data/DataSource';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionPage } from '../data/generated/SessionPage';
import type { SessionRow } from '../data/generated/SessionRow';
import type { SessionTitles } from '../data/generated/SessionTitles';
import { events } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
import { TITLE_BATCH, useSessionTitles, type TitleRow } from './session-titles';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));

const exported = fixture as FixtureExport;
const unavailable = async (): Promise<never> => {
  throw new Error('Not part of this test.');
};

/** One host-title read the test answers by hand. */
interface Pending {
  ids: readonly string[];
  readId: string;
  resolve: (answer: SessionTitles) => void;
  reject: (error: Error) => void;
}

/** Title controls whose reads wait until the test answers them. */
function titleControls() {
  const reads: Pending[] = [];
  const cancelled: string[] = [];
  const controls: SessionTitleControls = {
    read: (ids, readId) =>
      new Promise((resolve, reject) => reads.push({ ids: [...ids], readId, resolve, reject })),
    cancel: async (readId) => {
      cancelled.push(readId);
    },
  };
  return { controls, reads, cancelled };
}

const answer = (read: Pending, titles: Record<string, string>) =>
  act(async () => {
    read.resolve({ titles: Object.entries(titles).map(([id, title]) => ({ id, title })) });
  });

function row(id: string, patch: Partial<SessionRow> = {}): SessionRow {
  return { ...structuredClone(exported.sessions[0].rows[0]), id, title: null, ...patch };
}

function source(overrides: Partial<DataSource>, titles?: SessionTitleControls): DataSource {
  return {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    dashboard: async (days) => {
      const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
      // These Sessions title tests exercise the table's read queue; tests
      // below supply their own lanes when recent titles are in scope.
      report.lanes = [];
      report.lane_sessions = [];
      report.lanes_total = 0;
      report.lanes_truncated = false;
      return report;
    },
    tokensByHost: async (days) => {
      const report = exported.dashboards.find((r) => r.window.days === days)!;
      return { window: report.window, hosts: report.tokens_by_host };
    },
    today: async () => exported.today,
    environment: async (days) => exported.environments.find((r) => r.window.days === days)!,
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    subscribe: async () => () => {},
    ...overrides,
    ...(titles ? { titles } : {}),
  };
}

function mount(data: DataSource, path: string) {
  return render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

const sessionLinks = () =>
  within(screen.getByRole('table', { name: 'Indexed sessions' }))
    .getAllByRole('link')
    .map((link) => link.textContent);

it('shows a host title in Sessions for exactly the rows of the page it read', async () => {
  const { controls, reads } = titleControls();
  const list = vi.fn(async (): Promise<SessionPage> => ({
    window: exported.sessions[0].window,
    rows: [row('session-a'), row('session-b', { title: 'Saved title' }), row('session-c')],
    next: null,
  }));
  mount(source({ sessionsList: list }, controls), '/sessions');
  await screen.findByText('Saved title');
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual(['session-a', 'session-b', 'session-c']);
  // Until the answer, every row keeps what it had: identity, or a saved title.
  expect(sessionLinks()).toEqual(['Session session-', 'Saved title', 'Session session-']);
  await answer(reads[0], { 'session-a': 'Host title A', 'session-b': 'Host title B' });
  expect(sessionLinks()).toEqual(['Host title A', 'Host title B', 'Session session-']);
  const link = screen.getByRole('link', { name: 'Open session Host title A, session-a' });
  expect(link.getAttribute('title')).toBe('Host title A · session-a');
  // One read per page; nothing is read again while the rows stay the same.
  expect(reads).toHaveLength(1);
  // The list query itself is the same one it always was.
  expect(list).toHaveBeenCalledWith({ search: '', hosts: null, withPrs: false }, null, 7);
});

it('reads the next page on its own and never more than one page at once', async () => {
  const { controls, reads } = titleControls();
  const first = Array.from({ length: TITLE_BATCH }, (_, index) => row(`first-${index}`));
  const second = [row('second-0'), row('second-1')];
  const list = vi.fn(async (_: SessionListFilter, after: string | null): Promise<SessionPage> => ({
    window: exported.sessions[0].window,
    rows: after ? second : first,
    next: after ? null : 'page-two',
  }));
  mount(source({ sessionsList: list }, controls), '/sessions');
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual(first.map((entry) => entry.id));
  await answer(reads[0], { 'first-0': 'First page title' });
  fireEvent.click(await screen.findByRole('button', { name: 'Load more sessions' }));
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual(['second-0', 'second-1']);
  await answer(reads[1], { 'second-1': 'Second page title' });
  expect(screen.getByText('First page title')).toBeTruthy();
  expect(screen.getByText('Second page title')).toBeTruthy();
  // Pagination is the list's own: the second page was asked for by cursor.
  expect(list).toHaveBeenLastCalledWith({ search: '', hosts: null, withPrs: false }, 'page-two', 7);
});

it('cancels a read for a list that changed and never shows its late answer', async () => {
  const { controls, reads, cancelled } = titleControls();
  const list = vi.fn(async (filter: SessionListFilter): Promise<SessionPage> => ({
    window: exported.sessions[0].window,
    rows: filter.search ? [row('shared'), row('match')] : [row('shared'), row('other')],
    next: null,
  }));
  mount(source({ sessionsList: list }, controls), '/sessions');
  await waitFor(() => expect(reads).toHaveLength(1));
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'ma' } });
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(cancelled).toEqual([reads[0].readId]);
  expect(reads[1].ids).toEqual(['shared', 'match']);
  // The old list's answer arrives late, naming a row the new list also shows.
  await answer(reads[0], { shared: 'Stale answer', other: 'Stale other' });
  expect(screen.queryByText('Stale answer')).toBeNull();
  await answer(reads[1], { match: 'Current answer' });
  expect(sessionLinks()).toEqual(['Session shared', 'Current answer']);
});

it('replaces a read whose row changed version, and shows the fallback until the new read', async () => {
  const { controls, reads, cancelled } = titleControls();
  let records = 1;
  const listeners = new Set<() => void>();
  const list = vi.fn(async (): Promise<SessionPage> => ({
    window: exported.sessions[0].window,
    rows: [row('growing', { title: 'Saved title', record_count: records }), row('steady')],
    next: null,
  }));
  mount(
    source(
      {
        sessionsList: list,
        subscribe: async (event, listener) => {
          if (event !== events.turnCompleted) return () => {};
          const heard = () => listener();
          listeners.add(heard);
          return () => listeners.delete(heard);
        },
      },
      controls,
    ),
    '/sessions',
  );
  const grow = async () => {
    records += 1;
    const called = list.mock.calls.length;
    act(() => listeners.forEach((listener) => listener()));
    await waitFor(() => expect(list.mock.calls.length).toBeGreaterThan(called));
  };
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual(['growing', 'steady']);
  // The same session gains a record before the first answer: that read is
  // replaced, and its answer about the old version is never shown.
  await grow();
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(cancelled).toEqual([reads[0].readId]);
  expect(reads[1].ids).toEqual(['growing', 'steady']);
  await answer(reads[0], { growing: 'Old version', steady: 'Old steady' });
  expect(sessionLinks()).toEqual(['Saved title', 'Session steady']);
  await answer(reads[1], { growing: 'Current version', steady: 'Steady title' });
  expect(sessionLinks()).toEqual(['Current version', 'Steady title']);
  // Answered, then changed again: the old title gives way to the fallback
  // until the new version is read, and only the changed row is read.
  await grow();
  await waitFor(() => expect(reads).toHaveLength(3));
  expect(sessionLinks()).toEqual(['Saved title', 'Steady title']);
  expect(reads[2].ids).toEqual(['growing']);
  await answer(reads[2], { growing: 'Newest version' });
  expect(sessionLinks()).toEqual(['Newest version', 'Steady title']);
});

it('replaces a read when a refresh of the same list drops a row it named', async () => {
  const { controls, reads, cancelled } = titleControls();
  let refreshed = false;
  const listeners = new Set<() => void>();
  const list = vi.fn(async (): Promise<SessionPage> => ({
    window: exported.sessions[0].window,
    rows: refreshed ? [row('kept'), row('arrived')] : [row('kept'), row('dropped')],
    next: null,
  }));
  mount(
    source(
      {
        sessionsList: list,
        subscribe: async (event, listener) => {
          if (event !== events.turnCompleted) return () => {};
          const heard = () => listener();
          listeners.add(heard);
          return () => listeners.delete(heard);
        },
      },
      controls,
    ),
    '/sessions',
  );
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual(['kept', 'dropped']);
  // Same filters and range, but the refreshed list no longer shows a row the
  // running read named: that read is cancelled and the rows now shown are read.
  refreshed = true;
  act(() => listeners.forEach((listener) => listener()));
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(cancelled).toEqual([reads[0].readId]);
  expect(reads[1].ids).toEqual(['kept', 'arrived']);
  await answer(reads[0], { kept: 'Stale kept', dropped: 'Stale dropped' });
  expect(sessionLinks()).toEqual(['Session kept', 'Session arrived']);
  await answer(reads[1], { arrived: 'Arrived title' });
  expect(sessionLinks()).toEqual(['Session kept', 'Arrived title']);
});

/** The hook alone, over rows the test replaces at will. */
function titlesFor(initial: readonly TitleRow[], options?: { keepResolved?: boolean }) {
  const { controls, reads, cancelled } = titleControls();
  const data = source({}, controls);
  const hook = renderHook(({ rows }) => useSessionTitles('scope', rows, options), {
    initialProps: { rows: initial },
    wrapper: ({ children }: { children: ReactNode }) => (
      <DataProvider source={data}>{children}</DataProvider>
    ),
  });
  return { hook, reads, cancelled };
}

it('keeps a running read when rows are only added, then reads the added rows', async () => {
  const { hook, reads, cancelled } = titlesFor([{ id: 'a', version: 1 }]);
  await waitFor(() => expect(reads).toHaveLength(1));
  hook.rerender({
    rows: [
      { id: 'a', version: 1 },
      { id: 'b', version: 1 },
    ],
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(reads).toHaveLength(1);
  expect(cancelled).toEqual([]);
  await answer(reads[0], { a: 'Title A' });
  expect(hook.result.current('a', 1)).toBe('Title A');
  // A title is for the version it was read at.
  expect(hook.result.current('a', 2)).toBeNull();
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual(['b']);
});

it('keeps a resolved title across a version change only until a read fails', async () => {
  const { hook, reads } = titlesFor([{ id: 'a', version: 1 }], { keepResolved: true });
  await waitFor(() => expect(reads).toHaveLength(1));
  await answer(reads[0], { a: 'Title A' });
  hook.rerender({ rows: [{ id: 'a', version: 2 }] });
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(hook.result.current('a', 2)).toBe('Title A');
  // Another row never borrows it.
  expect(hook.result.current('b', 2)).toBeNull();
  await act(async () => reads[1].reject(new Error('title read was refused')));
  expect(hook.result.current('a', 2)).toBeNull();
});

it('keeps identities when a title read fails, and does not retry it', async () => {
  const { controls, reads } = titleControls();
  mount(
    source(
      {
        sessionsList: async () => ({
          window: exported.sessions[0].window,
          rows: [row('failing')],
          next: null,
        }),
      },
      controls,
    ),
    '/sessions',
  );
  await waitFor(() => expect(reads).toHaveLength(1));
  await act(async () => reads[0].reject(new Error('transcript read was refused')));
  expect(sessionLinks()).toEqual(['Session failing']);
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(reads).toHaveLength(1);
});

it('does not offer to search titles it cannot search', async () => {
  mount(source({}), '/sessions');
  const box = await screen.findByLabelText('Search sessions');
  expect(box.getAttribute('placeholder')).toBe('Search ID, repo or branch…');
});

/** An IntersectionObserver the test drives: rows are in view when it says. */
class FakeObserver {
  static all: FakeObserver[] = [];
  readonly targets = new Set<Element>();
  constructor(
    private readonly callback: IntersectionObserverCallback,
    readonly options?: IntersectionObserverInit,
  ) {
    FakeObserver.all.push(this);
  }
  observe(target: Element) {
    this.targets.add(target);
  }
  unobserve(target: Element) {
    this.targets.delete(target);
  }
  disconnect() {
    this.targets.clear();
  }
  takeRecords() {
    return [];
  }
  /** Report exactly these session rows as in view, and every other as out. */
  show(ids: readonly string[]) {
    const entries = [...this.targets].map(
      (target) =>
        ({
          target,
          isIntersecting: ids.includes((target as HTMLElement).dataset.visibleId ?? ''),
        }) as IntersectionObserverEntry,
    );
    act(() => this.callback(entries, this as unknown as IntersectionObserver));
  }
}
beforeEach(() => {
  FakeObserver.all = [];
  vi.stubGlobal('IntersectionObserver', FakeObserver);
  return () => vi.unstubAllGlobals();
});

function lanesReport(days: number, ids: readonly string[]): DashboardMetrics {
  const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
  const minute = 60_000;
  report.lanes = ids.map((session_id, index) => ({
    session_id,
    host: 'claude',
    start_ms: report.lane_end_ms - (index + 1) * 30 * minute,
    end_ms: report.lane_end_ms - (index + 1) * 30 * minute + 10 * minute,
  }));
  report.lanes_total = ids.length;
  report.lanes_truncated = false;
  report.lane_sessions = ids.map((session_id) => ({
    session_id,
    host: 'claude',
    repo: '/repo/fixture',
    branch: 'main',
    title: null,
    automated_review: false,
    started_at_ms: null,
    pr_links: 0,
    inferred_pr_links: 0,
    cost: null,
    child_check: 'checked' as const,
  }));
  return report;
}

const laneNames = () =>
  [...within(screen.getByRole('table', { name: 'Session lanes' })).getAllByRole('link')].map(
    (link) => link.textContent,
  );

it('reads Dashboard titles only for lane rows in view, including rows scrolled into view', async () => {
  const { controls, reads } = titleControls();
  const ids = Array.from({ length: 12 }, (_, index) => `lane-${index}`);
  mount(source({ dashboard: async (days) => lanesReport(days, ids) }, controls), '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  const observer = FakeObserver.all[0];
  // The table's own scroll area is the viewport, not the page.
  expect((observer.options?.root as HTMLElement).classList.contains('xt-table-scroll')).toBe(true);
  // Nothing is read before a row is known to be in view.
  expect(reads).toHaveLength(0);
  observer.show(ids.slice(0, 4));
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual(ids.slice(0, 4));
  await answer(reads[0], { 'lane-1': 'Visible title' });
  expect(laneNames()[1]).toBe('Visible title');
  // Scrolling brings later rows into view; only they are read next.
  observer.show(ids.slice(3, 7));
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual(ids.slice(4, 7));
  await answer(reads[1], { 'lane-6': 'Scrolled title' });
  expect(laneNames()[6]).toBe('Scrolled title');
  // Rows never in view were never named.
  expect(reads.flatMap((read) => read.ids)).not.toContain('lane-11');
  // A title read earlier stays while its row is out of view.
  expect(laneNames()[1]).toBe('Visible title');
});

it('replaces a Dashboard read when scrolling shows other rows before its answer', async () => {
  const { controls, reads, cancelled } = titleControls();
  const ids = Array.from({ length: 12 }, (_, index) => `lane-${index}`);
  mount(source({ dashboard: async (days) => lanesReport(days, ids) }, controls), '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  const observer = FakeObserver.all[0];
  observer.show(ids.slice(0, 4));
  await waitFor(() => expect(reads).toHaveLength(1));
  // Every row it named scrolls away before the answer: that read is cancelled,
  // and the rows now in view are read instead of waiting behind it.
  observer.show(ids.slice(6, 10));
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(cancelled).toEqual([reads[0].readId]);
  expect(reads[1].ids).toEqual(ids.slice(6, 10));
  await answer(reads[0], { 'lane-1': 'Offscreen answer' });
  await answer(reads[1], { 'lane-7': 'In view title' });
  expect(laneNames()[7]).toBe('In view title');
  expect(screen.queryByText('Offscreen answer')).toBeNull();
  // The cancelled answer taught nothing: scrolling back reads those rows again.
  observer.show(ids.slice(0, 4));
  await waitFor(() => expect(reads).toHaveLength(3));
  expect(reads[2].ids).toEqual(ids.slice(0, 4));
  await answer(reads[2], { 'lane-1': 'Fresh answer' });
  expect(laneNames()[1]).toBe('Fresh answer');
});

it('keeps an active Dashboard lane title while its new activity is read, then shows what that read finds', async () => {
  const { controls, reads, cancelled } = titleControls();
  let activity = 0;
  const listeners = new Set<() => void>();
  const dashboard = vi.fn(async (days: number) => {
    const report = lanesReport(days, ['active', 'steady']);
    report.lanes[0].end_ms += activity * 1000;
    return report;
  });
  mount(
    source(
      {
        dashboard,
        subscribe: async (event, listener) => {
          if (event !== events.turnCompleted) return () => {};
          const heard = () => listener();
          listeners.add(heard);
          return () => listeners.delete(heard);
        },
      },
      controls,
    ),
    '/dashboard',
  );
  const moveActive = async () => {
    activity += 1;
    const called = dashboard.mock.calls.length;
    act(() => listeners.forEach((listener) => listener()));
    await waitFor(() => expect(dashboard.mock.calls.length).toBeGreaterThan(called));
  };
  await screen.findByRole('table', { name: 'Session lanes' });
  FakeObserver.all[0].show(['active', 'steady']);
  await waitFor(() => expect(reads).toHaveLength(1));
  await answer(reads[0], { active: 'Active title', steady: 'Steady title' });
  expect(laneNames()).toEqual(['Active title', 'Steady title']);
  // The active session's latest span moves: only it is read again, and its
  // last title stays while that read runs.
  await moveActive();
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual(['active']);
  expect(laneNames()).toEqual(['Active title', 'Steady title']);
  // It moves again before the answer: that read is replaced, its late answer
  // is never shown, and the last title still stays.
  await moveActive();
  await waitFor(() => expect(reads).toHaveLength(3));
  expect(cancelled).toEqual([reads[1].readId]);
  await answer(reads[1], { active: 'Superseded title' });
  expect(laneNames()).toEqual(['Active title', 'Steady title']);
  // A rename replaces it.
  await answer(reads[2], { active: 'Renamed title' });
  expect(laneNames()).toEqual(['Renamed title', 'Steady title']);
  // A read that finds no title gives the row its fallback.
  await moveActive();
  await waitFor(() => expect(reads).toHaveLength(4));
  expect(laneNames()).toEqual(['Renamed title', 'Steady title']);
  await answer(reads[3], {});
  expect(laneNames()).toEqual(['Session active', 'Steady title']);
  // Another range starts with no titles, whatever the last one showed.
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await screen.findByRole('table', { name: 'Session lanes' });
  FakeObserver.all.at(-1)!.show(['active', 'steady']);
  await waitFor(() => expect(reads).toHaveLength(5));
  expect(laneNames()).toEqual(['Session active', 'Session steady']);
});

it('drops a Dashboard title answer that arrives after the range changed', async () => {
  const { controls, reads, cancelled } = titleControls();
  mount(
    source({ dashboard: async (days) => lanesReport(days, ['lane-a', 'lane-b']) }, controls),
    '/dashboard',
  );
  await screen.findByRole('table', { name: 'Session lanes' });
  FakeObserver.all[0].show(['lane-a', 'lane-b']);
  await waitFor(() => expect(reads).toHaveLength(1));
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(cancelled).toEqual([reads[0].readId]));
  await answer(reads[0], { 'lane-a': 'Seven-day answer' });
  expect(screen.queryByText('Seven-day answer')).toBeNull();
  // The 30-day report's rows, in view in whichever table now shows them.
  await screen.findByRole('table', { name: 'Session lanes' });
  FakeObserver.all.at(-1)!.show(['lane-a', 'lane-b']);
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual(['lane-a', 'lane-b']);
  await answer(reads[1], { 'lane-b': 'Thirty-day answer' });
  expect(laneNames()).toEqual(['Session lane-a', 'Thirty-day answer']);
});

it('cancels a running title read when the view goes away', async () => {
  const { controls, reads, cancelled } = titleControls();
  const view = mount(
    source(
      {
        sessionsList: async () => ({
          window: exported.sessions[0].window,
          rows: [row('leaving')],
          next: null,
        }),
      },
      controls,
    ),
    '/sessions',
  );
  await waitFor(() => expect(reads).toHaveLength(1));
  view.unmount();
  expect(cancelled).toEqual([reads[0].readId]);
});
