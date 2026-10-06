import { act, render, screen, fireEvent, waitFor, within, cleanup } from '@testing-library/react';
import { Link, MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import type { DataSource } from '../data/DataSource';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { LiveSessionSnapshot } from '../data/generated/LiveSessionSnapshot';
import fixture from '../../fixtures/F1.json';
import { events } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
import { SessionsPage } from './SessionsPage';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const all = { search: '', hosts: null, withPrs: false };
/** Leave exactly one host checked in the compact menu, as a user would. */
async function onlyHost(label: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Filter by host' }));
  for (const other of ['Claude Code', 'Codex', 'Cursor']) {
    const box = (await screen.findByRole('checkbox', { name: other })) as HTMLInputElement;
    if ((other === label) !== box.checked) fireEvent.click(box);
  }
}

it('renders stored metadata and resets pagination when filters change', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList');
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'not-present' } });
  await screen.findByText('No sessions match these filters.');
  await waitFor(() =>
    expect(list).toHaveBeenLastCalledWith({ ...all, search: 'not-present' }, null, 7),
  );
  await onlyHost('Codex');
  await waitFor(() =>
    expect(list).toHaveBeenLastCalledWith(
      { ...all, search: 'not-present', hosts: ['codex'] },
      null,
      7,
    ),
  );
  fireEvent.click(screen.getByRole('switch', { name: 'With PRs only' }));
  await waitFor(() =>
    expect(list).toHaveBeenLastCalledWith(
      { search: 'not-present', hosts: ['codex'], withPrs: true },
      null,
      7,
    ),
  );
});

it("shows each session's window measurements and keeps a real zero apart from an unknown", async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  const measured = row.metrics;
  if (measured.state !== 'indexed') throw new Error('F1 exports one indexed session');
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: [
      row,
      // An indexed session that did nothing in this window: measured zeros, and
      // M-04 counters that stay unknown without a selected response.
      {
        ...row,
        id: 'quiet-session',
        hands_off: { state: 'measured', n: 0, median_min: null },
        metrics: {
          ...measured,
          events: 0,
          human_messages: 0,
          tool_calls: 0,
          agent_ms: 0,
          tokens: {
            ...measured.tokens,
            selected_responses: 0,
            measured_responses: 0,
            sessions: 0,
            measured_sessions: 0,
            counters: {
              input_tokens: null,
              output_tokens: null,
              cache_read_tokens: null,
              cache_creation_tokens: null,
              total_tokens: null,
            },
          },
        },
      },
      // A row whose human classification and tool counts are unmeasured keeps
      // its other numbers.
      {
        ...row,
        id: 'unclassified',
        metrics: { ...measured, human_messages: null, tool_calls: null },
      },
    ],
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  // Per-row measurements, apart from the range summary above the list, which
  // reports the whole range rather than these rows.
  const table = within(screen.getByRole('table', { name: 'Indexed sessions' }));
  const rowFor = (id: string) =>
    within(table.getByText(`Session ${id.slice(0, 8)}`).closest('[role="row"]') as HTMLElement);
  // F1: five human messages, five tool calls, 150 output tokens (not the
  // 1,100 total), a 23-minute active span and a three-minute hands-off median.
  expect(rowFor('00000000').getAllByText('5').length).toBe(2);
  expect(rowFor('00000000').getByText('150')).toBeTruthy();
  expect(rowFor('00000000').queryByText('1.1K')).toBeNull();
  // Agent time as hours and remaining minutes, with the exact measurement.
  expect(rowFor('00000000').getByText('0h23m')).toBeTruthy();
  expect(rowFor('00000000').getByText('23 minutes, exactly 1,380,000 ms')).toBeTruthy();
  expect(rowFor('00000000').getByText('3')).toBeTruthy();
  expect(rowFor('00000000').getByText(/median of 5 stretches/)).toBeTruthy();
  // The quiet window reports zeros, never an em dash, for what it measured;
  // no stretch is no median, not a zero-minute one.
  expect(rowFor('quiet-se').getAllByText('0').length).toBe(2);
  expect(rowFor('quiet-se').getByText('0h00m')).toBeTruthy();
  expect(
    rowFor('quiet-se').getByText(/Unmeasured: Selected responses do not all state output/),
  ).toBeTruthy();
  expect(
    rowFor('quiet-se').getByText(/Unmeasured: No hands-off stretch in this window/),
  ).toBeTruthy();
  expect(rowFor('unclassif').getByText(/Unmeasured: Human classification/)).toBeTruthy();
  expect(
    rowFor('unclassif').getByText(/Unmeasured: A record in this window did not state/),
  ).toBeTruthy();
});

// The visible header is abbreviated for the reference's density; the spoken
// name stays whole, so the definition is still reachable by its column name.
it.each([
  ['Human messages', 'M-02'],
  ['Tool calls', 'M-17'],
  ['Agent minutes', 'M-05'],
  ['Hands-off median minutes', 'M-09'],
  ['Output tokens', 'M-04'],
] as const)('opens the %s definition from its keyboard-reachable header', async (label, ruleId) => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  const trigger = screen.getByRole('button', { name: `${label}, definition ${ruleId}` });
  // Reachable and operable by keyboard, not by hover alone.
  trigger.focus();
  expect(document.activeElement).toBe(trigger);
  fireEvent.focus(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id);
  expect(tooltip.textContent).toBeTruthy();
  expect(tooltip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
  // Escape dismisses it without moving focus off the header.
  fireEvent.keyDown(trigger, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  expect(document.activeElement).toBe(trigger);
});

it('does not pretend a PR filter is applied before that query is implemented', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList');
  render(
    <MemoryRouter initialEntries={['/sessions?pr=example']}>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText(/Pull request filtering is not available/)).toBeTruthy();
  expect(list).not.toHaveBeenCalled();
});

it('loads the next page, refreshes after import and resets pages for a host filter', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let count = 25;
  const row = (fixture as FixtureExport).sessions[0].rows[0];
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (filter, after) => {
    const page = (fixture as FixtureExport).sessions[0];
    if (filter.hosts) return { window: page.window, rows: [], next: null };
    return after
      ? {
          window: page.window,
          rows: [{ ...row, id: 'second-session', model: 'model-9' }],
          next: null,
        }
      : { window: page.window, rows: [{ ...row, model: `model-${count}` }], next: 'page-two' };
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('model-25');
  // The rows scroll inside their own region, which takes the page's remaining
  // height rather than a viewport cap; the header sticks above the rows and
  // Load more stays outside the scroll, under it, so it is always reachable.
  const scroll = screen.getByRole('region', { name: 'Indexed sessions scroll area' });
  expect(scroll.style.maxHeight).toBe('');
  expect(scroll.querySelector('.xt-table-head')!.getAttribute('data-sticky')).toBe('true');
  const more = screen.getByRole('button', { name: 'Load more sessions' });
  expect(scroll.contains(more)).toBe(false);
  expect(scroll.compareDocumentPosition(more) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  fireEvent.click(more);
  await screen.findByText('Session second-s');
  expect(list).toHaveBeenCalledWith(all, 'page-two', 7);
  count = 26;
  act(() => source.emit(events.importReceived));
  await screen.findByText('model-26');
  expect(screen.getByText('Session second-s')).toBeTruthy();
  await onlyHost('Cursor');
  await screen.findByText('No sessions match these filters.');
  expect(screen.queryByText('Session second-s')).toBeNull();
  // A new filter starts again from the first page, not from page two.
  expect(list).toHaveBeenLastCalledWith({ ...all, hosts: ['cursor'] }, null, 7);
});

it('recovers from a failed query using the retry control', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi
    .spyOn(source, 'sessionsList')
    .mockRejectedValue(new Error('database unavailable'));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('alert');
  list.mockRestore();
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await screen.findByText('Session 00000000');
});

it('keeps indexed rows visible with a warning when native indexing is disabled', async () => {
  const fixtureSource = new FixtureDataSource(fixture as FixtureExport);
  const source: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: () => fixtureSource.appInfo(),
    dbCounts: () => fixtureSource.dbCounts(),
    dashboard: (windowDays) => fixtureSource.dashboard(windowDays),
    tokensByHost: (windowDays) => fixtureSource.tokensByHost(windowDays),
    today: () => fixtureSource.today(),
    environment: (windowDays) => fixtureSource.environment(windowDays),
    nativeIndexStatus: () => fixtureSource.nativeIndexStatus(),
    sessionRow: (sessionId, windowDays) => fixtureSource.sessionRow(sessionId, windowDays),
    sessionStretches: (sessionId, windowDays) =>
      fixtureSource.sessionStretches(sessionId, windowDays),
    sessionTranscript: () => fixtureSource.sessionTranscript(),
    cancelSessionTranscript: () => fixtureSource.cancelSessionTranscript(),
    sessionsList: (filter, after, windowDays) =>
      fixtureSource.sessionsList(filter, after, windowDays),
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    subscribe: (event, listener) => fixtureSource.subscribe(event, listener),
  };
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  // One small triangle at the end of the heading's controls; its name states the state.
  const flag = await screen.findByRole('button', { name: 'History limits: Index unavailable' });
  expect(flag.closest('.xt-sessions-filters')).toBeTruthy();
  expect(screen.queryByText(/Indexing is unavailable/)).toBeNull();
  // Focus reads the sentence in a passive tooltip that holds no control.
  fireEvent.focus(flag);
  const gist = await screen.findByRole('tooltip');
  expect(flag.getAttribute('aria-describedby')).toBe(gist.id);
  expect(gist.textContent).toBe(
    'Index unavailable: Indexing is unavailable; showing previously indexed history.',
  );
  expect(within(gist).queryByRole('link')).toBeNull();
  fireEvent.blur(flag);
  // Pressing it opens the explanation with the way to Settings, and Escape returns focus.
  fireEvent.click(flag);
  const dialog = await screen.findByRole('dialog', { name: 'Index status' });
  expect(within(dialog).getByTestId('sessions-index-status').textContent).toContain(
    'Indexing is unavailable; showing previously indexed history.',
  );
  expect(
    within(dialog).getByRole('link', { name: 'View indexing status' }).getAttribute('href'),
  ).toBe('/settings');
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(document.activeElement).toBe(flag);
});

