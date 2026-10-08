import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource, TrayControls } from '../../data/DataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import type { SessionRow } from '../../data/generated/SessionRow';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { TodaySummary } from '../../data/generated/TodaySummary';
import type { LiveSessionStatus } from '../live-session-status';
import { events, type DataEvent } from '../../data/ipc-names';
import { registrationTimeoutMs } from '../../data/subscribe-invalidation';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { costText, TrayPage } from './TrayPage';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
beforeEach(() => {
  vi.useFakeTimers({ now: Date.parse('2026-09-20T15:00:00Z') });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  delete document.documentElement.dataset.theme;
});

/**
 * Advance, then drain the promise → state → effect → read chains it started.
 * React commits at the end of each act, so every round is its own act.
 */
const flush = async (ms = 0) => {
  for (const step of [ms, 0, 0, 0])
    await act(async () => {
      await vi.advanceTimersByTimeAsync(step);
    });
};
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
};

/** A synthetic summary (not real history), read at the fake clock's now. */
function summary(overrides: Partial<TodaySummary> = {}): TodaySummary {
  const observed = Date.now();
  return {
    ...exported.today,
    date: '2026-09-20',
    timezone: 'America/Los_Angeles',
    clock: 'system',
    start_ms: Date.parse('2026-09-20T07:00:00Z'),
    observed_ms: observed,
    next_midnight_ms: Date.parse('2026-09-21T07:00:00Z'),
    empty: false,
    output: { state: 'recorded', output_tokens: 1_523_000, selected_responses: 412, sessions: 6 },
    cost: {
      ...exported.today.cost,
      state: 'priced',
      total_usd: 41.27,
      priced_subtotal_usd: 41.27,
      selected_observations: 412,
      priced_observations: 412,
      price_version: 'test',
    },
    agent: { active_ms: 5.7 * 3_600_000, sessions: 7 },
    human: { active_ms: 2 * 3_600_000, break_minutes: 45 },
    ...overrides,
  };
}

const elsewhere = async (): Promise<never> => {
  throw new Error('the tray reads only today');
};

const ready: NativeIndexStatus = {
  ...exported.native_index,
  phase: { phase: 'ready' },
  python: { state: 'available', path: 'python3' },
};

function nativeTray(initial = summary(), visible = false) {
  const listeners = new Map<DataEvent, Set<(payload?: unknown) => void>>();
  const shown = new Set<() => void>();
  const hidden = new Set<() => void>();
  const state = {
    visible,
    next: initial as TodaySummary | Error,
    registered: Promise.resolve() as Promise<unknown>,
    shownRegistered: Promise.resolve() as Promise<unknown>,
    hiddenRegistered: Promise.resolve() as Promise<unknown>,
    /** How the next data-event registrations answer. */
    listen: 'ok' as 'ok' | 'reject' | 'hold',
  };
  const stopShown = vi.fn((stop: () => void) => stop);
  const stopHidden = vi.fn((stop: () => void) => stop);
  const tray = {
    visible: vi.fn(async () => state.visible),
    hide: vi.fn(async () => {
      state.visible = false;
      hidden.forEach((listener) => listener());
    }),
    openMain: vi.fn(async () => {
      state.visible = false;
      hidden.forEach((listener) => listener());
    }),
    // A listener hears events only once its registration has completed.
    onShown: vi.fn(async (listener: () => void) => {
      await state.registered;
      await state.shownRegistered;
      shown.add(listener);
      return stopShown(() => void shown.delete(listener));
    }),
    onHidden: vi.fn(async (listener: () => void) => {
      await state.registered;
      await state.hiddenRegistered;
      hidden.add(listener);
      return stopHidden(() => void hidden.delete(listener));
    }),
  } satisfies TrayControls;
  const source = {
    kind: 'native',
    accountUsage: vi.fn(async () => new FixtureDataSource(exported).accountUsage()),
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    tray,
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    dashboard: vi.fn(async () => exported.dashboards[0]),
    tokensByHost: vi.fn(async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    })),
    environment: vi.fn(async () => exported.environments[0]),
    today: vi.fn(async (): Promise<TodaySummary> => {
      if (state.next instanceof Error) throw state.next;
      return structuredClone(state.next);
    }),
    sessionsList: vi.fn<DataSource['sessionsList']>(async () => ({
      window: exported.sessions[0].window,
      rows: [],
      next: null,
    })),
    // The tray reads only today; any other read is a failure here.
    sessionRow: elsewhere,
    sessionStretches: elsewhere,
    sessionTranscript: elsewhere,
    cancelSessionTranscript: elsewhere,
    pullRequests: elsewhere,
    pullRequestAnalytics: elsewhere,
    pullRequestSessions: elsewhere,
    refreshPullRequests: elsewhere,
    cancelPullRequestRefresh: elsewhere,
    nativeIndexStatus: async () => ready,
    subscribe: vi.fn(async (event: DataEvent, listener: (payload?: unknown) => void) => {
      if (state.listen === 'reject') throw new Error('adapter detail: listen rejected');
      if (state.listen === 'hold') await new Promise(() => {});
      const set = listeners.get(event) ?? new Set();
      set.add(listener);
      listeners.set(event, set);
      return () => void set.delete(listener);
    }),
  } satisfies DataSource;
  /** Data-event listeners currently heard, per event. */
  const heard = () => Object.values(events).map((event) => listeners.get(event)?.size ?? 0);
  const emit = (event: DataEvent, payload?: unknown) =>
    listeners.get(event)?.forEach((listener) => listener(payload));
  const show = async () => {
    state.visible = true;
    await act(async () => shown.forEach((listener) => listener()));
  };
  /** The native side's own visibility change, heard only by registered listeners. */
  const native = async (visible: boolean) => {
    state.visible = visible;
    await act(async () => (visible ? shown : hidden).forEach((listener) => listener()));
  };
  return { source, tray, state, emit, show, native, shown, hidden, heard };
}

