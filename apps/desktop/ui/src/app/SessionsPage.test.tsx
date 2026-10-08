import { act, render, screen, fireEvent, waitFor, within, cleanup } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import type { DataSource } from '../data/DataSource';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { NativeHostState } from '../data/generated/NativeHostState';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import type { FixtureExport } from '../data/generated/FixtureExport';
import fixture from '../../fixtures/F1.json';
import { events } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
import { SessionsPage } from './SessionsPage';
import { useNativeIndexStatus } from './useAppInfo';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const all = { search: '', hosts: null, withPrs: false, sort: 'started' as const };
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
      { search: 'not-present', hosts: ['codex'], withPrs: true, sort: 'started' },
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
  expect(rowFor('00000000').getByText('0 h 23 m')).toBeTruthy();
  expect(rowFor('00000000').getByText('23 minutes, exactly 1,380,000 ms')).toBeTruthy();
  expect(rowFor('00000000').getByText('3 min')).toBeTruthy();
  expect(rowFor('00000000').getByText(/median of 5 stretches/)).toBeTruthy();
  // The quiet window reports zeros, never an em dash, for what it measured;
  // no stretch is no median, not a zero-minute one.
  expect(rowFor('quiet-se').getAllByText('0').length).toBe(2);
  expect(rowFor('quiet-se').getByText('0 h 0 m')).toBeTruthy();
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
  ['Agent time', 'M-05'],
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