it.each(['initial scan', 'pending host', 'degraded watcher'] as const)(
  'warns during %s and clears after a healthy status event',
  async (scenario) => {
    const source = new FixtureDataSource(fixture as FixtureExport);
    const complete: NativeIndexStatus = {
      ...(fixture as FixtureExport).native_index,
      phase: { phase: 'ready' },
      freshness: { freshness: 'live' },
      hosts: [
        {
          host: 'claude',
          state: 'complete',
          detail: null,
          sessions_imported: 1,
          sessions_partial: 0,
          sessions_skipped: 0,
          skipped_conversations: [],
          skipped_conversations_omitted: 0,
          records_new: 25,
          records_enriched: 0,
          diagnostics: 0,
        },
      ],
    };
    let status = structuredClone(complete);
    if (scenario === 'initial scan') {
      status.phase = { phase: 'scanning' };
      status.hosts = [];
    } else if (scenario === 'pending host') {
      status.hosts[0].state = 'pending';
    } else {
      status.freshness = { freshness: 'degraded', reason: 'watcher stopped' };
    }
    vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => status);
    render(
      <MemoryRouter>
        <DataProvider source={source}>
          <SessionsPage />
        </DataProvider>
      </MemoryRouter>,
    );
    const state = scenario === 'degraded watcher' ? 'Updates interrupted' : 'Indexing';
    const flag = await screen.findByRole('button', { name: `History limits: ${state}` });
    expect(await screen.findByText('Session 00000000')).toBeTruthy();
    fireEvent.focus(flag);
    expect((await screen.findByRole('tooltip')).textContent).toMatch(
      scenario === 'degraded watcher'
        ? /^Updates interrupted: Live indexing is interrupted/
        : /^Indexing: History is still being indexed/,
    );
    fireEvent.blur(flag);
    status = complete;
    act(() => source.emit(events.nativeIndexStatus));
    // Healthy is quiet: no triangle at all.
    await waitFor(() => expect(screen.queryByTestId('sessions-issues')).toBeNull());
    expect(screen.queryByRole('button', { name: /^History limits/ })).toBeNull();
    expect(screen.getByText('Session 00000000')).toBeTruthy();
  },
);

it('states the session start as started and keeps first recorded work in the details', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: [
      { ...row, id: 'with-start', started_at_ms: Date.UTC(2026, 8, 6, 9, 12) },
      // No known start: unknown, never its first recorded work.
      { ...row, id: 'no-start', started_at_ms: null, has_conflict: true },
    ],
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session with-sta');
  const headers = screen.getAllByRole('columnheader').map((cell) => cell.textContent);
  expect(headers).toContain('started');
  expect(headers).not.toContain('first recorded');
  expect(headers).not.toContain('records');
  expect(headers).not.toContain('source');
  const rowFor = (id: string) =>
    within(screen.getByText(`Session ${id}`).closest('[role="row"]') as HTMLElement);
  expect(
    screen
      .getByText('Session with-sta')
      .closest('[role="row"]')!
      .querySelector('time')!
      .getAttribute('dateTime'),
  ).toBe('2026-09-06T09:12:00.000Z');
  expect(rowFor('no-start').getByText(/Unmeasured: No start is known/)).toBeTruthy();
  // The diagnostic facts are not erased: a conflict stays visible in the row,
  // and the rest is one keyboard-operable disclosure away.
  expect(rowFor('no-start').getByText(/source conflict/)).toBeTruthy();
  const expand = screen.getByRole('button', { name: 'Expand no-start' });
  expand.focus();
  expect(document.activeElement).toBe(expand);
  fireEvent.click(expand);
  expect(expand.getAttribute('aria-expanded')).toBe('true');
  const details = document.getElementById(expand.getAttribute('aria-controls')!)!;
  expect(details.textContent).toContain('First recorded');
  expect(details.textContent).toContain('including copies inherited from a fork');
  expect(details.textContent).toContain(`Records${row.record_count.toLocaleString()}`);
  expect(details.textContent).toContain('Some source observations disagree');
  expect(details.textContent).toContain('Total tokens');
  fireEvent.click(expand);
  expect(expand.getAttribute('aria-expanded')).toBe('false');
});

it('leads a row with a saved title or its identity, never an invented one', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: [
      {
        ...row,
        id: 'known-1',
        title: 'Saved synthetic title',
        repo: '/Users/example/code/atlas',
        branch: 'feat/navigation',
      },
      { ...row, id: 'known-2', repo: 'C:\\code\\atlas', branch: null },
      { ...row, id: 'known-3', repo: null, branch: 'main' },
      { ...row, id: 'known-4', repo: null, branch: null, model: null },
    ],
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  const titled = await screen.findByRole('link', {
    name: 'Open session Saved synthetic title, known-1',
  });
  expect(titled.textContent).toBe('Saved synthetic title');
  // Without a saved title the identity leads; nothing is derived from content.
  for (const id of ['known-2', 'known-3', 'known-4']) {
    expect(screen.getByRole('link', { name: `Open session ${id}` }).textContent).toBe(
      `Session ${id}`,
    );
  }
  // The repository's own name and the branch sit under it.
  const meta = (id: string) =>
    screen
      .getByText(id === 'known-1' ? 'Saved synthetic title' : `Session ${id}`)
      .closest('.xt-session-name')!
      .querySelector('.xt-session-meta')!;
  expect(meta('known-1').textContent).toBe(`atlas ⑂ feat/navigation · ${row.model}`);
  expect(meta('known-2').getAttribute('title')).toBe('C:\\code\\atlas · Unknown branch');
  expect(meta('known-3').textContent).toContain('Unknown repository');
  expect(meta('known-3').textContent).toContain('main');
  expect(meta('known-4').textContent).toContain('Unknown model');
  const headers = screen.getAllByRole('columnheader').map((cell) => cell.textContent);
  expect(headers).toContain('session · repo · branch');
});