function mount(source: DataSource) {
  return render(
    <StrictMode>
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter initialEntries={['/tray']}>
            <Routes>
              <Route path="/tray" element={<TrayPage />} />
              <Route path="/dashboard" element={<p>Dashboard route</p>} />
            </Routes>
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>
    </StrictMode>,
  );
}
const tile = (name: string) => screen.getByRole('region', { name });

it('reads only while shown, and again on every reopen', async () => {
  const { source, tray, show } = nativeTray();
  mount(source);
  await flush();
  // Created hidden: nothing is read until the menu-bar item shows it.
  expect(source.today).not.toHaveBeenCalled();
  expect(screen.queryByRole('region', { name: 'Today’s cost' })).toBeNull();
  await show();
  await flush();
  expect(source.today).toHaveBeenCalledTimes(1);
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
  await act(() => tray.hide());
  await flush();
  expect(screen.queryByRole('region', { name: 'Today’s cost' })).toBeNull();
  await show();
  await flush();
  expect(source.today).toHaveBeenCalledTimes(2);
  // Nothing else in the popover reads the Dashboard's reports.
  expect(source.dashboard).not.toHaveBeenCalled();
  expect(source.tokensByHost).not.toHaveBeenCalled();
  expect(source.environment).not.toHaveBeenCalled();
});

it('reads when it mounts after the window was already shown', async () => {
  const { source } = nativeTray(summary(), true);
  mount(source);
  await flush();
  expect(source.today).toHaveBeenCalledTimes(1);
  expect(screen.getByTestId('tray-observed').textContent).toBe('Sun, Sep 20 · as of 8:00 AM');
  expect(screen.queryByText(/Today in/)).toBeNull();
  expect(screen.queryByText(/Hours use the Dashboard/)).toBeNull();
  expect(screen.queryByTitle(/Agent hours count work after midnight/)).toBeNull();
});

it('a show while its listeners are still registering is seen by the snapshot taken after', async () => {
  const { source, tray, state, native } = nativeTray(summary(), false);
  const registration = deferred<void>();
  state.registered = registration.promise;
  mount(source);
  await flush();
  // No snapshot before both listeners exist.
  expect(tray.onShown).toHaveBeenCalled();
  expect(tray.onHidden).toHaveBeenCalled();
  expect(tray.visible).not.toHaveBeenCalled();
  // Shown in the gap: no registered listener can hear it.
  await native(true);
  registration.resolve();
  await flush();
  expect(tray.visible).toHaveBeenCalled();
  expect(source.today).toHaveBeenCalledTimes(1);
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
  // Listeners are live afterwards: a hide unmounts Today.
  await native(false);
  await flush();
  expect(screen.queryByRole('region', { name: 'Today’s cost' })).toBeNull();
});

it('a hide while its listeners are still registering leaves it closed', async () => {
  const { source, state, native } = nativeTray(summary(), true);
  const registration = deferred<void>();
  state.registered = registration.promise;
  mount(source);
  await flush();
  await native(false);
  registration.resolve();
  await flush();
  expect(source.today).not.toHaveBeenCalled();
  expect(screen.queryByRole('region', { name: 'Today’s cost' })).toBeNull();
});

it('unmounting before registration completes releases the listeners and takes no snapshot', async () => {
  const { source, tray, state, shown, hidden } = nativeTray(summary(), true);
  const registration = deferred<void>();
  state.registered = registration.promise;
  const view = mount(source);
  await flush();
  view.unmount();
  registration.resolve();
  await flush();
  expect(tray.visible).not.toHaveBeenCalled();
  expect(source.today).not.toHaveBeenCalled();
  // Each late registration is stopped as it lands.
  expect(shown.size).toBe(0);
  expect(hidden.size).toBe(0);
});