it('starts at page one on both deliberate sort changes but keeps pages through detail Back', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const base = (fixture as FixtureExport).sessions[0];
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (filter, after) => ({
    ...base,
    next: after ? null : 'second-page',
    rows: Array.from({ length: 50 }, (_, i) => ({
      ...base.rows[0],
      id: after ? `page-two-${i}` : `${filter.sort}-page-one-${i}`,
      title: after ? `Second page ${i}` : `${filter.sort} first page ${i}`,
    })),
  }));
  mountRoutes(source, '/sessions?range=7d');
  await screen.findByRole('link', {
    name: 'Open session started first page 0, started-page-one-0',
  });
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  await screen.findByRole('link', { name: 'Open session Second page 0, page-two-0' });
  expect(screen.getAllByRole('link', { name: /^Open session / })).toHaveLength(100);
  fireEvent.click(screen.getByRole('link', { name: 'Open session Second page 0, page-two-0' }));
  fireEvent.click(await screen.findByRole('link', { name: '← All sessions' }));
  await screen.findByRole('link', { name: 'Open session Second page 0, page-two-0' });
  fireEvent.change(screen.getByRole('combobox', { name: 'Sort sessions' }), {
    target: { value: 'recently_active' },
  });
  await screen.findByRole('link', {
    name: 'Open session recently_active first page 0, recently_active-page-one-0',
  });
  expect(screen.getAllByRole('link', { name: /^Open session / })).toHaveLength(50);
  expect(list).toHaveBeenLastCalledWith({ ...all, sort: 'recently_active' }, null, 7);
  expect(screen.queryByRole('link', { name: 'Open session Second page 0, page-two-0' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  await screen.findByRole('link', { name: 'Open session Second page 0, page-two-0' });
  fireEvent.change(screen.getByRole('combobox', { name: 'Sort sessions' }), {
    target: { value: 'started' },
  });
  await screen.findByRole('link', {
    name: 'Open session started first page 0, started-page-one-0',
  });
  expect(screen.getAllByRole('link', { name: /^Open session / })).toHaveLength(50);
  expect(list).toHaveBeenLastCalledWith(all, null, 7);
  expect(screen.queryByRole('link', { name: 'Open session Second page 0, page-two-0' })).toBeNull();
});

it('reads live states only for visible supported hosts, caps them and releases the lease', async () => {
  let observer!: { show: (ids: string[]) => void };
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      targets = new Set<Element>();
      constructor(private callback: IntersectionObserverCallback) {
        observer = { show: (ids) => this.show(ids) };
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
      show(ids: string[]) {
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
    },
  );
  const exported = structuredClone(fixture as FixtureExport);
  const base = exported.sessions[0].rows[0];
  const rows = Array.from({ length: 20 }, (_, i) => ({
    ...base,
    id: `visible-${i}`,
    host: i === 19 ? 'cursor' : i % 2 ? 'claude' : 'codex',
  }));
  for (const page of exported.sessions) page.rows = rows;
  const source = new FixtureDataSource(exported);
  const read = vi.fn<NonNullable<DataSource['liveSessions']>['read']>(async (ids, token) => ({
    view_id: token ?? 'lease-table',
    states: ids.map((id) => ({ id, status: 'running' })),
  }));
  const release = vi.fn(async () => {});
  Object.assign(source, { liveSessions: { read, release } });
  const view = mountRoutes(source, '/sessions');
  await screen.findByRole('table', { name: 'Indexed sessions' });
  await waitFor(() => expect(observer).toBeTruthy());
  expect(read).not.toHaveBeenCalled();
  observer.show(rows.map((row) => row.id));
  await waitFor(() =>
    expect(read).toHaveBeenLastCalledWith(
      rows
        .slice(0, 16)
        .map((row) => row.id)
        .sort(),
      'lease-table',
    ),
  );
  await waitFor(() => expect(screen.getAllByLabelText(/ · Running$/)).toHaveLength(16));
  observer.show(['visible-18', 'visible-19']);
  await waitFor(() => expect(read).toHaveBeenLastCalledWith(['visible-18'], 'lease-table'));
  expect(release).toHaveBeenCalledWith('lease-table');
  view.unmount();
  expect(release).toHaveBeenCalledWith('lease-table');
  for (const [ids] of read.mock.calls) expect(ids).not.toContain('visible-19');
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

it.each(['initial scan', 'host left unscanned', 'degraded watcher'] as const)(
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
          needs_attention: false,
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
      needs_attention: false,
    };
    let status = structuredClone(complete);
    if (scenario === 'initial scan') {
      status.phase = { phase: 'scanning' };
      status.hosts = [];
    } else if (scenario === 'host left unscanned') {
      // No scan is running, so the app reports the unscanned host as a problem.
      status.hosts[0].state = 'pending';
      status.hosts[0].needs_attention = true;
      status.needs_attention = true;
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
    const [state, sentence] = {
      'initial scan': ['Indexing', /^Indexing: History is still being indexed/],
      'host left unscanned': [
        'History incomplete',
        /^History incomplete: Some history could not be fully indexed/,
      ],
      'degraded watcher': [
        'Updates interrupted',
        /^Updates interrupted: Live indexing is interrupted/,
      ],
    }[scenario] as [string, RegExp];
    const flag = await screen.findByRole('button', { name: `History limits: ${state}` });
    expect(await screen.findByText('Session 00000000')).toBeTruthy();
    fireEvent.focus(flag);
    expect((await screen.findByRole('tooltip')).textContent).toMatch(sentence);
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
      { ...row, id: 'known-3', repo: null, branch: 'main', model: 'model-a', other_models: 2 },
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
  expect(meta('known-4').textContent).toContain('no model recorded');
  // The most-used model Rust chose leads; other models are counted, not named.
  expect(meta('known-3').textContent).toContain('model-a +2 more');
  expect(meta('known-3').textContent).not.toContain('Multiple models');
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

function mountRoutes(source: DataSource, address: string) {
  return render(
    <ThemeProvider>
      <MemoryRouter initialEntries={[address]}>
        <DataProvider source={source}>
          <AppRoutes />
        </DataProvider>
      </MemoryRouter>
    </ThemeProvider>,
  );
}

it('restarts paging for recently active sorting and carries it through session Back', async () => {
  const exported = structuredClone(fixture as FixtureExport);
  const base = exported.sessions[0].rows[0];
  for (const page of exported.sessions)
    page.rows = [
      { ...base, id: 'new-start', title: 'New start', last_activity_at_ms: 1000 },
      { ...base, id: 'old-resumed', title: 'Old resumed', last_activity_at_ms: 9000 },
    ];
  const source = new FixtureDataSource(exported);
  const list = vi.spyOn(source, 'sessionsList');
  const dashboard = vi.spyOn(source, 'dashboard');
  mountRoutes(source, '/sessions?host=claude&range=14d');
  await screen.findByRole('link', { name: 'Open session New start, new-start' });
  expect(screen.queryByRole('region', { name: 'Recent indexed activity' })).toBeNull();
  fireEvent.change(screen.getByRole('combobox', { name: 'Sort sessions' }), {
    target: { value: 'recently_active' },
  });
  await waitFor(() =>
    expect(list).toHaveBeenLastCalledWith(
      { search: '', hosts: ['claude'], withPrs: false, sort: 'recently_active' },
      null,
      14,
    ),
  );
  const links = within(screen.getByRole('table', { name: 'Indexed sessions' })).getAllByRole(
    'link',
    { name: /^Open session/ },
  );
  expect(links.map((link) => link.textContent)).toEqual(['Old resumed', 'New start']);
  expect(links[0].getAttribute('href')).toContain('sort=recently_active');
  const reportReads = dashboard.mock.calls.length;
  fireEvent.click(links[0]);
  const back = await screen.findByRole('link', { name: '← All sessions' });
  expect(back.getAttribute('href')).toContain('sort=recently_active');
  expect(back.getAttribute('href')).toContain('host=claude');
  fireEvent.click(back);
  await screen.findByRole('link', { name: 'Open session Old resumed, old-resumed' });
  expect((screen.getByRole('combobox', { name: 'Sort sessions' }) as HTMLSelectElement).value).toBe(
    'recently_active',
  );
  expect(dashboard.mock.calls.length).toBe(reportReads);
  fireEvent.change(screen.getByRole('combobox', { name: 'Sort sessions' }), {
    target: { value: 'started' },
  });
  await waitFor(() =>
    expect(
      within(screen.getByRole('table', { name: 'Indexed sessions' }))
        .getAllByRole('link', { name: /^Open session/ })
        .map((link) => link.textContent),
    ).toEqual(['New start', 'Old resumed']),
  );
  expect((screen.getByRole('combobox', { name: 'Sort sessions' }) as HTMLSelectElement).value).toBe(
    'started',
  );
});

it('shows Rust totals for all matching indexed sessions and follows search filters', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  expect(document.querySelector('.xt-sessions-heading p')!.textContent).toBe(
    'Matching indexed sessions · messages/time in selected range',
  );
  expect(tile('Sessions').textContent).toContain('1');
  expect(tile('Sessions').textContent).toContain('0 sub · 0 checking');
  expect(tile('Your input').textContent).toContain('5');
  expect(tile('Your input').textContent).toContain('5 / main');
  expect(tile('Agent working time').textContent).toContain('23');
  expect(tile('Agent working time').textContent).toContain('Parallel sessions add together');
  expect(tile('Sessions with PRs').textContent).toContain('1');
  const summary = within(screen.getByRole('region', { name: 'Range summary' }));
  expect(summary.queryByText('Output tokens')).toBeNull();
  expect(summary.queryByText('Sessions / day')).toBeNull();
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'not-present' } });
  await screen.findByText('No sessions match these filters.');
  expect(tile('Sessions').textContent).toContain('0');
  expect(tile('Your input').textContent).toContain('0');
  expect(tile('Your input').textContent).not.toContain('/ main');
  expect(tile('Agent working time').textContent).toContain('0 m');
});