it('badges every recorded pull request with its own evidence and keeps the rest in details', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  const link = (number: number, confidence: 'exact' | 'sha' | 'inferred') => ({
    repository: 'example/atlas',
    number,
    url: `https://github.com/example/atlas/pull/${number}`,
    confidence,
    title: null,
  });
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: [
      {
        ...row,
        id: 'three-prs',
        pr_links: [link(9, 'exact'), link(3, 'sha'), link(7, 'inferred')],
      },
      { ...row, id: 'inferred', pr_links: [link(7, 'inferred')] },
      { ...row, id: 'no-prs', pr_links: [] },
    ],
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session three-pr');
  const rowFor = (id: string) =>
    within(screen.getByText(`Session ${id.slice(0, 8)}`).closest('[role="row"]') as HTMLElement);
  expect(
    rowFor('three-prs').getByRole('img', {
      name: 'Pull request #9 in example/atlas, exact evidence',
    }),
  ).toBeTruthy();
  expect(rowFor('three-prs').getByText('+1')).toBeTruthy();
  // An inferred link is badged as inferred, never presented as exact.
  const inferred = rowFor('inferred').getByRole('img', {
    name: 'Pull request #7 in example/atlas, inferred evidence',
  });
  expect(inferred.getAttribute('data-confidence')).toBe('inferred');
  // Nothing here opens the network.
  expect(inferred.closest('a')).toBeNull();
  expect(rowFor('no-prs').getByText('No recorded pull request')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Expand three-prs' }));
  expect(await screen.findByText(/example\/atlas#7 · inferred/)).toBeTruthy();
  expect(screen.getByText(/example\/atlas#3 · commit/)).toBeTruthy();
});

/** The range summary's tile, by the label the reference gives it. */
const tile = (label: string) =>
  within(screen.getByRole('region', { name: 'Range summary' }))
    .getByText(label)
    .closest('button')!;

const f1Report = (days: number) => {
  const report = (fixture as FixtureExport).dashboards.find((entry) => entry.window.days === days);
  if (!report) throw new Error('F1 exports 7, 14 and 30-day reports');
  return structuredClone(report);
};

const recent = () => within(screen.getByRole('region', { name: 'Recent indexed activity' }));

/** These tests decide which table cells are actually inside their scroll area. */
class LiveRowsObserver {
  static current: LiveRowsObserver;
  readonly targets = new Set<Element>();
  constructor(
    private readonly callback: IntersectionObserverCallback,
    readonly options: IntersectionObserverInit,
  ) {
    LiveRowsObserver.current = this;
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
  show(ids: readonly string[]) {
    act(() =>
      this.callback(
        [...this.targets].map(
          (target) =>
            ({
              target,
              isIntersecting: ids.includes((target as HTMLElement).dataset.visibleId ?? ''),
            }) as IntersectionObserverEntry,
        ),
        this as unknown as IntersectionObserver,
      ),
    );
  }
}

it('gives eight mixed recent chats priority, caps visible Claude/Codex rows together, and releases on changes', async () => {
  vi.stubGlobal('IntersectionObserver', LiveRowsObserver);
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  const recentIds = Array.from({ length: 9 }, (_, index) => `recent-chat-${index}`);
  report.lanes = recentIds.map((session_id, index) => ({
    host: index % 2 === 0 ? 'claude' : 'codex',
    session_id,
    start_ms: report.lane_end_ms - (index + 1) * 1000 - 100,
    end_ms: report.lane_end_ms - (index + 1) * 1000,
  }));
  report.lane_sessions = recentIds.map((session_id, index) => ({
    ...report.lane_sessions[0],
    host: index % 2 === 0 ? 'claude' : 'codex',
    session_id,
    title: session_id,
  }));
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  const page = (fixture as FixtureExport).sessions[0];
  const tableIds = Array.from({ length: 12 }, (_, index) => `table-chat-${index}`);
  const rows = [recentIds[0], ...tableIds]
    .map((id, index) => ({
      ...page.rows[0],
      id,
      host: index % 2 === 0 ? 'claude' : 'codex',
      title: id,
    }))
    .concat(
      { ...page.rows[0], id: 'claude-chat', host: 'claude', title: 'Claude chat' },
      { ...page.rows[0], id: 'cursor-chat', host: 'cursor', title: 'Cursor chat' },
    );
  vi.spyOn(source, 'sessionsList').mockImplementation(async (filter) => ({
    ...page,
    rows: filter.withPrs ? [] : rows,
    next: null,
  }));
  let issued = 0;
  const read = vi.fn<NonNullable<DataSource['liveSessions']>['read']>(async (ids, viewId) => ({
    view_id: viewId ?? `native-table-lease-${++issued}`,
    states: ids.map((id) => ({ id, status: 'running' })),
  }));
  const release = vi.fn(async () => {});
  Object.assign(source, { liveSessions: { read, release } });
  const view = render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  const table = await screen.findByRole('table', { name: 'Indexed sessions' });
  await recent().findAllByText('Running');
  expect(read).toHaveBeenCalledTimes(2);
  expect(read.mock.calls[0]).toEqual([[], null]);
  expect(read.mock.calls[1]).toEqual([recentIds.slice(0, 8).sort(), 'native-table-lease-1']);
  const rowFor = (id: string) =>
    within(
      within(table)
        .getByRole('link', {
          name: `Open session ${id}, ${id}`,
        })
        .closest('[role="row"]')! as HTMLElement,
    );
  expect(rowFor(tableIds[0]).getByText('Unknown')).toBeTruthy();
  const observer = LiveRowsObserver.current;
  expect(observer.options.root).toBe(table.closest('.xt-table-scroll'));
  observer.show([recentIds[0], ...tableIds.slice(0, 10), 'claude-chat', 'cursor-chat']);
  await rowFor(tableIds[0]).findByText('Running');
  const selected = [...recentIds.slice(0, 8), ...tableIds.slice(0, 8)].sort();
  expect(read).toHaveBeenLastCalledWith(selected, expect.any(String));
  expect(rowFor(tableIds[8]).getByText('Unknown')).toBeTruthy();
  expect(rowFor(tableIds[10]).getByText('Unknown')).toBeTruthy();
  expect(
    within(table)
      .getByText('Claude chat')
      .closest('[role="row"]')!
      .querySelector('[data-live-status]')!
      .getAttribute('data-live-status'),
  ).toBe('unknown');
  expect(
    within(table)
      .getByText('Cursor chat')
      .closest('[role="row"]')!
      .querySelector('[data-live-status]'),
  ).toBeNull();
  expect(read.mock.calls.every(([ids]) => !ids.includes('cursor-chat') && ids.length <= 16)).toBe(
    true,
  );
  const visibleId = read.mock.calls.at(-1)![1];
  observer.show([tableIds[11], 'claude-chat']);
  expect(rowFor(tableIds[0]).getByText('Unknown')).toBeTruthy();
  await rowFor(tableIds[11]).findByText('Running');
  const claudeRow = within(
    within(table).getByText('Claude chat').closest('[role="row"]')! as HTMLElement,
  );
  await claudeRow.findByLabelText('Claude Code · Running');
  expect(claudeRow.getByText('Running').getAttribute('title')).toBe(
    'Claude Code runtime · Running',
  );
  expect(read).toHaveBeenLastCalledWith(
    [...recentIds.slice(0, 8), tableIds[11], 'claude-chat'].sort(),
    expect.any(String),
  );
  expect(release).toHaveBeenCalledWith(visibleId);
  const beforeFilterId = read.mock.calls.at(-1)![1];
  fireEvent.click(screen.getByRole('switch', { name: 'With PRs only' }));
  await screen.findByText('No sessions match these filters.');
  await waitFor(() =>
    expect(read).toHaveBeenLastCalledWith(recentIds.slice(0, 8).sort(), expect.any(String)),
  );
  expect(release).toHaveBeenCalledWith(beforeFilterId);
  // Recent activity and its full IDs remain outside the All sessions filter.
  expect(recent().getAllByRole('link')).toHaveLength(8);
  const lastId = read.mock.calls.at(-1)![1];
  view.unmount();
  expect(release).toHaveBeenLastCalledWith(lastId);
});

it('keeps mixed recent and paged table states exact, including children and missing parents', async () => {
  vi.stubGlobal('IntersectionObserver', LiveRowsObserver);
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  const hosts = ['claude', 'codex', 'claude', 'codex', 'claude', 'claude'];
  const recentStates = [
    'running',
    'idle',
    'waiting_approval',
    'waiting_input',
    'unknown',
    'unknown',
  ] as const;
  const tableStates = [
    'running',
    'idle',
    'waiting_input',
    'waiting_approval',
    'unknown',
    'unknown',
  ] as const;
  const recentIds = hosts.map((_, index) => `recent-state-${index}`);
  const tableIds = hosts.map((_, index) => `table-state-${index}`);
  report.lanes = recentIds.map((session_id, index) => ({
    host: hosts[index],
    session_id,
    start_ms: report.lane_end_ms - (index + 1) * 1000 - 100,
    end_ms: report.lane_end_ms - (index + 1) * 1000,
  }));
  report.lane_sessions = recentIds.map((session_id, index) => ({
    ...report.lane_sessions[0],
    session_id,
    host: hosts[index],
    title: session_id,
    parent: null,
  }));
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  const page = (fixture as FixtureExport).sessions[0];
  const missingParent = 'absent-claude-parent';
  const rows = tableIds.map((id, index) => ({
    ...page.rows[0],
    id,
    host: hosts[index],
    title: id,
    has_conflict: index >= 4,
    parent:
      index === 2 || index === 4
        ? {
            session_id: index === 2 ? tableIds[0] : missingParent,
            host: 'claude',
            title: index === 2 ? tableIds[0] : missingParent,
            evidence: 'native_spawn' as const,
          }
        : null,
  }));
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (_filter, cursor) => ({
    ...page,
    rows: cursor ? rows.slice(2) : rows.slice(0, 2),
    next: cursor ? null : 'second-page',
    // As the native page does: the context of the parent a row names that no
    // loaded page lists, an ordinary checked main session.
    referenced_parents: cursor
      ? [
          {
            session_id: missingParent,
            host: 'claude',
            known_child: false,
            parent: null,
            child_check: 'checked' as const,
          },
        ]
      : [],
  }));
  const states = new Map([
    ...recentIds.map((id, index) => [id, recentStates[index]] as const),
    ...tableIds.map((id, index) => [id, tableStates[index]] as const),
  ]);
  let issued = 0;
  const read = vi.fn<NonNullable<DataSource['liveSessions']>['read']>(async (ids, token) => ({
    view_id: token ?? `mixed-paging-${++issued}`,
    states: ids.map((id) => ({ id, status: states.get(id) ?? 'unknown' })),
  }));
  const release = vi.fn(async () => {});
  Object.assign(source, { liveSessions: { read, release } });
  const view = render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  const table = await screen.findByRole('table', { name: 'Indexed sessions' });
  const rowFor = (id: string) =>
    within(
      within(table)
        .getByRole('link', {
          name: `Open session ${id}, ${id}`,
        })
        .closest('[role="row"]')! as HTMLElement,
    );
  for (const [index, text] of [
    'Running',
    'Idle',
    'Waiting for approval',
    'Waiting for input',
    'Unknown',
    'Unknown',
  ].entries()) {
    const item = (
      await recent().findByRole('link', { name: new RegExp(`${recentIds[index]}$`) })
    ).closest('li')!;
    const badge = await within(item).findByLabelText(
      `${hosts[index] === 'claude' ? 'Claude Code' : 'Codex'} · ${text}`,
    );
    expect(badge.getAttribute('data-live-status')).toBe(recentStates[index]);
  }
  expect(read).toHaveBeenLastCalledWith([...recentIds].sort(), expect.any(String));
  LiveRowsObserver.current.show(tableIds.slice(0, 2));
  await rowFor(tableIds[0]).findByLabelText('Claude Code · Running');
  await rowFor(tableIds[1]).findByLabelText('Codex · Idle');
  const firstToken = read.mock.calls.at(-1)![1];
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  // Each child is collapsed under its group, the loaded parent's or the one
  // naming a parent that is not loaded, and is not read until opened.
  const loadedGroup = await within(table).findByRole('button', {
    name: `1 loaded sub-session of ${tableIds[0]}`,
  });
  const absentGroup = within(table).getByRole('button', {
    name: `1 loaded sub-session of ${missingParent}`,
  });
  expect(within(table).queryByRole('link', { name: new RegExp(tableIds[2]) })).toBeNull();
  expect(within(table).queryByRole('link', { name: new RegExp(tableIds[4]) })).toBeNull();
  LiveRowsObserver.current.show(tableIds);
  fireEvent.click(loadedGroup);
  fireEvent.click(absentGroup);
  await within(table).findByRole('link', { name: `Open session ${tableIds[4]}, ${tableIds[4]}` });
  expect(list).toHaveBeenLastCalledWith(all, 'second-page', 7);
  expect(rowFor(tableIds[2]).getByLabelText('Claude Code · Unknown')).toBeTruthy();
  expect(read.mock.calls.every(([ids]) => !ids.includes(tableIds[2]))).toBe(true);
  LiveRowsObserver.current.show(tableIds);
  await rowFor(tableIds[2]).findByLabelText('Claude Code · Waiting for input');
  await rowFor(tableIds[3]).findByLabelText('Codex · Waiting for approval');
  expect(rowFor(tableIds[4]).getByLabelText('Claude Code · Unknown')).toBeTruthy();
  expect(rowFor(tableIds[5]).getByLabelText('Claude Code · Unknown')).toBeTruthy();
  for (const id of tableIds.slice(4)) {
    expect(rowFor(id).getByText(/source conflict/)).toBeTruthy();
    expect(rowFor(id).queryByLabelText('Claude Code · Running')).toBeNull();
  }
  expect(rowFor(tableIds[2]).queryByLabelText('Claude Code · Running')).toBeNull();
  expect(read).toHaveBeenLastCalledWith([...recentIds, ...tableIds].sort(), expect.any(String));
  expect(release).toHaveBeenCalledWith(firstToken);
  const parent = rowFor(tableIds[4]).getByRole('link', { name: new RegExp(missingParent) });
  expect(parent.querySelector('[data-live-status]')).toBeNull();
  expect(read.mock.calls.every(([ids]) => !ids.includes(missingParent))).toBe(true);
  expect(within(table).getAllByRole('link', { name: /^Open session / })).toHaveLength(6);
  const lastToken = read.mock.calls.at(-1)![1];
  view.unmount();
  expect(release).toHaveBeenCalledWith(lastToken);
});

it('shows no live badges for a fixture without the capability', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  report.lanes = [
    { ...report.lanes[0], host: 'claude', session_id: 'fixture-claude' },
    { ...report.lanes[0], host: 'codex', session_id: 'fixture-codex' },
  ];
  report.lane_sessions = report.lanes.map((lane) => ({
    ...report.lane_sessions[0],
    host: lane.host,
    session_id: lane.session_id,
  }));
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Recent indexed activity' });
  await recent().findAllByRole('link');
  expect(document.querySelector('[data-live-status]')).toBeNull();
});

it.each([
  { phase: 'registration', host: 'claude' },
  { phase: 'selected read', host: 'claude' },
  { phase: 'registration', host: 'codex' },
  { phase: 'selected read', host: 'codex' },
])(
  'releases a late $host $phase on Sessions navigation without restoring Running',
  async ({ phase, host }) => {
    const source = new FixtureDataSource(fixture as FixtureExport);
    const report = f1Report(7);
    const id = 'exact-navigation-chat';
    report.lanes = [
      {
        host,
        session_id: id,
        start_ms: report.lane_end_ms - 1000,
        end_ms: report.lane_end_ms,
      },
    ];
    report.lane_sessions = [{ ...report.lane_sessions[0], host, session_id: id, title: id }];
    vi.spyOn(source, 'dashboard').mockResolvedValue(report);
    const viewId = 'native-navigation-lease';
    let answer!: (value: LiveSessionSnapshot) => void;
    const late = new Promise<LiveSessionSnapshot>((resolve) => {
      answer = resolve;
    });
    const read = vi.fn<NonNullable<DataSource['liveSessions']>['read']>(async (_ids, token) =>
      token === null && phase === 'selected read' ? { view_id: viewId, states: [] } : late,
    );
    const release = vi.fn<NonNullable<DataSource['liveSessions']>['release']>(async () => {});
    Object.assign(source, { liveSessions: { read, release } });
    render(
      <MemoryRouter initialEntries={['/sessions']}>
        <DataProvider source={source}>
          <Link to="/elsewhere">Leave Sessions</Link>
          <Routes>
            <Route path="/sessions" element={<SessionsPage />} />
            <Route path="/elsewhere" element={<p>Other page</p>} />
          </Routes>
        </DataProvider>
      </MemoryRouter>,
    );
    await screen.findByRole('region', { name: 'Recent indexed activity' });
    const reads = phase === 'registration' ? 1 : 2;
    await waitFor(() => expect(read).toHaveBeenCalledTimes(reads));
    expect(read.mock.calls[0]).toEqual([[], null]);
    fireEvent.click(screen.getByRole('link', { name: 'Leave Sessions' }));
    await screen.findByText('Other page');
    if (phase === 'registration') expect(release).not.toHaveBeenCalled();
    else expect(release).toHaveBeenCalledWith(viewId);
    await act(async () => answer({ view_id: viewId, states: [{ id, status: 'running' }] }));
    expect(document.querySelector('[data-live-status]')).toBeNull();
    expect(release).toHaveBeenCalledWith(viewId);
    expect(release.mock.calls.every(([released]) => released === viewId)).toBe(true);
    expect(read).toHaveBeenCalledTimes(reads);
  },
);

it('puts an old-start Codex chat with recent activity above the first All sessions page', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  const base = report.lane_sessions[0];
  const old = '01a0aaaa-0000-7000-8000-000000000001';
  const child = '01a0bbbb-0000-7000-8000-000000000002';
  const now = report.lane_end_ms;
  report.lanes = [
    { host: 'codex', session_id: child, start_ms: now - 4000, end_ms: now - 3000 },
    { host: 'codex', session_id: old, start_ms: now - 9000, end_ms: now - 8000 },
    { host: 'claude', session_id: base.session_id, start_ms: now - 7000, end_ms: now - 6000 },
    { host: 'codex', session_id: old, start_ms: now - 2000, end_ms: now - 1000 },
  ];
  // Context is ordered by ID, independently of the spans. The child is
  // listed under the parent its context names, collapsed.
  report.lane_sessions = [
    {
      ...base,
      host: 'codex',
      session_id: old,
      title: 'Old-start chat',
      started_at_ms: now - 9 * 86_400_000,
    },
    {
      ...base,
      host: 'codex',
      session_id: child,
      title: 'Child chat',
      parent: {
        host: 'codex',
        session_id: old,
        evidence: 'native_spawn',
        title: null,
      },
    },
    base,
  ];
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  const page = (fixture as FixtureExport).sessions[0];
  const list = vi.spyOn(source, 'sessionsList').mockResolvedValue({
    window: page.window,
    rows: page.rows,
    next: 'another-page',
  });
  let issued = 0;
  const read = vi.fn<NonNullable<DataSource['liveSessions']>['read']>(async (ids, viewId) => ({
    view_id: viewId ?? `native-old-start-${++issued}`,
    states: ids.map((id) => ({
      id,
      status: id === old ? ('running' as const) : ('idle' as const),
    })),
  }));
  Object.assign(source, { liveSessions: { read, release: vi.fn(async () => {}) } });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Recent indexed activity' });
  const oldLink = await recent().findByRole('link', {
    name: `Open recent Codex session Old-start chat, ${old}`,
  });
  const links = recent().getAllByRole('link');
  // Two spans of the old chat become one session; its child is collapsed.
  expect(links).toHaveLength(2);
  expect(links[0]).toBe(oldLink); // Newest by recorded end, not native start or span order.
  expect(recent().queryByRole('link', { name: new RegExp(child) })).toBeNull();
  const toggle = recent().getByRole('button', { name: '1 recent sub-session of Old-start chat' });
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(oldLink.textContent).toContain(old);
  expect(oldLink.closest('li')!.querySelector('time')!.dateTime).toBe(
    new Date(now - 1000).toISOString(),
  );
  const table = within(screen.getByRole('table', { name: 'Indexed sessions' }));
  expect(table.queryByRole('link', { name: new RegExp(old) })).toBeNull();
  expect(list).toHaveBeenCalledWith(all, null, 7);
  expect(
    recent().getByText(
      'Recorded activity is history; Claude Code and Codex badges show their runtime’s live state.',
    ),
  ).toBeTruthy();
  expect(await within(oldLink.closest('li')!).findByText('Running')).toBeTruthy();
  expect(within(oldLink.closest('li')!).getByText('Running').getAttribute('title')).toBe(
    'Codex desktop runtime · Running',
  );
  expect(read.mock.calls[0]).toEqual([[], null]);
  // A collapsed sub-session's live state is not read.
  expect(read.mock.calls[1]).toEqual([[old, base.session_id].sort(), 'native-old-start-1']);
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  const childLink = recent().getByRole('link', {
    name: `Open recent Codex session Child chat, ${child}`,
  });
  // Listed under its parent, before the older ordinary session.
  expect(recent().getAllByRole('link')).toEqual([oldLink, childLink, expect.anything()]);
  expect(childLink.closest('li')!.dataset.depth).toBe('1');
  await waitFor(() =>
    expect(read.mock.calls.at(-1)![0]).toEqual([old, child, base.session_id].sort()),
  );
  expect(recent().getByLabelText('Claude Code · Idle')).toBeTruthy();
  expect(recent().getByText('Search and filters affect All sessions only.')).toBeTruthy();
  expect(
    recent()
      .getByRole('link', { name: new RegExp(old) })
      .compareDocumentPosition(screen.getByText('All sessions')) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});

it('shows a host title for the eight newest sessions and keeps each full ID visible', async () => {
  const ids = Array.from({ length: 9 }, (_, index) => `codex-chat-${index + 1}`);
  const longTitle = 'Host-named chat with a long title that will be clipped in the recent list';
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  const base = report.lane_sessions[0];
  report.lanes = ids.map((id, index) => ({
    host: 'codex',
    session_id: id,
    start_ms: report.lane_end_ms - (index + 1) * 1000 - 100,
    end_ms: report.lane_end_ms - (index + 1) * 1000,
  }));
  report.lane_sessions = ids.map((id) => ({
    ...base,
    host: 'codex',
    session_id: id,
    title: null,
    repo: '/Users/example/code/atlas',
    branch: 'feat/long-title',
  }));
  report.lanes_total = report.lanes.length;
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  const page = (fixture as FixtureExport).sessions[0];
  const loaded = Array.from({ length: 50 }, (_, index) => ({
    ...page.rows[0],
    id: index === 0 ? ids[0] : `loaded-${index}`,
  }));
  vi.spyOn(source, 'sessionsList').mockResolvedValue({
    window: page.window,
    rows: loaded,
    next: null,
  });
  const read = vi.fn(async (requested: readonly string[]) => ({
    titles: requested.includes(ids[0]) ? [{ id: ids[0], title: longTitle }] : [],
  }));
  const data: DataSource = Object.assign(source, {
    titles: { read, cancel: async () => {} },
  });
  render(
    <MemoryRouter>
      <DataProvider source={data}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Recent indexed activity' });
  const named = await recent().findByRole('link', {
    name: `Open recent Codex session ${longTitle}, ${ids[0]}`,
  });
  expect(named.getAttribute('title')).toBe(
    `${longTitle} · ${ids[0]} · /Users/example/code/atlas · feat/long-title`,
  );
  expect(named.querySelector('.xt-sessions-recent-id')?.textContent).toBe(ids[0]);
  expect(recent().getAllByRole('link')).toHaveLength(8);
  expect(recent().queryByText(ids[8])).toBeNull();
  expect(
    recent().getByText(
      '8 most recent main sessions and groups with activity in the last 48 hours; opening a group also lists its sub-sessions',
    ),
  ).toBeTruthy();
  await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(read.mock.calls[0][0]).toEqual(loaded.map((row) => row.id));
  expect(read.mock.calls[1][0]).toEqual(ids.slice(1, 8));
  expect(
    read.mock.calls.flatMap(([requested]) => requested).filter((id) => id === ids[0]),
  ).toHaveLength(1);
  expect(read.mock.calls.every(([requested]) => !requested.includes(ids[8]))).toBe(true);
});

it('keeps a warmed dashboard report hidden when opening an unsupported pull-request view', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const dashboard = vi.spyOn(source, 'dashboard');
  vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => ({
    ...(fixture as FixtureExport).native_index,
    phase: { phase: 'scanning' },
    hosts: [],
  }));
  const read = vi.fn(async () => ({ titles: [] }));
  const data: DataSource = Object.assign(source, {
    titles: { read, cancel: async () => {} },
  });
  render(
    <MemoryRouter initialEntries={['/sessions']}>
      <DataProvider source={data}>
        <Link to="/sessions?pr=example">Open unsupported view</Link>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Recent indexed activity' });
  await recent().findByRole('link');
  await waitFor(() => expect(read).toHaveBeenCalled());
  const dashboardReads = dashboard.mock.calls.length;
  const titleReads = read.mock.calls.length;
  fireEvent.click(screen.getByRole('link', { name: 'Open unsupported view' }));
  await screen.findByText(/Pull request filtering is not available/);
  await act(async () => {});
  expect(screen.queryByRole('region', { name: 'Recent indexed activity' })).toBeNull();
  expect(dashboard).toHaveBeenCalledTimes(dashboardReads);
  expect(read).toHaveBeenCalledTimes(titleReads);
});

it('opens the exact recent session and preserves the list address despite its filters', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  const id = 'codex/chat?with=parts';
  report.lanes = [
    {
      host: 'codex',
      session_id: id,
      start_ms: report.lane_end_ms - 5000,
      end_ms: report.lane_end_ms - 1000,
    },
  ];
  report.lane_sessions = [
    { ...report.lane_sessions[0], host: 'codex', session_id: id, title: 'Filtered out chat' },
  ];
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter initialEntries={['/sessions?q=absent&host=claude&with_prs=1&range=30d']}>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Recent indexed activity' });
  const link = await recent().findByRole('link', {
    name: `Open recent Codex session Filtered out chat, ${id}`,
  });
  expect(link.getAttribute('href')).toBe(
    '/sessions/codex%2Fchat%3Fwith%3Dparts?q=absent&host=claude&with_prs=1&range=30d',
  );
});

it('warns when recent spans are truncated and hides retained rows after a failed refresh', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  report.lanes_truncated = true;
  report.lanes_total = report.lanes.length + 10;
  const dashboard = vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Recent indexed activity' });
  await recent().findByRole('link');
  expect(recent().getByText(/earlier activity can be missing/)).toBeTruthy();
  dashboard.mockRejectedValue(new Error('dashboard unavailable'));
  act(() => source.emit(events.importReceived));
  await recent().findByText('Recent indexed activity could not be loaded.');
  expect(recent().queryByRole('link')).toBeNull();
  expect(recent().queryByText(/earlier activity can be missing/)).toBeNull();
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
});

const mountRoutes = (source: DataSource, entry: string) =>
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[entry]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );

it('summarises the whole range above the list, independently of its filters', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const dashboard = vi.spyOn(source, 'dashboard');
  const list = vi.spyOn(source, 'sessionsList');
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  // The heading's one visible line says what the summary covers, and the
  // whole sentence is the summary region's own description.
  const heading = document.querySelector('.xt-sessions-heading')!;
  expect(heading.querySelector('p')!.textContent).toBe(
    'Range: all indexed activity · filters: table only',
  );
  const caption = screen.getByText(
    'The summary counts all indexed activity in the selected range; filters narrow only the table.',
  );
  expect(heading.contains(caption)).toBe(true);
  expect(caption.className).toBe('sr-only');
  expect(
    screen.getByRole('region', { name: 'Range summary' }).getAttribute('aria-describedby'),
  ).toBe(caption.id);
  await waitFor(() => expect(tile('Human messages').textContent).toContain('5'));
  // M-04 measures output on its own, so the tile reports F1's 150 output
  // tokens rather than the 1,100-token range total the list's column sums.
  expect(tile('Output tokens').textContent).toContain('150');
  // M-05 reports 0.383 hours; the tile states that same span in minutes.
  expect(tile('Agent minutes').textContent).toContain('23');
  // M-16's own mean, and the busiest of the very day buckets it divides by.
  expect(tile('Sessions / day').textContent).toContain('0.1');
  expect(tile('Sessions / day').textContent).toContain('max 1');
  // The first settled index status invalidates the metric queries once, as the
  // `ready` event it stands in for would have; let that land before counting.
  await waitFor(() => expect(dashboard).toHaveBeenCalledTimes(2));
  const reads = dashboard.mock.calls.length;
  // Filtering reaches the list's query alone: the report is not read again,
  // never narrowed by a filter, and the tiles keep reporting the whole range
  // rather than what the table has been narrowed to.
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'not-present' } });
  await screen.findByText('No sessions match these filters.');
  await onlyHost('Codex');
  fireEvent.click(screen.getByRole('switch', { name: 'With PRs only' }));
  await waitFor(() =>
    expect(list).toHaveBeenLastCalledWith(
      { search: 'not-present', hosts: ['codex'], withPrs: true },
      null,
      7,
    ),
  );
  expect(dashboard).toHaveBeenCalledTimes(reads);
  expect(dashboard.mock.calls).toEqual(dashboard.mock.calls.map(() => [7]));
  expect(tile('Human messages').textContent).toContain('5');
  expect(tile('Agent minutes').textContent).toContain('23');
  expect(tile('Output tokens').textContent).toContain('150');
});