it('a show lost while its listener registers opens it, though a hide was heard first', async () => {
  const { source, tray, state, native } = nativeTray(summary(), false);
  const shownLate = deferred<void>();
  state.shownRegistered = shownLate.promise;
  mount(source);
  await flush();
  // The hide listener is live and hears a hide; the show listener is not yet.
  await native(false);
  await native(true);
  expect(source.today).not.toHaveBeenCalled();
  expect(tray.visible).not.toHaveBeenCalled();
  shownLate.resolve();
  await flush();
  // The snapshot runs regardless of the earlier hide and says "shown".
  expect(tray.visible).toHaveBeenCalled();
  expect(source.today).toHaveBeenCalledTimes(1);
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
});

it('a hide lost while its listener registers closes it, though a show was heard first', async () => {
  const { source, tray, state, native } = nativeTray(summary(), false);
  const hiddenLate = deferred<void>();
  state.hiddenRegistered = hiddenLate.promise;
  mount(source);
  await flush();
  // The show listener hears a show and Today mounts; the hide is lost.
  await native(true);
  await flush();
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
  await native(false);
  hiddenLate.resolve();
  await flush();
  expect(tray.visible).toHaveBeenCalled();
  expect(screen.queryByRole('region', { name: 'Today’s cost' })).toBeNull();
  // No read loop: the one read from the open, nothing after the close.
  await flush(60_000);
  expect(source.today).toHaveBeenCalledTimes(1);
});

it('a show heard while the snapshot is in flight outranks a "hidden" reply', async () => {
  const { source, tray, native } = nativeTray(summary(), false);
  const late = deferred<boolean>();
  tray.visible.mockImplementation(() => late.promise);
  mount(source);
  await flush();
  expect(tray.visible).toHaveBeenCalled();
  await native(true);
  late.resolve(false);
  await flush();
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
});

it('a hide heard before the initial visibility read keeps it closed', async () => {
  const { source, tray, state } = nativeTray(summary(), true);
  const late = deferred<boolean>();
  tray.visible.mockImplementation(() => late.promise);
  mount(source);
  await flush();
  // Hidden meanwhile; the read that began while it was shown lands after.
  await act(() => tray.hide());
  late.resolve(true);
  await flush();
  expect(state.visible).toBe(false);
  expect(source.today).not.toHaveBeenCalled();
});

it.each([
  ['priced', 24.79, 24.79, 573, 573, '$24.79', '573 of 573 responses priced'],
  ['priced', 0, 0, 1, 1, '$0.00', '1 of 1 response priced'],
  ['priced', 0.004, 0.004, 1, 1, '<$0.01', '1 of 1 response priced'],
  ['priced', 1234.56, 1234.56, 1, 1, '$1,235', '1 of 1 response priced'],
  ['priced', null, 0, 1, 1, '—', 'Today’s total is unavailable'],
  ['partial', null, 0.004, 3, 1, '—', 'Partial <$0.01, not a total · 1 of 3 responses priced'],
  ['unpriced', null, 0, 3, 0, '—', '0 of 3 responses priced'],
  ['none_recorded', null, 0, 0, 0, '—', 'No responses recorded yet today; nothing to price'],
] as const)(
  'shows %s cost with total %s as %s, with details only on hover and for assistive readers',
  async (state, total, subtotal, selected, priced, value, explanation) => {
    const next = summary({
      cost: {
        ...summary().cost,
        state,
        total_usd: total,
        priced_subtotal_usd: subtotal,
        selected_observations: selected,
        priced_observations: priced,
      },
    });
    const { source } = nativeTray(next, true);
    mount(source);
    await flush();
    const cost = tile('Today’s cost');
    expect(cost.textContent).toBe(`Today · cost${value}`);
    expect(within(cost).getByText(value)).toBeTruthy();
    expect(cost.title).toContain(explanation);
    expect(cost.title).toContain('Estimated at public API prices');
    expect(cost.title).toContain('subscriptions and tool fees are not included');
    expect(cost.getAttribute('aria-description')).toBe(cost.title);
    expect(costText(next)).toEqual({ value, title: cost.title });
    expect(screen.queryByRole('region', { name: 'Today’s output tokens' })).toBeNull();
    expect(screen.queryByText(/responses|API-equivalent|not a total|Today · output/)).toBeNull();
    expect(cost.parentElement?.className).toBe('xt-tray-tiles');
    expect(
      Array.from(cost.parentElement!.children).map((element) => element.getAttribute('aria-label')),
    ).toEqual(['Today’s agent hours', 'Today’s human hours', 'Today’s cost']);
    expect(cost.parentElement?.nextElementSibling?.getAttribute('aria-label')).toBe(
      'Account usage',
    );
  },
);