it('renders the supplied ratio without deriving it from loaded rows', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = source.sessionsList.bind(source);
  vi.spyOn(source, 'sessionsList').mockImplementation(async (...args) => ({
    ...(await read(...args)),
    summary: {
      main_sessions: 85,
      sub_sessions: 31,
      checking_sessions: 0,
      unlinked_sub_sessions: 4,
      human_messages: 500,
      messages_per_main_session: 5.9,
      agent_ms: 916_020_000,
      sessions_with_prs: 12,
    },
  }));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  expect(tile('Sessions').textContent).toContain('85');
  expect(tile('Sessions').textContent).toContain('31 sub');
  expect(tile('Your input').textContent).toContain('5.9 / main');
  expect(tile('Agent working time').textContent).toContain('254');
  expect(tile('Sessions with PRs').textContent).toContain('12');
});

it('keeps a missing summary apart from zero and retries without losing loaded rows', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = source.sessionsList.bind(source);
  let unavailable = true;
  vi.spyOn(source, 'sessionsList').mockImplementation(async (...args) => {
    const page = await read(...args);
    return { ...page, summary: unavailable ? null : page.summary };
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  for (const label of ['Sessions', 'Your input', 'Agent working time', 'Sessions with PRs'])
    expect(
      within(tile(label)).getByText('Unmeasured: Summary unavailable for these filters'),
    ).toBeTruthy();
  unavailable = false;
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(tile('Your input').textContent).toContain('5 / main'));
  expect(
    screen.queryByText('Session summary unavailable. Loaded sessions remain below.'),
  ).toBeNull();
});