it('shows measured output when the range total is not, and a zero that was measured', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  // M-04: an incomplete selected usage observation leaves the total unmeasured
  // while independently measured output is still shown.
  report.tokens.counters.total_tokens = null;
  report.tokens.counters.output_tokens = 4200;
  report.tiles.human_messages.value = 0;
  report.tiles.agent_hours.value = null;
  report.tiles.agent_hours.reason = 'Active spans are unmeasured';
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  await waitFor(() => expect(tile('Output tokens').textContent).toContain('4.2K'));
  // A measured zero is reported as zero, never as an unknown.
  expect(tile('Human messages').textContent).toContain('0');
  expect(within(tile('Human messages')).queryByText(/^Unmeasured/)).toBeNull();
  // An unmeasured hour count stays unmeasured: no zero is invented by the
  // conversion, and the report's own reason (one this app has no plain
  // wording for) is what the tile gives, as a sentence.
  expect(
    within(tile('Agent minutes')).getByText('Unmeasured: Active spans are unmeasured.'),
  ).toBeTruthy();
  expect(tile('Agent minutes').textContent).not.toContain('0');
});

it('drops the busiest day when the report carries no buckets', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  report.tokens.counters.output_tokens = null;
  report.days = [];
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  await waitFor(() =>
    expect(
      within(tile('Output tokens')).getByText(
        'Unmeasured: Output token counts are missing or incomplete',
      ),
    ).toBeTruthy(),
  );
  // Without day buckets there is no busiest one; none is implied from the mean.
  expect(tile('Sessions / day').textContent).not.toContain('max');
});