it('keeps the empty day free of invented hours or cost', async () => {
  const { source } = nativeTray(exported.today, true);
  mount(source);
  await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(tile('Today’s cost').textContent).toBe('Today · cost—');
  expect(tile('Today’s cost').title).toContain('nothing to price');
  expect(tile('Today’s agent hours').textContent).toContain('0 h 0 m');
  expect(tile('Today’s agent hours').textContent).toContain('No activity');
  expect(tile('Today’s agent hours').title).toBe('No agent activity recorded today');
  expect(screen.getByTestId('tray-observed').textContent).toBe('Tue, Sep 8 · as of 12:00 AM');
});

it('shows human hours and the shared usage widget without the unavailable filler', async () => {
  const { source, show } = nativeTray();
  mount(source);
  await show();
  await flush();
  expect(tile('Today’s human hours').textContent).toContain('2 h 0 m');
  expect(tile('Today’s human hours').textContent).toContain('Estimate');
  expect(tile('Today’s human hours').title).toBe(
    'Estimated from your messages. 45 min break length.',
  );
  expect(tile('Today’s human hours').getAttribute('aria-description')).toBe(
    tile('Today’s human hours').title,
  );
  expect(tile('Today’s agent hours').textContent).toContain('7 sessions');
  expect(tile('Today’s agent hours').textContent).toContain('5 h 42 m');
  expect(tile('Account usage')).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Active now' })).toBeNull();
  expect(screen.queryByRole('region', { name: 'Last rule fire' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Refresh usage' }).hasAttribute('disabled')).toBe(true);
});

it('keeps unknown human hours unknown', async () => {
  const { source, show } = nativeTray(summary({ human: { active_ms: null, break_minutes: 60 } }));
  mount(source);
  await show();
  await flush();
  expect(within(tile('Today’s human hours')).getByText('—')).toBeTruthy();
  expect(within(tile('Today’s human hours')).getByText('Unknown')).toBeTruthy();
  expect(tile('Today’s human hours').title).toBe(
    'Who sent some messages is unknown. 60 min break length.',
  );
});

it('reads a bounded recent list, uses host titles and live states, and stops on hide', async () => {
  const { source, tray, show } = nativeTray();
  const fixtureRows = exported.sessions[0].rows;
  const rows = Array.from({ length: 5 }, (_, index) => ({
    ...fixtureRows[0],
    id: `codex-tray-${index}`,
    host: 'codex',
    child_check: 'checked' as const,
    known_child: false,
    parent: null,
    title: `Saved title ${index}`,
    record_count: index + 1,
  }));
  source.sessionsList.mockResolvedValue({
    window: exported.sessions[0].window,
    rows,
    next: 'next-page',
  });
  const titles = {
    read: vi.fn(async (ids: string[]) => ({
      titles: ids.map((id) => ({ id, title: `Host ${id}` })),
    })),
    cancel: vi.fn(async () => {}),
  };
  const live = {
    read: vi.fn(async (ids: string[], token: string | null) => ({
      view_id: token ?? 'tray-lease',
      states: ids.map((id) => ({ id, status: 'running' as const })),
    })),
    release: vi.fn(async () => {}),
  };
  mount({ ...source, titles, liveSessions: live });
  await flush();
  expect(source.accountUsage).not.toHaveBeenCalled();
  expect(source.sessionsList).not.toHaveBeenCalled();
  expect(titles.read).not.toHaveBeenCalled();
  expect(live.read).not.toHaveBeenCalled();
  await show();
  await flush();
  expect(source.sessionsList).toHaveBeenCalledWith(
    { sort: 'recently_active', search: '', hosts: null, withPrs: false },
    null,
    7,
  );
  expect(within(tile('Recent sessions')).getAllByRole('listitem')).toHaveLength(3);
  expect(
    within(tile('Recent sessions')).getAllByRole('img', { name: 'Codex · Running' }),
  ).toHaveLength(3);
  expect(within(tile('Recent sessions')).queryByText('Running')).toBeNull();
  expect(within(tile('Recent sessions')).queryByText(/^Codex/)).toBeNull();
  expect(screen.getByText('Host codex-tray-0')).toBeTruthy();
  expect(titles.read.mock.calls.every(([ids]) => ids.length <= 3)).toBe(true);
  expect(live.read.mock.calls.every(([ids]) => ids.length <= 3)).toBe(true);
  await act(() => tray.hide());
  await flush();
  expect(live.release).toHaveBeenCalledWith('tray-lease');
  const counts = [
    source.accountUsage.mock.calls.length,
    source.sessionsList.mock.calls.length,
    titles.read.mock.calls.length,
    live.read.mock.calls.length,
  ];
  await flush(5 * 60_000);
  expect([
    source.accountUsage.mock.calls.length,
    source.sessionsList.mock.calls.length,
    titles.read.mock.calls.length,
    live.read.mock.calls.length,
  ]).toEqual(counts);
});

it.each([
  ['codex', 'running', 'Codex · Running', null],
  ['claude', 'running', 'Claude Code · Running', null],
  ['codex', 'waiting_approval', 'Codex', 'Waiting for approval'],
  ['claude', 'waiting_input', 'Claude Code', 'Waiting for input'],
  ['codex', 'idle', 'Codex', null],
  ['claude', 'unknown', 'Claude Code', null],
  ['cursor', 'running', 'Cursor', null],
  ['other', 'running', 'Unknown host: other', null],
] as const)(
  'uses the %s icon for %s and keeps waiting badges',
  async (host, status, label, badge) => {
    const { source, show } = nativeTray();
    source.sessionsList.mockResolvedValue({
      window: exported.sessions[0].window,
      rows: [
        {
          ...exported.sessions[0].rows[0],
          id: 'tray-icon',
          host,
          title: 'Synthetic tray session',
          repo: '/synthetic/repository',
          known_child: false,
          child_check: 'checked',
          parent: null,
        },
      ],
      next: null,
    });
    const live = {
      read: vi.fn(async (ids: string[], token: string | null) => ({
        view_id: token ?? 'tray-icon-lease',
        states: ids.map((id) => ({ id, status: status as LiveSessionStatus })),
      })),
      release: vi.fn(async () => {}),
    };
    mount({ ...source, liveSessions: live });
    await show();
    await flush();
    const recent = within(tile('Recent sessions'));
    const icon = recent.getByRole('img', { name: label });
    const row = recent.getByRole('listitem');
    expect(row.querySelector('.xt-tray-session-meta')?.contains(icon)).toBe(true);
    expect(recent.getByText('repository')).toBeTruthy();
    expect(recent.queryByText('Running')).toBeNull();
    expect(recent.queryByText(/^(Codex|Claude Code|Cursor|other)( ·|$)/)).toBeNull();
    if (badge) expect(recent.getByRole('status').textContent).toBe(badge);
    else expect(recent.queryByRole('status')).toBeNull();
    expect(icon.classList.contains('xt-lane-live-host')).toBe(label.endsWith(' · Running'));
    if (host === 'cursor' || host === 'other') expect(live.read).not.toHaveBeenCalled();
  },
);

it.each(['returned', 'referenced'] as const)(
  'skips sub-sessions with an eligible %s parent and fills three main rows before reading titles or live states',
  async (where) => {
    const { source, show } = nativeTray();
    const main = (id: string): SessionRow => ({
      ...exported.sessions[0].rows[0],
      id,
      host: 'codex',
      title: id,
      known_child: false,
      child_check: 'checked',
      parent: null,
    });
    const parent = main('main-parent');
    const children = Array.from({ length: 3 }, (_, index): SessionRow => ({
      ...main(`child-${index}`),
      known_child: true,
      child_check: 'child',
      parent: {
        session_id: parent.id,
        host: parent.host,
        title: parent.title,
        evidence: 'native_spawn',
      },
    }));
    const mains = [where === 'returned' ? parent : main('main-0'), main('main-1'), main('main-2')];
    source.sessionsList.mockResolvedValue({
      window: exported.sessions[0].window,
      rows: [...children, ...mains],
      next: 'later-page',
      referenced_parents:
        where === 'referenced'
          ? [
              {
                session_id: parent.id,
                host: parent.host,
                known_child: false,
                child_check: 'checked',
                parent: null,
              },
            ]
          : [],
    });
    const titles = {
      read: vi.fn(async (ids: string[]) => ({ titles: ids.map((id) => ({ id, title: id })) })),
      cancel: vi.fn(async () => {}),
    };
    const live = {
      read: vi.fn(async (ids: string[]) => ({
        view_id: 'lease',
        states: ids.map((id) => ({ id, status: 'running' as const })),
      })),
      release: vi.fn(async () => {}),
    };
    mount({ ...source, titles, liveSessions: live });
    await show();
    await flush();
    const recent = within(tile('Recent sessions'));
    expect(
      recent
        .getAllByRole('listitem')
        .map((row) => row.querySelector('.xt-tray-session-name')?.textContent),
    ).toEqual(mains.map((row) => row.id));
    expect(recent.getByText('Up to 3 recently active main sessions')).toBeTruthy();
    expect(recent.queryByText(/Sub-session/)).toBeNull();
    expect(titles.read).toHaveBeenCalled();
    expect(live.read).toHaveBeenCalled();
    for (const [ids] of titles.read.mock.calls) expect(ids).toEqual(mains.map((row) => row.id));
    expect(live.read.mock.calls.some(([ids]) => ids.length === 3)).toBe(true);
    for (const [ids] of live.read.mock.calls)
      if (ids.length > 0) expect(ids).toEqual(mains.map((row) => row.id).sort());
    for (const [, cursor] of source.sessionsList.mock.calls) expect(cursor).toBeNull();
  },
);

it('keeps children without a verified parent and sessions still being checked out of Recent sessions', async () => {
  const { source, show } = nativeTray();
  const base = { ...exported.sessions[0].rows[0], host: 'codex', parent: null };
  source.sessionsList.mockResolvedValue({
    window: exported.sessions[0].window,
    rows: [
      { ...base, id: 'child-no-parent', known_child: false, child_check: 'child' },
      { ...base, id: 'known-child-no-parent', known_child: true, child_check: 'checked' },
      { ...base, id: 'checking', known_child: false, child_check: 'checking' },
      { ...base, id: 'missing-check', known_child: false, child_check: null },
    ],
    next: null,
  });
  const titles = { read: vi.fn(async () => ({ titles: [] })), cancel: vi.fn(async () => {}) };
  const live = {
    read: vi.fn(async () => ({ view_id: 'lease', states: [] })),
    release: vi.fn(async () => {}),
  };
  mount({ ...source, titles, liveSessions: live });
  await show();
  await flush();
  expect(within(tile('Recent sessions')).getByText('No recent sessions to show.')).toBeTruthy();
  expect(titles.read).not.toHaveBeenCalled();
  expect(live.read).not.toHaveBeenCalled();
});

it('shows an empty recent list when the page contains only verified sub-sessions', async () => {
  const { source, show } = nativeTray();
  source.sessionsList.mockResolvedValue({
    window: exported.sessions[0].window,
    rows: Array.from({ length: 3 }, (_, index) => ({
      ...exported.sessions[0].rows[0],
      id: `verified-child-${index}`,
      host: 'codex',
      known_child: true,
      child_check: 'child' as const,
      parent: {
        session_id: 'parent',
        host: 'codex',
        title: null,
        evidence: 'native_spawn' as const,
      },
    })),
    next: 'later-page',
    referenced_parents: [
      {
        session_id: 'parent',
        host: 'codex',
        known_child: false,
        child_check: 'checked',
        parent: null,
      },
    ],
  });
  const titles = { read: vi.fn(async () => ({ titles: [] })), cancel: vi.fn(async () => {}) };
  const live = {
    read: vi.fn(async () => ({ view_id: 'lease', states: [] })),
    release: vi.fn(async () => {}),
  };
  mount({ ...source, titles, liveSessions: live });
  await show();
  await flush();
  expect(within(tile('Recent sessions')).getByText('No recent sessions to show.')).toBeTruthy();
  expect(titles.read).not.toHaveBeenCalled();
  expect(live.read).not.toHaveBeenCalled();
  for (const [, cursor] of source.sessionsList.mock.calls) expect(cursor).toBeNull();
});

it.each(['returned', 'referenced'] as const)(
  'hides a child whose %s parent is not shown, without reading its title or live state',
  async (where) => {
    const { source, show } = nativeTray();
    const base = exported.sessions[0].rows[0];
    const parent = {
      ...base,
      id: 'hidden-parent',
      host: 'codex',
      known_child: true,
      child_check: 'child' as const,
      parent: null,
    };
    const child = {
      ...base,
      id: 'hidden-child',
      host: 'codex',
      known_child: true,
      child_check: 'child' as const,
      parent: {
        session_id: parent.id,
        host: parent.host,
        title: null,
        evidence: 'native_spawn' as const,
      },
    };
    source.sessionsList.mockResolvedValue({
      window: exported.sessions[0].window,
      rows: where === 'returned' ? [child, parent] : [child],
      next: null,
      referenced_parents:
        where === 'referenced'
          ? [
              {
                session_id: parent.id,
                host: parent.host,
                known_child: true,
                child_check: 'child',
                parent: null,
              },
            ]
          : [],
    });
    const titles = { read: vi.fn(async () => ({ titles: [] })), cancel: vi.fn(async () => {}) };
    const live = {
      read: vi.fn(async () => ({ view_id: 'lease', states: [] })),
      release: vi.fn(async () => {}),
    };
    mount({ ...source, titles, liveSessions: live });
    await show();
    await flush();
    expect(within(tile('Recent sessions')).queryAllByRole('listitem')).toHaveLength(0);
    expect(titles.read).not.toHaveBeenCalled();
    expect(live.read).not.toHaveBeenCalled();
  },
);

it('refreshes on committed data while shown, not on scan progress or while hidden', async () => {
  const { source, tray, emit, show } = nativeTray();
  mount(source);
  await show();
  await flush();
  expect(source.today).toHaveBeenCalledTimes(1);
  emit(events.importReceived);
  await flush(500);
  expect(source.today).toHaveBeenCalledTimes(2);
  emit(events.nativeIndexStatus, { ...ready, phase: { phase: 'scanning' } });
  await flush(500);
  expect(source.today).toHaveBeenCalledTimes(2);
  await act(() => tray.hide());
  emit(events.turnCompleted);
  await flush(500);
  expect(source.today).toHaveBeenCalledTimes(2);
});

it('reads once more when a commit lands during its first read', async () => {
  vi.useRealTimers();
  const { source, emit, show } = nativeTray();
  const first = deferred<TodaySummary>();
  source.today.mockImplementationOnce(() => first.promise);
  mount(source);
  await show();
  await waitFor(() => expect(source.today).toHaveBeenCalledTimes(1));
  emit(events.importReceived);
  // Past the 500 ms coalescing: the in-flight read is joined, not restarted.
  await act(() => new Promise((done) => setTimeout(done, 600)));
  expect(source.today).toHaveBeenCalledTimes(1);
  // It may predate the commit, so it is read again when it lands.
  first.resolve(summary({ cost: { ...summary().cost, total_usd: 1, priced_subtotal_usd: 1 } }));
  await waitFor(() => expect(source.today).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy());
});

it('shows a first-read error with a working retry', async () => {
  const { source, state, show } = nativeTray();
  state.next = new Error('database is closed');
  mount(source);
  await show();
  await flush();
  expect(screen.getByRole('alert').textContent).toContain('Today’s figures could not be read.');
  state.next = summary();
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
});

/*
 * Real timers from here: under fake timers React does not commit a refetch the
 * query client starts on its own (the cache and observer update, the view does
 * not), so what a refresh renders is observed on the real clock.
 */
it('renders a committed import while shown, and keeps the figures when a refresh fails', async () => {
  vi.useRealTimers();
  const { source, state, emit, show } = nativeTray();
  mount(source);
  await show();
  await screen.findByText('$41.27');
  state.next = summary({ cost: { ...summary().cost, total_usd: 52, priced_subtotal_usd: 52 } });
  emit(events.importReceived);
  await waitFor(() => expect(within(tile('Today’s cost')).getByText('$52.00')).toBeTruthy());
  state.next = new Error('busy');
  emit(events.importReceived);
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Could not refresh'));
  expect(within(tile('Today’s cost')).getByText('$52.00')).toBeTruthy();
  state.next = summary();
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  expect(within(tile('Today’s cost')).getByText('$41.27')).toBeTruthy();
});

it('keeps one midnight timer while shown and reads the new date when it fires', async () => {
  const { source, tray, show } = nativeTray(
    summary({ next_midnight_ms: Date.parse('2026-09-20T15:10:00Z') }),
  );
  // Track the timers set for the midnight delay: exactly one may be pending.
  const midnight = 10 * 60 * 1000;
  const pending = new Set<unknown>();
  const set = globalThis.setTimeout;
  const clear = globalThis.clearTimeout;
  vi.spyOn(globalThis, 'setTimeout').mockImplementation(((handler: () => void, delay?: number) => {
    const id = set(() => {
      pending.delete(id);
      handler();
    }, delay);
    if (delay === midnight) pending.add(id);
    return id;
  }) as typeof setTimeout);
  vi.spyOn(globalThis, 'clearTimeout').mockImplementation(((id: ReturnType<typeof setTimeout>) => {
    pending.delete(id);
    clear(id);
  }) as typeof clearTimeout);
  mount(source);
  await show();
  await flush();
  expect(source.today).toHaveBeenCalledTimes(1);
  expect(pending.size).toBe(1);
  await flush(10 * 60 * 1000 - 1);
  expect(source.today).toHaveBeenCalledTimes(1);
  await flush(1);
  expect(source.today).toHaveBeenCalledTimes(2);
  // Hidden: the timer goes with the view.
  await act(() => tray.hide());
  await flush();
  expect(pending.size).toBe(0);
  await flush(24 * 3_600_000);
  expect(source.today).toHaveBeenCalledTimes(2);
});

it('hides on Escape and opens the main window from its button', async () => {
  const { source, tray, show } = nativeTray();
  mount(source);
  await show();
  await flush();
  fireEvent.keyDown(window, { key: 'Escape' });
  await flush();
  expect(tray.hide).toHaveBeenCalledTimes(1);
  await show();
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Open XTrace Desktop' }));
  await flush();
  expect(tray.openMain).toHaveBeenCalledTimes(1);
  // Hidden now, so Escape asks nothing more.
  fireEvent.keyDown(window, { key: 'Escape' });
  await flush();
  expect(tray.hide).toHaveBeenCalledTimes(1);
});

it('renders outside the Shell on a transparent surface in light and dark', async () => {
  for (const theme of ['light', 'dark'] as const) {
    localStorage.setItem('xt.theme', theme);
    const { source, show, tray } = nativeTray();
    const view = mount(source);
    await show();
    await flush();
    expect(document.documentElement.dataset.theme).toBe(theme);
    expect(document.documentElement.dataset.surface).toBe('tray');
    expect(screen.getByRole('main', { name: 'XTrace today' })).toBeTruthy();
    expect(screen.queryByRole('navigation')).toBeNull();
    await act(() => tray.hide());
    view.unmount();
    expect(document.documentElement.dataset.surface).toBeUndefined();
  }
  localStorage.removeItem('xt.theme');
});

it('in a fixture preview it is always shown and opens the Dashboard route', async () => {
  const { source } = nativeTray();
  const preview: DataSource = { ...source, tray: undefined, kind: 'fixture' };
  mount(preview);
  await flush();
  expect(source.today).toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Open XTrace Desktop' }));
  await flush();
  expect(screen.getByText('Dashboard route')).toBeTruthy();
});

it('shows its own failed live updates and reconnects them, catching up without a remount', async () => {
  const { source, tray, state, emit, heard } = nativeTray(summary(), true);
  state.listen = 'reject';
  mount(source);
  await flush();
  // The fence opens on the failure: Today is read, and the popover says its
  // live updates are unavailable, without any adapter detail.
  expect(source.today).toHaveBeenCalledTimes(1);
  expect(screen.getByRole('alert').textContent).toBe('Live updates are unavailable.');
  expect(document.body.textContent).not.toContain('adapter detail');
  expect(heard().every((count) => count === 0)).toBe(true);
  expect(screen.getByRole('button', { name: 'Open XTrace Desktop' })).toBeTruthy();
  const mounts = tray.onShown.mock.calls.length;

  // A change the failed attempt could not hear.
  state.next = summary({
    cost: { ...summary().cost, total_usd: 52, priced_subtotal_usd: 52 },
  });
  state.listen = 'ok';
  const button = screen.getByRole('button', { name: 'Reconnect' });
  button.focus();
  await act(async () => fireEvent.click(button));
  await flush();
  // Connected: the notice is gone, each event is heard exactly once, and the
  // catch-up re-read Today in the same view.
  expect(screen.queryByRole('button', { name: 'Reconnect' })).toBeNull();
  expect(screen.queryByText(/live updates/i)).toBeNull();
  expect(heard().every((count) => count === 1)).toBe(true);
  expect(source.today).toHaveBeenCalledTimes(2);
  expect(within(tile('Today’s cost')).getByText('$52.00')).toBeTruthy();
  expect(tray.onShown.mock.calls.length).toBe(mounts);
  // And it now hears committed data.
  await act(async () => emit(events.importReceived));
  await flush(600);
  expect(source.today).toHaveBeenCalledTimes(3);
});

it('a registration that outlasts its wait fails visibly; a failed retry stays retryable', async () => {
  const { source, state, heard } = nativeTray(summary(), true);
  state.listen = 'hold';
  mount(source);
  await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  await flush(registrationTimeoutMs);
  expect(screen.getByRole('alert').textContent).toBe('Live updates are unavailable.');
  expect(source.today).toHaveBeenCalledTimes(1);

  const button = screen.getByRole('button', { name: 'Reconnect' });
  button.focus();
  await act(async () => fireEvent.click(button));
  // Connecting: announced, the same button, focus kept, and a second click
  // starts nothing more.
  expect(screen.getByRole('status').textContent).toBe('Reconnecting live updates…');
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBe(button);
  expect(button.getAttribute('aria-disabled')).toBe('true');
  expect(document.activeElement).toBe(button);
  const calls = source.subscribe.mock.calls.length;
  await act(async () => fireEvent.click(button));
  expect(source.subscribe.mock.calls.length).toBe(calls);
  await flush(registrationTimeoutMs);
  expect(screen.getByRole('alert').textContent).toBe('Live updates are still unavailable.');
  expect(button.getAttribute('aria-disabled')).toBe('false');
  expect(document.activeElement).toBe(button);
  expect(heard().every((count) => count === 0)).toBe(true);
  // A failed attempt re-reads nothing.
  expect(source.today).toHaveBeenCalledTimes(1);
});