it('withholds stale summary values after a failed refresh and preserves loaded sessions', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = source.sessionsList.bind(source);
  let failing = false;
  vi.spyOn(source, 'sessionsList').mockImplementation(async (...args) => {
    if (failing) throw new Error('backend-specific detail');
    return read(...args);
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  expect(tile('Your input').textContent).toContain('5 / main');
  failing = true;
  act(() => source.emit(events.importReceived));
  await screen.findByText('Session summary unavailable. Loaded sessions remain below.', undefined, {
    timeout: 3000,
  });
  expect(tile('Your input').textContent).not.toContain('5 / main');
  expect(
    within(screen.getByRole('table', { name: 'Indexed sessions' })).getByText('150'),
  ).toBeTruthy();
  failing = false;
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(tile('Your input').textContent).toContain('5 / main'));
});

it('keeps the first-page totals when loading another page fails and succeeds', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = source.sessionsList.bind(source);
  let failMore = true;
  vi.spyOn(source, 'sessionsList').mockImplementation(async (filter, after, days) => {
    const page = await read(filter, null, days);
    if (after && failMore) throw new Error('backend-specific detail');
    return after
      ? {
          ...page,
          rows: [{ ...page.rows[0], id: 'later-session', title: 'Later session' }],
          summary: null,
          next: null,
        }
      : {
          ...page,
          next: 'more',
          summary: page.summary ? { ...page.summary, main_sessions: 42 } : null,
        };
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  expect(tile('Sessions').textContent).toContain('42');
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  await screen.findByText(/More sessions could not be loaded/);
  expect(tile('Sessions').textContent).toContain('42');
  expect(
    screen.queryByText('Session summary unavailable. Loaded sessions remain below.'),
  ).toBeNull();
  failMore = false;
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await screen.findByText('Later session');
  expect(tile('Sessions').textContent).toContain('42');
  expect(screen.queryByText(/More sessions could not be loaded/)).toBeNull();
});

it('loads the summary for the selected range through the session page read', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = source.sessionsList.bind(source);
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (...args) => {
    const page = await read(...args);
    return { ...page, summary: page.summary ? { ...page.summary, human_messages: args[2] } : null };
  });
  mountRoutes(source, '/sessions?range=7d');
  await waitFor(() => expect(within(tile('Your input')).getByText('7')).toBeTruthy());
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(within(tile('Your input')).getByText('14')).toBeTruthy());
  expect(list).toHaveBeenLastCalledWith(all, null, 14);
});

it('never displays zero while the first summary is still being read', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  vi.spyOn(source, 'sessionsList').mockImplementation(() => new Promise(() => {}));
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByRole('region', { name: 'Range summary' });
  for (const label of ['Sessions', 'Your input', 'Agent working time', 'Sessions with PRs'])
    expect(
      within(tile(label)).getByText('Unmeasured: Session summary is still being read'),
    ).toBeTruthy();
});