it('reads the summary again for the range the shell selects', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const dashboard = vi.spyOn(source, 'dashboard').mockImplementation(async (days) => {
    const report = f1Report(days);
    // Distinct per range, so the tiles are seen to follow the selection.
    report.tiles.human_messages.value = days;
    return report;
  });
  mountRoutes(source, '/sessions?range=7d');
  await screen.findByText('Session 00000000');
  await waitFor(() => expect(tile('Human messages').textContent).toContain('7'));
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(tile('Human messages').textContent).toContain('14'));
  expect(dashboard).toHaveBeenLastCalledWith(14);
});

it('keeps the session list working when the range summary cannot be read', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let failing = true;
  vi.spyOn(source, 'dashboard').mockImplementation(async (days) => {
    if (failing) throw new Error('backend-specific detail');
    return f1Report(days);
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  // The list itself is unaffected: its rows load and stay readable.
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  const table = within(screen.getByRole('table', { name: 'Indexed sessions' }));
  expect(table.getByText('150')).toBeTruthy();
  await screen.findByText(/the session list below is unaffected/);
  expect(screen.queryByText(/backend-specific detail/)).toBeNull();
  // No tile stands a zero in for a metric that was never read.
  for (const label of ['Human messages', 'Output tokens', 'Agent minutes', 'Sessions / day']) {
    expect(
      within(tile(label)).getByText('Unmeasured: Range metrics could not be loaded'),
    ).toBeTruthy();
  }
  failing = false;
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(tile('Human messages').textContent).toContain('5'));
  expect(screen.queryByText(/the session list below is unaffected/)).toBeNull();
});

it('never stands a zero in for a summary it has not finished reading', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  vi.spyOn(source, 'dashboard').mockImplementation(() => new Promise(() => {}));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  for (const label of ['Human messages', 'Output tokens', 'Agent minutes', 'Sessions / day']) {
    expect(
      within(tile(label)).getByText('Unmeasured: Range metrics are still being read'),
    ).toBeTruthy();
  }
});

// The report's one-decimal scale prints a small positive as "0", which reads
// as "nothing happened". A positive below the scale, a measured zero and an
// unknown must each stay distinguishable from the other two.
it.each([
  ['a positive below the shown scale', 1 / 30, '<0.1'],
  ['a measured zero', 0, '0'],
  ['a value the scale can show', 0.5, '0.5'],
] as const)('reports %s as its own value', async (_case, value, shown) => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  report.tiles.sessions_per_day.value = value;
  // Two seconds of active span: 0.033 minutes, and not nothing either.
  report.tiles.agent_hours.value = 2 / 3600;
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  await waitFor(() => expect(within(tile('Sessions / day')).getByText(shown)).toBeTruthy());
  expect(within(tile('Sessions / day')).queryByText(/^Unmeasured/)).toBeNull();
  // Agent minutes reads on the same scale and is stated the same way.
  expect(within(tile('Agent minutes')).getByText('<0.1')).toBeTruthy();
});

it('stops showing values it can no longer confirm when a refresh of a loaded report fails', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let failing = false;
  vi.spyOn(source, 'dashboard').mockImplementation(async (days) => {
    const report = f1Report(days);
    // A change the report did not suppress, so its disappearance is visible.
    report.tiles.human_messages.delta = { previous: 4, pct: 25, suppressed: false };
    if (failing) throw new Error('backend-specific detail');
    return report;
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  await waitFor(() => expect(tile('Human messages').textContent).toContain('5'));
  expect(tile('Human messages').textContent).toContain('▲25%');
  expect(tile('Sessions / day').textContent).toContain('max 1');
  // An ingest event refreshes the report, and that refresh fails.
  failing = true;
  act(() => source.emit(events.importReceived));
  await screen.findByText(/the session list below is unaffected/, undefined, { timeout: 3000 });
  // The held report is no longer confirmed, so nothing from it is shown as
  // though it were: not a value, not a change, not the busiest day.
  for (const label of ['Human messages', 'Output tokens', 'Agent minutes', 'Sessions / day']) {
    expect(
      within(tile(label)).getByText('Unmeasured: Range metrics could not be loaded'),
    ).toBeTruthy();
  }
  expect(tile('Human messages').textContent).not.toContain('5');
  expect(tile('Human messages').textContent).not.toContain('▲25%');
  expect(tile('Sessions / day').textContent).not.toContain('max');
  expect(screen.queryByText(/backend-specific detail/)).toBeNull();
  // The list is untouched by the summary's failure.
  expect(
    within(screen.getByRole('table', { name: 'Indexed sessions' })).getByText('150'),
  ).toBeTruthy();
  failing = false;
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(tile('Human messages').textContent).toContain('▲25%'));
  expect(tile('Sessions / day').textContent).toContain('max 1');
  expect(screen.queryByText(/the session list below is unaffected/)).toBeNull();
});

it('does not read absent output counters as an absence of output', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const report = f1Report(7);
  // M-04: output is null when its counters are absent, and also when a
  // response mixes measured counters with missing ones. The tile cannot tell
  // those apart, so it must not claim there was no output.
  report.tokens.counters.output_tokens = null;
  report.tokens.counters.input_tokens = 750;
  report.tokens.counters.total_tokens = null;
  report.usage_coverage.total = {
    sessions: 4,
    measured: 1,
    pct: 25,
    gaps: ['incomplete_counters'],
  };
  vi.spyOn(source, 'dashboard').mockResolvedValue(report);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  const output = () => tile('Output tokens');
  await waitFor(() =>
    expect(
      within(output()).getByText('Unmeasured: Output token counts are missing or incomplete'),
    ).toBeTruthy(),
  );
  // The coverage count is complete four-counter coverage, not output coverage:
  // the definition names the sessions that lack it, not sessions with output.
  output().focus();
  fireEvent.focus(output());
  const definition = await screen.findByRole('tooltip');
  expect(definition.textContent).toContain('Tokens used by each model response');
  expect(definition.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
  expect(definition.textContent).toContain(
    'Output token counts are missing or incomplete. Total tokens unknown because some counts are missing.',
  );
  expect(definition.textContent).toContain(
    '3 of 4 sessions are missing a token count or model name.',
  );
  expect(definition.textContent).not.toContain('sessions measured');
});

// The summary above the list and the rows in it read on one scale, so the same
// three states stay apart in a row: a span too short for the scale to show, a
// window that ran nothing, and a session that was never indexed.
it('tells a two-second span, an idle window and an unindexed session apart in a row', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  const measured = row.metrics;
  if (measured.state !== 'indexed') throw new Error('F1 exports one indexed session');
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: [
      { ...row, id: 'brief-span', metrics: { ...measured, agent_ms: 2000 } },
      { ...row, id: 'idle-window', metrics: { ...measured, agent_ms: 0 } },
      {
        ...row,
        id: 'not-indexed',
        metrics: { state: 'missing' },
        hands_off: { state: 'missing' },
      },
    ],
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session brief-sp');
  const rowFor = (id: string) =>
    within(screen.getByText(`Session ${id.slice(0, 8)}`).closest('[role="row"]')!);
  // 0.03 minutes is not nothing, and must not borrow the idle window's zero.
  expect(rowFor('brief-span').getByText('<0.1m')).toBeTruthy();
  expect(rowFor('brief-span').getByText('less than 0.1 minutes, exactly 2,000 ms')).toBeTruthy();
  expect(rowFor('brief-span').queryByText('0h00m')).toBeNull();
  // A window that ran nothing measured a zero, and says so.
  expect(rowFor('idle-window').getByText('0h00m')).toBeTruthy();
  expect(rowFor('idle-window').getByText('0 minutes, exactly 0 ms')).toBeTruthy();
  expect(rowFor('idle-window').queryByText('<0.1m')).toBeNull();
  // Nothing was measured for an unindexed session, which is neither of those.
  expect(
    rowFor('not-indexed').getAllByText(/^Unmeasured: This session is not indexed/).length,
  ).toBe(5);
  expect(rowFor('not-indexed').queryByText('0h00m')).toBeNull();
  expect(rowFor('not-indexed').queryByText('<0.1m')).toBeNull();
  expect(rowFor('not-indexed').queryByText(/exactly/)).toBeNull();
});

// The design states agent time as hours and minutes. The column keeps its
// one-decimal minute scale, rounds once and then splits, so nothing it showed
// is lost and no hour reads as sixty minutes; the exact measurement is the
// cell's tooltip, its spoken text and a line in the row's details.
it('states agent time as hours and remaining minutes with the exact measurement', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  const measured = row.metrics;
  if (measured.state !== 'indexed') throw new Error('F1 exports one indexed session');
  const cases = [
    ['long-whole', 192 * 60_000, '3h12m', '3 hours 12 minutes', '11,520,000 ms'],
    ['long-tenth', 11_568_000, '3h12.8m', '3 hours 12.8 minutes', '11,568,000 ms'],
    ['padded', 124 * 60_000, '2h04m', '2 hours 4 minutes', '7,440,000 ms'],
    ['under-hour', 47 * 60_000, '0h47m', '47 minutes', '2,820,000 ms'],
    ['exact-hour', 3_600_000, '1h00m', '1 hour 0 minutes', '3,600,000 ms'],
    ['carry-hour', 3_597_600, '1h00m', '1 hour 0 minutes', '3,597,600 ms'],
    ['sub-minute', 6_000, '0h00.1m', '0.1 minutes', '6,000 ms'],
    ['tiny-span', 1, '<0.1m', 'less than 0.1 minutes', '1 ms'],
    [
      'very-large',
      1_234 * 3_600_000 + 330_000,
      '1,234h05.5m',
      '1,234 hours 5.5 minutes',
      '4,442,730,000 ms',
    ],
  ] as const;
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: cases.map(([id, agent_ms]) => ({ ...row, id, metrics: { ...measured, agent_ms } })),
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session long-who');
  for (const [id, , visible, spoken, exact] of cases) {
    const rowElement = screen.getByText(`Session ${id.slice(0, 8)}`).closest('[role="row"]')!;
    const cell = within(rowElement as HTMLElement);
    const shown = cell.getByText(visible, { selector: '.xt-metric-cell' });
    // Drawn for the eye only; the spoken form carries the same value once.
    expect(shown.closest('[aria-hidden="true"]')).toBeTruthy();
    expect(cell.getByText(`${spoken}, exactly ${exact}`).className).toBe('sr-only');
    expect(shown.closest('[title]')!.getAttribute('title')).toBe(
      `Exactly ${exact} active in this range`,
    );
    expect(rowElement.textContent).not.toMatch(/h60/);
  }
  // Keyboard users reach the exact measurement through the row's details.
  fireEvent.click(screen.getByRole('button', { name: 'Expand long-tenth' }));
  const details = document.querySelector('.xt-session-details')!;
  expect(details.textContent).toContain(
    'Agent time3h12.8m exactly 11,568,000 ms active in this range',
  );
});

it('flags untimed indexed history in the heading, independently of its filters', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = vi.spyOn(source, 'dashboard');
  // F1 indexes nothing without a timestamp, and its index is not warned
  // about, so Sessions draws no triangle at all.
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  await waitFor(() => expect(tile('Human messages').textContent).toContain('5'));
  expect(read).toHaveBeenCalled();
  expect(screen.queryByTestId('sessions-issues')).toBeNull();
  expect(screen.queryByRole('region', { name: 'Untimed indexed history' })).toBeNull();
  cleanup();
  const disclosed = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(disclosed, 'sessionsList');
  vi.spyOn(disclosed, 'dashboard').mockImplementation(async (days) => {
    const report = f1Report(days);
    report.untimed_history = {
      records: 12,
      by_surface: [
        { host: 'claude', surface: 'cli', records: 8 },
        { host: 'cursor', surface: null, records: 4 },
      ],
    };
    return report;
  });
  render(
    <MemoryRouter>
      <DataProvider source={disclosed}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  // One triangle, whose name and gist state the count and its scope; no row
  // of its own stands between the heading and the tiles.
  const flag = await screen.findByRole('button', { name: 'History limits: 12 untimed records' });
  expect(flag.closest('.xt-sessions-heading')).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Untimed indexed history' })).toBeNull();
  expect(screen.queryByTestId('untimed-count')).toBeNull();
  fireEvent.focus(flag);
  expect((await screen.findByRole('tooltip')).textContent).toBe(
    '12 untimed records: outside dated measurements, in all indexed history.',
  );
  fireEvent.blur(flag);
  // Pressing it opens the explanation and the host and surface breakdown.
  const open = async () => {
    fireEvent.click(screen.getByRole('button', { name: 'History limits: 12 untimed records' }));
    return screen.findByRole('dialog', { name: 'Untimed history' });
  };
  let dialog = await open();
  expect(dialog.textContent).toContain(
    'Some indexed history has no timestamps and cannot contribute to date-based measurements.',
  );
  expect(within(dialog).getByTestId('untimed-count').textContent).toContain(
    '12 records in all indexed history',
  );
  expect(
    within(dialog)
      .getAllByRole('listitem')
      .map((item) => item.textContent),
  ).toEqual(['claude · cli8 records', 'cursor · unknown surface4 records']);
  expect(within(dialog).queryByRole('link', { name: 'View indexing status' })).toBeNull();
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(document.activeElement).toBe(flag);
  // The rows below are dated too, so narrowing them cannot change this count.
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'not-present' } });
  await screen.findByText('No sessions match these filters.');
  await onlyHost('Codex');
  await waitFor(() =>
    expect(list).toHaveBeenLastCalledWith(
      { ...all, search: 'not-present', hosts: ['codex'] },
      null,
      7,
    ),
  );
  dialog = await open();
  expect(within(dialog).getByTestId('untimed-count').textContent).toContain(
    '12 records in all indexed history',
  );
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // It qualifies the whole page, so it sits in the heading, outside the range
  // summary, whose tiles do measure the selected range.
  expect(screen.getByRole('region', { name: 'Range summary' }).contains(flag)).toBe(false);
});

it('never counts untimed history the current range report has not confirmed', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let answer: 'pending' | 'failing' | 'counted' = 'pending';
  const pending = new Promise<never>(() => {});
  vi.spyOn(source, 'dashboard').mockImplementation(async (days) => {
    if (answer === 'pending') return pending;
    if (answer === 'failing') throw new Error('backend-specific detail');
    const report = f1Report(days);
    report.untimed_history = {
      records: 8356,
      by_surface: [{ host: 'claude', surface: 'cli', records: 8356 }],
    };
    return report;
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  // While the report is being read, nothing is counted and nothing is flagged.
  expect(await screen.findByText('Session 00000000')).toBeTruthy();
  expect(
    within(tile('Human messages')).getByText('Unmeasured: Range metrics are still being read'),
  ).toBeTruthy();
  expect(screen.queryByTestId('sessions-issues')).toBeNull();
  cleanup();

  answer = 'counted';
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  const flag = await screen.findByRole('button', {
    name: 'History limits: 8,356 untimed records',
  });
  // A refresh of that report fails: the held count is no longer confirmed, so
  // it is not shown as current, and with no index warning the flag goes.
  answer = 'failing';
  act(() => source.emit(events.importReceived));
  await screen.findByText(/the session list below is unaffected/, undefined, { timeout: 3000 });
  await waitFor(() => expect(screen.queryByTestId('sessions-issues')).toBeNull());
  expect(flag.isConnected).toBe(false);
  expect(screen.queryByText(/8,356/)).toBeNull();
  answer = 'counted';
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  expect(
    await screen.findByRole('button', { name: 'History limits: 8,356 untimed records' }),
  ).toBeTruthy();
});

it.each([
  ['beside a known index warning', true],
  ['with no index warning', false],
] as const)(
  'withholds a held untimed count while its report is read again, %s',
  async (_case, warned) => {
    const source = new FixtureDataSource(fixture as FixtureExport);
    if (warned) {
      const status: NativeIndexStatus = {
        ...(fixture as FixtureExport).native_index,
        phase: { phase: 'ready' },
        freshness: { freshness: 'live' },
        hosts: [
          {
            host: 'claude',
            state: 'incomplete',
            detail: null,
            sessions_imported: 1,
            sessions_partial: 1,
            sessions_skipped: 0,
            skipped_conversations: [],
            skipped_conversations_omitted: 0,
            records_new: 25,
            records_enriched: 0,
            diagnostics: 1,
          },
        ],
      };
      vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => status);
    }
    // Answers at once until a read is held; a held read answers when released.
    let records = 8356;
    const held: { release?: () => void } = {};
    let hold = false;
    const read = vi.spyOn(source, 'dashboard').mockImplementation((days) => {
      const report = f1Report(days);
      report.untimed_history = {
        records,
        by_surface: [{ host: 'claude', surface: 'cli', records }],
      };
      if (!hold) return Promise.resolve(report);
      return new Promise((resolve) => {
        held.release = () => resolve(report);
      });
    });
    const name = (count: string) =>
      `History limits: ${warned ? 'History incomplete, ' : ''}${count} untimed records`;
    render(
      <MemoryRouter>
        <DataProvider source={source}>
          <SessionsPage />
        </DataProvider>
      </MemoryRouter>,
    );
    await screen.findByText('Session 00000000');
    // The first settled index status reads the report once more; let that land.
    await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    await screen.findByRole('button', { name: name('8,356') });

    // An ingest refreshes the report, and that read is held: the cache still
    // holds 8,356, but it is no longer confirmed, so no count is stated.
    hold = true;
    records = 9000;
    act(() => source.emit(events.importReceived));
    await waitFor(() => expect(held.release).toBeDefined());
    await waitFor(() => expect(screen.queryByRole('button', { name: /untimed/ })).toBeNull());
    if (warned) {
      // The index warning does not depend on the report, so its triangle stays.
      const flag = screen.getByRole('button', { name: 'History limits: History incomplete' });
      fireEvent.focus(flag);
      expect((await screen.findByRole('tooltip')).textContent).toBe(
        'History incomplete: Some history could not be fully indexed.',
      );
      fireEvent.blur(flag);
    } else expect(screen.queryByTestId('sessions-issues')).toBeNull();

    // The replacement answers, and its count is the one stated.
    act(() => held.release!());
    expect(await screen.findByRole('button', { name: name('9,000') })).toBeTruthy();
    expect(screen.queryByRole('button', { name: /8,356/ })).toBeNull();
  },
);