it('does not turn incomplete messages into zero or recompute an unavailable ratio', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const read = source.sessionsList.bind(source);
  vi.spyOn(source, 'sessionsList').mockImplementation(async (...args) => {
    const page = await read(...args);
    return {
      ...page,
      summary: page.summary
        ? {
            ...page.summary,
            human_messages: null,
            messages_per_main_session: null,
            checking_sessions: 3,
          }
        : null,
    };
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  await screen.findByText('Session 00000000');
  expect(
    within(tile('Your input')).getByText('Unmeasured: Some messages have not been classified'),
  ).toBeTruthy();
  expect(tile('Your input').textContent).not.toContain('/ main');
  expect(tile('Sessions').textContent).toContain('3 checking');
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
  expect(rowFor('brief-span').getByText('<1 m')).toBeTruthy();
  expect(rowFor('brief-span').getByText('less than 1 minute, exactly 2,000 ms')).toBeTruthy();
  expect(rowFor('brief-span').queryByText('0 h 0 m')).toBeNull();
  // A window that ran nothing measured a zero, and says so.
  expect(rowFor('idle-window').getByText('0 h 0 m')).toBeTruthy();
  expect(rowFor('idle-window').getByText('0 minutes, exactly 0 ms')).toBeTruthy();
  expect(rowFor('idle-window').queryByText('<1 m')).toBeNull();
  // Nothing was measured for an unindexed session, which is neither of those.
  expect(
    rowFor('not-indexed').getAllByText(/^Unmeasured: This session is not indexed/).length,
  ).toBe(5);
  expect(rowFor('not-indexed').queryByText('0 h 0 m')).toBeNull();
  expect(rowFor('not-indexed').queryByText('<1 m')).toBeNull();
  expect(rowFor('not-indexed').queryByText(/exactly/)).toBeNull();
});

// The design states agent time as hours and whole minutes. The column rounds
// once and then splits, so no hour reads as sixty minutes; the exact
// measurement is the cell's tooltip, its spoken text and a line in the row's
// details.
it('states agent time as hours and remaining minutes with the exact measurement', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const page = (fixture as FixtureExport).sessions[0];
  const row = page.rows[0];
  const measured = row.metrics;
  if (measured.state !== 'indexed') throw new Error('F1 exports one indexed session');
  const cases = [
    ['long-whole', 192 * 60_000, '3 h 12 m', '3 hours 12 minutes', '11,520,000 ms'],
    ['long-tenth', 11_568_000, '3 h 13 m', '3 hours 13 minutes', '11,568,000 ms'],
    ['padded', 124 * 60_000, '2 h 4 m', '2 hours 4 minutes', '7,440,000 ms'],
    ['under-hour', 47 * 60_000, '0 h 47 m', '47 minutes', '2,820,000 ms'],
    ['exact-hour', 3_600_000, '1 h 0 m', '1 hour 0 minutes', '3,600,000 ms'],
    ['carry-hour', 3_597_600, '1 h 0 m', '1 hour 0 minutes', '3,597,600 ms'],
    ['sub-minute', 6_000, '<1 m', 'less than 1 minute', '6,000 ms'],
    ['tiny-span', 1, '<1 m', 'less than 1 minute', '1 ms'],
    [
      'very-large',
      1_234 * 3_600_000 + 330_000,
      '1,234 h 6 m',
      '1,234 hours 6 minutes',
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
    expect(rowElement.textContent).not.toMatch(/ 60 m/);
  }
  // Keyboard users reach the exact measurement through the row's details.
  fireEvent.click(screen.getByRole('button', { name: 'Expand long-tenth' }));
  const details = document.querySelector('.xt-session-details')!;
  expect(details.textContent).toContain(
    'Agent time3 h 13 m exactly 11,568,000 ms active in this range',
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
  await waitFor(() => expect(tile('Your input').textContent).toContain('5'));
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
  ).toEqual(['Claude Code · cli8 records', 'Cursor · unknown surface4 records']);
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
  expect(tile('Your input').textContent).toContain('5');
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
  await waitFor(() => expect(screen.queryByTestId('sessions-issues')).toBeNull(), {
    timeout: 3000,
  });
  await waitFor(() => expect(screen.queryByTestId('sessions-issues')).toBeNull());
  expect(flag.isConnected).toBe(false);
  expect(screen.queryByText(/8,356/)).toBeNull();
  answer = 'counted';
  act(() => source.emit(events.importReceived));
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
            needs_attention: true,
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
        needs_attention: true,
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
    ['excluded', 'Excluded: Claude Code · raw-batched timestamps are too coarse (3 of 4 sessions)'],
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
        needs_attention: true,
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
    needs_attention: true,
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
    'Counts cover all matching indexed sessions. Messages and agent working time cover the selected range. The table hides sessions still being checked and sub-sessions without a verified parent.',
  );
  expect(scope.closest('.xt-sessions-heading')).toBeTruthy();
  expect(document.querySelector('.xt-sessions-heading p')!.textContent).toBe(
    'Matching indexed sessions · messages/time in selected range',
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
        needs_attention: true,
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
    needs_attention: true,
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

it.each([
  // The findings' example: Cursor not installed is settled, so no flag.
  ['a host with no local history', 'missing_source', false, null],
  ['a cancelled host scan', 'cancelled', true, 'History incomplete'],
] as const)(
  'flags %s exactly when the app says it needs attention',
  async (_case, state, needsAttention, flag) => {
    const source = new FixtureDataSource(fixture as FixtureExport);
    const host = (name: string, hostState: NativeHostState, attention: boolean) => ({
      host: name,
      state: hostState,
      needs_attention: attention,
      detail: null,
      sessions_imported: 1,
      sessions_partial: 0,
      sessions_skipped: 0,
      skipped_conversations: [],
      skipped_conversations_omitted: 0,
      records_new: 25,
      records_enriched: 0,
      diagnostics: 0,
    });
    vi.spyOn(source, 'nativeIndexStatus').mockImplementation(async () => ({
      ...(fixture as FixtureExport).native_index,
      phase: { phase: 'ready' },
      freshness: { freshness: 'live' },
      hosts: [host('claude', 'complete', false), host('cursor', state, needsAttention)],
      needs_attention: needsAttention,
    }));
    render(
      <MemoryRouter>
        <DataProvider source={source}>
          <SessionsPage />
          <IndexRead />
        </DataProvider>
      </MemoryRouter>,
    );
    await screen.findByText('Session 00000000');
    // The page renders from the same cached status read as this probe, so
    // once the probe shows the read, the page has rendered it too.
    await screen.findByText(`index read: ${needsAttention}`);
    if (flag) expect(screen.getByRole('button', { name: `History limits: ${flag}` })).toBeTruthy();
    else expect(screen.queryByRole('button', { name: /^History limits/ })).toBeNull();
  },
);

/** Shows when the shared index status query has answered, and its overall flag. */
function IndexRead() {
  const index = useNativeIndexStatus();
  return index.data ? <p>{`index read: ${index.data.needs_attention}`}</p> : null;
}

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

it('shows own whole-session cost, keeps partial and unknown honest, and uses the active host arc', async () => {
  let observer!: { show: (ids: string[]) => void };
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      targets = new Set<Element>();
      constructor(private callback: IntersectionObserverCallback) {
        observer = { show: (ids) => this.show(ids) };
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
      show(ids: string[]) {
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
    },
  );
  const exported = structuredClone(fixture) as FixtureExport;
  const base = exported.sessions[0].rows[0];
  const priced = {
    total_usd: 1.25,
    priced_subtotal_usd: 1.25,
    selected_observations: 1,
    priced_observations: 1,
    unpriced_observations: 0,
    assumed_tier_observations: 0,
    unpriced: [],
  };
  const rows = [
    { ...base, id: 'priced', title: 'Fully priced', host: 'codex', cost: priced },
    {
      ...base,
      id: 'partial',
      title: 'Partly priced',
      host: 'claude',
      cost: {
        ...priced,
        total_usd: null,
        selected_observations: 2,
        unpriced_observations: 1,
        unpriced: [
          {
            model: 'new-model',
            service_tier: null,
            observations: 1,
            reason: 'unknown_model' as const,
          },
        ],
      },
    },
    { ...base, id: 'unknown', title: 'Unknown cost', host: 'codex', cost: null },
    { ...base, id: 'idle', title: 'Idle example', host: 'claude', cost: priced },
    { ...base, id: 'unavailable', title: 'Unknown status example', host: 'codex', cost: priced },
  ];
  for (const page of exported.sessions) page.rows = rows;
  const source = new FixtureDataSource(exported);
  Object.assign(source, {
    liveSessions: {
      read: async (ids: string[], token: string) => ({
        view_id: token ?? 'arc-test',
        states: ids.map((id) => ({
          id,
          status: (
            {
              priced: 'running',
              partial: 'waiting_approval',
              unknown: 'waiting_input',
              idle: 'idle',
              unavailable: 'unknown',
            } as const
          )[id as 'priced'],
        })),
      }),
      release: async () => {},
    },
  });
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
  const table = await screen.findByRole('table', { name: 'Indexed sessions' });
  await waitFor(() => expect(observer).toBeTruthy());
  observer.show(rows.map((row) => row.id));
  await waitFor(() => expect(within(table).getByLabelText('Codex · Running')).toBeTruthy());
  expect(within(table).queryByText('Running', { exact: true })).toBeNull();
  expect(table.querySelector('.xt-lane-live-host')).toBeTruthy();
  for (const status of ['Waiting for approval', 'Waiting for input']) {
    expect(within(table).getByText(status, { exact: true })).toBeTruthy();
  }
  expect(table.querySelector('.xt-live-session-badge[data-live-status="idle"]')).toBeNull();
  expect(table.querySelector('.xt-live-session-badge[data-live-status="unknown"]')).toBeNull();
  const headers = within(table).getAllByRole('columnheader');
  const costIndex = headers.findIndex((header) => header.textContent === 'cost');
  expect(costIndex).toBeGreaterThan(-1);
  const costCell = (title: string) =>
    within(within(table).getByText(title).closest('[role="row"]') as HTMLElement).getAllByRole(
      'cell',
    )[costIndex];
  expect(costCell('Fully priced').textContent).toBe('$1.25');
  expect(costCell('Fully priced').querySelector('[title]')?.getAttribute('title')).toContain(
    'public API prices',
  );
  expect(costCell('Partly priced').textContent).toBe('$1.25+');
  expect(costCell('Partly priced').querySelector('[title]')?.getAttribute('title')).toContain(
    'At least',
  );
  expect(costCell('Unknown cost').textContent).toContain('Unmeasured');
  fireEvent.focus(
    screen.getByRole('button', { name: 'Whole-session API-equivalent cost, definition' }),
  );
  expect(await screen.findByText(/Each row shows its own session only/)).toBeTruthy();
});