it('states why a hands-off median is absent in plain words, leaving the rule ID to its header', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  vi.spyOn(source, 'sessionsList').mockImplementation(async () => ({
    window: page.window,
    next: null,
    rows: [
      { ...row, id: 'no-stretch', hands_off: { state: 'measured', n: 0, median_min: null } },
      { ...row, id: 'own-unknown', hands_off: { state: 'unmeasured', excluded_surface: null } },
      {
        ...row,
        id: 'excluded',
        hands_off: {
          state: 'unmeasured',
          excluded_surface: {
            host: 'claude',
            surface: 'raw-batched',
            qualifying_sessions: 4,
            degenerate_sessions: 3,
          },
        },
      },
    ],
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session no-stret');
  /** What a sighted reader sees: no screen-reader-only text, no tooltips. */
  const visible = (element: Element) => {
    const copy = element.cloneNode(true) as Element;
    copy.querySelectorAll('.sr-only').forEach((hidden) => hidden.remove());
    return copy.textContent ?? '';
  };
  for (const [id, reason] of [
    ['no-stretch', 'No hands-off stretch in this window, so there is no median'],
    ['own-unknown', 'A record in this window leaves a stretch boundary or its tool use unknown'],
    ['excluded', 'Excluded: claude raw-batched timestamps are too coarse (3 of 4 sessions)'],
  ] as const) {
    const expand = screen.getByRole('button', { name: `Expand ${id}` });
    fireEvent.click(expand);
    const details = document.getElementById(expand.getAttribute('aria-controls')!)!;
    const handsOff = within(details).getByText('Hands-off').parentElement!;
    expect(visible(handsOff)).toBe(`Hands-off${reason}`);
    expect(visible(details)).not.toMatch(/\bM-\d/);
    fireEvent.click(expand);
  }
  // The definition itself stays one keyboard stop away, on the column header.
  expect(
    screen.getByRole('button', { name: 'Hands-off median minutes, definition M-09' }),
  ).toBeTruthy();
});

it('states an incomplete index and untimed history together in one compact row', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => ({
    ...(fixture as FixtureExport).native_index,
    phase: { phase: 'ready' },
    freshness: { freshness: 'live' },
    hosts: [
      {
        host: 'claude',
        state: 'incomplete',
        detail: null,
        sessions_imported: 1,
        sessions_partial: 1,
        sessions_skipped: 0,
        skipped_conversations: [],
        skipped_conversations_omitted: 0,
        records_new: 25,
        records_enriched: 0,
        diagnostics: 1,
      },
    ],
  }));
  vi.spyOn(source, 'dashboard').mockImplementation(async (days) => {
    const report = f1Report(days);
    report.untimed_history = {
      records: 40,
      by_surface: [{ host: 'claude', surface: 'cli', records: 40 }],
    };
    return report;
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  // One triangle for both, named with the index state and the global untimed
  // count, which is never presented as a share or a filter.
  const flag = await screen.findByRole('button', {
    name: 'History limits: History incomplete, 40 untimed records',
  });
  expect(screen.getAllByRole('button', { name: /^History limits/ })).toHaveLength(1);
  expect(flag.closest('.xt-sessions-filters')!.lastElementChild).toBe(flag);
  // No row of limits stands between the heading and the tiles.
  expect(document.querySelector('.xt-sessions-context')).toBeNull();
  expect(document.querySelector('.xt-sessions-notice')).toBeNull();
  expect(document.querySelector('.xt-sessions details')).toBeNull();
  fireEvent.focus(flag);
  const gist = await screen.findByRole('tooltip');
  expect(gist.textContent).toBe(
    'History incomplete: Some history could not be fully indexed.' +
      '40 untimed records: outside dated measurements, in all indexed history.',
  );
  expect(gist.textContent).not.toMatch(/%/);
  fireEvent.blur(flag);
  fireEvent.click(flag);
  const dialog = await screen.findByRole('dialog', { name: 'Index status and untimed history' });
  const index = within(dialog).getByRole('region', { name: 'History incomplete' });
  expect(index.textContent).toContain('Some history could not be fully indexed.');
  // The Settings route stays one step away, and reachable by keyboard.
  const settings = within(index).getByRole('link', { name: 'View indexing status' });
  expect(settings.getAttribute('href')).toBe('/settings');
  expect(settings.tabIndex).toBe(0);
  const untimed = within(dialog).getByRole('region', { name: 'Untimed history' });
  expect(within(untimed).getByTestId('untimed-count').textContent).toContain(
    '40 records in all indexed history',
  );
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(document.activeElement).toBe(flag);
  // The summary still says what it measures: one short line in view, the
  // whole sentence as its description.
  const summary = screen.getByRole('region', { name: 'Range summary' });
  const scope = document.getElementById(summary.getAttribute('aria-describedby')!)!;
  expect(scope.textContent).toBe(
    'The summary counts all indexed activity in the selected range; filters narrow only the table.',
  );
  expect(scope.closest('.xt-sessions-heading')).toBeTruthy();
  expect(document.querySelector('.xt-sessions-heading p')!.textContent).toBe(
    'Range: all indexed activity · filters: table only',
  );
  expect(summary.contains(flag)).toBe(false);
});

it.each([
  ['scanning over an interrupted, incomplete index', 'scanning', 'Indexing'],
  ['an interrupted, incomplete index', 'degraded', 'Updates interrupted'],
  ['an incomplete index alone', 'incomplete', 'History incomplete'],
] as const)('names only the most serious state for %s', async (_case, worst, state) => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const status: NativeIndexStatus = {
    ...(fixture as FixtureExport).native_index,
    phase: worst === 'scanning' ? { phase: 'scanning' } : { phase: 'ready' },
    freshness:
      worst === 'incomplete'
        ? { freshness: 'live' }
        : { freshness: 'degraded', reason: 'watcher stopped' },
    hosts: [
      {
        host: 'claude',
        state: 'incomplete',
        detail: null,
        sessions_imported: 1,
        sessions_partial: 1,
        sessions_skipped: 0,
        skipped_conversations: [],
        skipped_conversations_omitted: 0,
        records_new: 25,
        records_enriched: 0,
        diagnostics: 1,
      },
    ],
  };
  vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => status);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  expect(await screen.findByRole('button', { name: `History limits: ${state}` })).toBeTruthy();
  expect(screen.getAllByRole('button', { name: /^History limits/ })).toHaveLength(1);
});

it('keeps the index flag on the unsupported pull-request view without reading the range', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = vi.spyOn(source, 'dashboard');
  vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => ({
    ...(fixture as FixtureExport).native_index,
    phase: { phase: 'scanning' },
    freshness: { freshness: 'live' },
    hosts: [],
  }));
  render(
    <MemoryRouter initialEntries={['/sessions?pr=example']}>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText(/Pull request filtering is not available/)).toBeTruthy();
  expect(await screen.findByRole('button', { name: 'History limits: Indexing' })).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Range summary' })).toBeNull();
  expect(read).not.toHaveBeenCalled();
  // With no range summary on the page, the subtitle makes no claim about one.
  const heading = document.querySelector('.xt-sessions-heading')!;
  expect(heading.querySelector('p')!.textContent).toBe('Every indexed session on this Mac');
  expect(heading.textContent).not.toMatch(/summary|Range:/);
});
