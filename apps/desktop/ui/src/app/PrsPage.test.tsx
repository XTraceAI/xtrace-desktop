import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import { MemoryRouter } from 'react-router';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, expect, it, vi } from 'vitest';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import { TauriDataSource } from '../data/TauriDataSource';
import type { PrList } from '../data/generated/PrList';
import { commands } from '../data/ipc-names';
import {
  cells,
  expectLocalReadsOnly,
  exported,
  loaded,
  meta,
  mount,
  names,
  shown,
  table,
  tokensByHost,
  trappedSource,
} from './PrsPage.harness';
import { manyPrRows, prRow } from './prs.synthetic';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockReset();
  localStorage.clear();
});

const PINNED = Date.parse('2026-09-08T00:00:00Z');
const NOT_CACHED = 'no successful refresh has stored it';
const NO_STATE = `—Unmeasured: State not cached: ${NOT_CACHED}, so it is not known to be open, closed or merged`;
const NO_SIZE = `—Unmeasured: Size not cached: ${NOT_CACHED}`;
const never = (number: number) => [
  `Title not cachedocto-org/xtrace-fixture#${number}`,
  NO_STATE,
  NO_SIZE,
  '1',
  'never refreshed',
];

// --- what the page is, and is not ---------------------------------------------

it('keeps the cached inventory a separate lifetime view, with no analytics or range', async () => {
  const source = trappedSource(async () => structuredClone(exported.pull_requests));
  mount(source);
  await loaded('octo-org/xtrace-fixture#11');
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Pull requests');
  expect(screen.queryByText('This view is coming next.')).toBeNull();
  expect(screen.getByRole('heading', { level: 2, name: 'Linked pull requests' })).toBeTruthy();
  expect(screen.getByRole('radio', { name: 'Cached inventory' }).getAttribute('aria-checked')).toBe(
    'true',
  );
  expect(meta()).toBe(
    '3 pull requests indexed · cached facts · lifetime link counts · by repo, then number',
  );
  // The limit is in view without opening anything.
  expect(screen.getByTestId('prs-scope').textContent).toBe(
    'Cached inventory · every linked PR, all time · not the merged-PR report',
  );
  const counted = screen.getByTestId('prs-counted') as HTMLDetailsElement;
  expect(counted.open).toBe(false);
  expect(counted.textContent).toContain('the selected range does not apply');
  expect(counted.textContent).toContain('whatever their evidence (exact, commit or inferred)');
  expect(counted.textContent).toContain('not who wrote it');
  expect(counted.textContent).toContain('Nothing here contacts GitHub');
  // No report figure is shown in this view, and the report is not read.
  expect(document.querySelectorAll('.xt-stat-tile')).toHaveLength(0);
  expect(screen.queryByText(/median/i)).toBeNull();
  expect(screen.queryByRole('switch')).toBeNull();
  // The Shell shows no range for this view, so nothing appears to filter the list.
  expect(screen.queryByRole('radiogroup', { name: 'Date range' })).toBeNull();
  expect(screen.queryByTestId('report-period')).toBeNull();
  expect(source.tokensByHost.mock.calls).toEqual([[7]]);
  // Nothing opens the network or a drilldown, and nothing offers a refresh.
  const links = [...document.querySelectorAll('main a')].map((a) => a.getAttribute('href'));
  expect(links).toEqual(['/dashboard']);
  expect(within(table()).queryAllByRole('link')).toHaveLength(0);
  expect(within(table()).queryAllByRole('button')).toHaveLength(0);
  expect(
    within(document.querySelector('main')!).queryByRole('button', { name: /refresh/i }),
  ).toBeNull();
  expect(source.pullRequests).toHaveBeenCalledOnce();
  expectLocalReadsOnly(source);
});

it('reads nothing again on focus or while idle, and only the list when mounted', async () => {
  vi.useFakeTimers();
  try {
    const source = trappedSource(async () => structuredClone(exported.pull_requests));
    // The initial scan still running: the one status a status reader of the
    // page's own would poll every second. The sidebar's observation does not.
    source.nativeIndexStatus.mockResolvedValue({
      ...structuredClone(exported.native_index),
      phase: { phase: 'scanning' },
    });
    mount(source);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(cells()).toHaveLength(3);
    expect(source.pullRequests).toHaveBeenCalledOnce();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      document.dispatchEvent(new Event('visibilitychange'));
      // Long past the stale time and any plausible interval.
      await vi.advanceTimersByTimeAsync(10 * 60_000);
    });
    expect(source.pullRequests).toHaveBeenCalledOnce();
    expect(source.tokensByHost).toHaveBeenCalledOnce();
    // And the sidebar's status once, transient as it is: nothing polled it.
    expectLocalReadsOnly(source);
  } finally {
    vi.useRealTimers();
  }
});

// --- fixture and native adapters agree in every cache state --------------------

const [initial11, initial12, initial13] = exported.pull_requests.rows;
const [done11, done12, done13] = exported.pull_requests_refreshed.rows;
const refreshed11 = [
  'Synthetic fixture pull request 11octo-org/xtrace-fixture#11 ⑂ fixture/pull-11',
  `merged${shown(PINNED)}`,
  '+33 additions −11 deletions',
  '1',
  `refreshed${shown(PINNED)}`,
];
const failed12 = [
  'Title not cachedocto-org/xtrace-fixture#12',
  NO_STATE,
  NO_SIZE,
  '1',
  `failed · never refreshedrate limited · tried ${shown(PINNED)}`,
];
const refreshed13 = [
  'Synthetic fixture pull request 13octo-org/xtrace-fixture#13 ⑂ fixture/pull-13',
  'open',
  '+39 additions −13 deletions',
  '1',
  `refreshed${shown(PINNED)}`,
];

for (const state of [
  {
    name: 'initial: nothing refreshed',
    refresh: [] as number[],
    stored: [initial11, initial12, initial13],
    expected: [never(11), never(12), never(13)],
  },
  {
    name: 'partial: one pull request selectively refreshed',
    refresh: [1],
    stored: [done11, initial12, initial13],
    expected: [refreshed11, never(12), never(13)],
  },
  {
    name: 'failure: a refresh that failed before any success',
    refresh: [2],
    stored: [initial11, done12, initial13],
    expected: [never(11), failed12, never(13)],
  },
  {
    name: 'complete: every pull request attempted',
    refresh: [1, 2, 3],
    stored: [done11, done12, done13],
    expected: [refreshed11, failed12, refreshed13],
  },
])
  it(`shows the same values from the fixture and the native adapter — ${state.name}`, async () => {
    const before = structuredClone(exported);
    // The fixture adapter, moved into this cache state by a batch run elsewhere
    // (this test's own call), and trapped from then on.
    const preview = new FixtureDataSource(exported);
    if (state.refresh.length > 0) await preview.refreshPullRequests(state.refresh);
    const refresh = vi.spyOn(preview, 'refreshPullRequests');
    const cancel = vi.spyOn(preview, 'cancelPullRequestRefresh');
    const sessions = vi.spyOn(preview, 'sessionsList');
    const transcript = vi.spyOn(preview, 'sessionTranscript');
    const status = vi.spyOn(preview, 'nativeIndexStatus');
    mount(preview);
    await loaded('octo-org/xtrace-fixture#11');
    const fromFixture = cells();
    const fixtureMeta = meta();
    expect(fromFixture).toEqual(state.expected);
    for (const spy of [refresh, cancel, sessions, transcript]) expect(spy).not.toHaveBeenCalled();
    // The Shell's sidebar read the status once; the page read nothing of it.
    expect(status).toHaveBeenCalledOnce();
    // Rendering changed neither the export nor what the adapter serves next.
    expect(exported).toEqual(before);
    expect((await preview.pullRequests()).rows).toEqual(state.stored);
    cleanup();

    // The native adapter, answering `prs_list` with the same stored rows.
    vi.mocked(listen).mockResolvedValue(() => {});
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === commands.appInfo) return exported.app_info;
      if (command === commands.tokensByHost)
        return tokensByHost((args as { windowDays: number }).windowDays);
      if (command === commands.nativeIndexStatus) return structuredClone(exported.native_index);
      if (command === commands.pullRequests) return { rows: structuredClone(state.stored) };
      throw new Error(`${command} is not a command the pull requests page may invoke`);
    });
    mount(new TauriDataSource());
    await loaded('octo-org/xtrace-fixture#11');
    expect(cells()).toEqual(fromFixture);
    expect(meta()).toBe(fixtureMeta);
    // Local reads only, at the IPC boundary: never `prs_refresh`, its cancel,
    // or any session, transcript, Dashboard or counts command. The one index
    // command is the Shell's sidebar reading the shared status, once, beside
    // this page's one `prs_list`.
    expect(new Set(vi.mocked(invoke).mock.calls.map(([command]) => command))).toEqual(
      new Set([
        commands.accountUsage,
        commands.appInfo,
        commands.tokensByHost,
        commands.nativeIndexStatus,
        commands.pullRequests,
      ]),
    );
    expect(
      vi.mocked(invoke).mock.calls.filter(([command]) => command === commands.pullRequests),
    ).toEqual([[commands.pullRequests]]);
    expect(
      vi.mocked(invoke).mock.calls.filter(([command]) => command === commands.nativeIndexStatus),
    ).toEqual([[commands.nativeIndexStatus]]);
  });

// --- null is not zero, open or unmerged; stale facts stay beside a failure ------

it('keeps a cached zero apart from an unknown, and an unknown state apart from open or unmerged', async () => {
  const at = Date.parse('2026-03-04T05:06:00Z');
  const source = trappedSource(async () => ({
    rows: [
      // Refreshed with measured zeros: an empty change is a fact, not a gap.
      prRow(1, {
        title: 'chore: empty change',
        state: 'open',
        additions: 0,
        deletions: 0,
        linked_sessions: 0,
        refreshed_at_ms: at,
        last_attempted_at_ms: at,
        status: { status: 'refreshed' },
      }),
      // Never refreshed: nothing is known, and nothing is guessed.
      prRow(2, { linked_sessions: 1204 }),
      // Closed without merging, with one side of the size missing.
      prRow(3, {
        title: 'fix: abandoned',
        state: 'closed',
        additions: 5,
        deletions: null,
        refreshed_at_ms: at,
        last_attempted_at_ms: at,
        status: { status: 'refreshed' },
      }),
      // Merged, but the merge time was never stored.
      prRow(4, {
        title: 'feat: merged',
        state: 'merged',
        merged_at: null,
        refreshed_at_ms: at,
        last_attempted_at_ms: at,
        status: { status: 'refreshed' },
      }),
      // A merge time with no cached state never makes a row merged.
      prRow(5, { merged_at: '2026-03-01T00:00:00Z' }),
    ],
  }));
  mount(source);
  await loaded('chore: empty change');
  const [zero, unknown, closed, merged, timeOnly] = cells();
  expect(zero).toEqual([
    'chore: empty changextrace/app#101',
    'open',
    '+0 additions −0 deletions',
    '0',
    `refreshed${shown(at)}`,
  ]);
  expect(unknown).toEqual([
    'Title not cachedxtrace/app#102',
    NO_STATE,
    NO_SIZE,
    '1,204',
    'never refreshed',
  ]);
  expect(closed![1]).toBe('closed');
  expect(closed![2]).toBe('+5 additions −— deletions not cached');
  expect(merged![1]).toBe('mergedmerge time not cached');
  expect(timeOnly![1]).toBe(NO_STATE);
  // Nothing unknown is drawn as a state word, and the unknown size is no number.
  const body = within(table()).getAllByRole('row').slice(1);
  expect(body[1]!.querySelector('.xt-pr-state-word')).toBeNull();
  expect(body[4]!.querySelector('.xt-pr-state-word')).toBeNull();
  expect(body[1]!.querySelector('.xt-pr-size')).toBeNull();
  expect(body[1]!.querySelector('time')).toBeNull();
  expectLocalReadsOnly(source);
});

it('keeps lockfile-sized counts exact, never rounded, with the whole value one hover away', async () => {
  const at = Date.parse('2026-03-04T05:06:00Z');
  const refreshed = {
    refreshed_at_ms: at,
    last_attempted_at_ms: at,
    status: { status: 'refreshed' },
  } as const;
  const source = trappedSource(async () => ({
    rows: [
      prRow(1, {
        title: 'chore: regenerate lockfile',
        state: 'open',
        additions: 1_234_567,
        deletions: 987_654,
        ...refreshed,
      }),
      prRow(2, {
        title: 'chore: vendor',
        state: 'open',
        additions: 12_345,
        deletions: 67_890,
        ...refreshed,
      }),
      prRow(3, {
        title: 'fix: one side',
        state: 'open',
        additions: null,
        deletions: 67_890,
        ...refreshed,
      }),
    ],
  }));
  mount(source);
  await loaded('chore: regenerate lockfile');
  const sizes = [...table().querySelectorAll<HTMLElement>('.xt-pr-size')];
  expect(sizes.map((size) => size.textContent)).toEqual([
    '+1,234,567 additions −987,654 deletions',
    '+12,345 additions −67,890 deletions',
    '+— additions not cached −67,890 deletions',
  ]);
  // Not compacted to 1.2M or 12.3K: the stored integers, grouped for reading.
  expect(table().textContent).not.toMatch(/[0-9]\.[0-9]+[KM]/);
  expect(sizes.map((size) => size.title)).toEqual([
    '+1,234,567 additions · −987,654 deletions',
    '+12,345 additions · −67,890 deletions',
    'additions not cached · −67,890 deletions',
  ]);
  // One line per side, so the column's width bounds a line and never a pair.
  for (const size of sizes)
    expect([...size.children].map((line) => line.getAttribute('data-kind'))).toEqual([
      'additions',
      'deletions',
    ]);
  expectLocalReadsOnly(source);
});

it('states all four refresh statuses, keeping earlier facts beside a later failure', async () => {
  const facts = Date.parse('2026-09-06T00:00:00Z');
  const tried = Date.parse('2026-09-07T00:00:00Z');
  const source = trappedSource(async () => ({
    rows: [
      prRow(1),
      prRow(2, {
        title: 'docs: fresh',
        state: 'open',
        additions: 2,
        deletions: 1,
        head_ref_name: 'docs/fresh',
        refreshed_at_ms: facts,
        last_attempted_at_ms: facts,
        status: { status: 'refreshed' },
      }),
      prRow(3, {
        last_attempted_at_ms: tried,
        status: { status: 'failed_never_refreshed', error: 'unauthorized' },
      }),
      prRow(4, {
        title: 'feat: kept',
        state: 'merged',
        merged_at: '2026-09-05T12:00:00Z',
        additions: 120,
        deletions: 4,
        head_ref_name: 'feat/kept',
        refreshed_at_ms: facts,
        last_attempted_at_ms: tried,
        status: { status: 'failed_after_refresh', error: 'rate_limited' },
      }),
    ],
  }));
  mount(source);
  await loaded('feat: kept');
  const [neverRow, fresh, failed, stale] = cells();
  expect(neverRow![4]).toBe('never refreshed');
  expect(fresh![4]).toBe(`refreshed${shown(facts)}`);
  expect(failed).toEqual([
    'Title not cachedxtrace/app#103',
    NO_STATE,
    NO_SIZE,
    '1',
    `failed · never refreshednot authorized; sign in with gh auth login · tried ${shown(tried)}`,
  ]);
  // The earlier successful facts are all still there, beside the failure.
  expect(stale).toEqual([
    'feat: keptxtrace/app#104 ⑂ feat/kept',
    `merged${shown(Date.parse('2026-09-05T12:00:00Z'))}`,
    '+120 additions −4 deletions',
    '1',
    `stale · last refresh failedrate limited · facts from ${shown(facts)}`,
  ]);
  const body = within(table()).getAllByRole('row').slice(1);
  // Instants are machine-readable whatever zone formats them.
  expect(body[3]!.querySelector('time')!.getAttribute('dateTime')).toBe('2026-09-05T12:00:00.000Z');
  // "refreshed" says when, not how current: it is never toned as healthy.
  const tones = body.map((r) => r.querySelector('.xt-pr-cache-word')!.getAttribute('data-tone'));
  expect(tones).toEqual(['meta', 'info', 'warning', 'warning']);
  // The canonical address and the whole branch are stated, never linked.
  expect(body[3]!.querySelector('.xt-pr-meta')!.getAttribute('title')).toBe(
    'https://github.com/xtrace/app/pull/104 · branch feat/kept',
  );
  expect(body[2]!.querySelector('.xt-pr-meta')!.getAttribute('title')).toBe(
    'https://github.com/xtrace/app/pull/103',
  );
  expectLocalReadsOnly(source);
});

it('survives stored values it cannot read without throwing or guessing', async () => {
  const source = trappedSource(async () => ({
    rows: [
      prRow(1, {
        title: 'feat: odd',
        state: 'merged',
        merged_at: 'not-a-time',
        refreshed_at_ms: 9e15,
        last_attempted_at_ms: 9e15,
        status: { status: 'refreshed' },
      }),
      prRow(2, { status: { status: 'failed_after_refresh', error: 'timeout' } }),
    ],
  }));
  mount(source);
  await loaded('feat: odd');
  const [odd, untimed] = cells();
  expect(odd![1]).toBe('mergednot-a-time');
  expect(odd![4]).toBe('refreshedan unreadable time');
  expect(untimed![4]).toBe('stale · last refresh failedtimed out · facts from an unknown time');
});

// --- empty, failure and retry ----------------------------------------------------

it('says when no session links a pull request, without inventing a row or a figure', async () => {
  const source = trappedSource(async () => ({ rows: [] }));
  mount(source);
  expect(await loaded('No session links a pull request yet.')).toBeTruthy();
  expect(meta()).toBe(
    '0 pull requests indexed · cached facts · lifetime link counts · by repo, then number',
  );
  expect(cells()).toEqual([['No session links a pull request yet.']]);
  // Filtering an empty list does not turn "nothing indexed" into "no match".
  fireEvent.change(screen.getByRole('searchbox', { name: 'Filter pull requests' }), {
    target: { value: 'anything' },
  });
  expect(cells()).toEqual([['No session links a pull request yet.']]);
  expect(meta()).toContain('0 of 0 shown');
  expectLocalReadsOnly(source);
});

it('shows a failed local read safely and reads storage again on Retry, never GitHub', async () => {
  let release: (list: PrList) => void = () => {};
  const read = vi
    .fn<() => Promise<PrList>>()
    .mockRejectedValueOnce(new Error('backend-specific detail'))
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
  const source = trappedSource(read);
  mount(source);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toBe('Indexed pull requests could not be read. Retry');
  expect(screen.queryByText(/backend-specific detail/)).toBeNull();
  expect(screen.queryByRole('table')).toBeNull();
  // No population is claimed for a list that was not read.
  expect(meta()).toBe('cached facts · lifetime link counts · by repo, then number');
  const retry = within(alert).getByRole('button', { name: 'Retry' });
  retry.focus();
  fireEvent.click(retry);
  // A list that was never read goes back to reading: the alert and its button
  // are gone while the one new read runs, so a second cannot be stacked on it.
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  expect(within(table()).getByRole('status', { name: 'Loading rows' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  expect(source.pullRequests).toHaveBeenCalledTimes(2);
  await act(async () => release({ rows: [prRow(1, { title: 'feat: back' })] }));
  await loaded('feat: back');
  expect(screen.queryByRole('alert')).toBeNull();
  expect(source.pullRequests).toHaveBeenCalledTimes(2);
  expectLocalReadsOnly(source);
});

it('stays explicitly unavailable in a browser preview and reads nothing', async () => {
  const source = { ...trappedSource(async () => ({ rows: [] })), kind: 'preview' as const };
  mount(source);
  expect(await screen.findByText('Open the desktop app to see its indexed pull requests.'));
  expect(screen.queryByRole('table')).toBeNull();
  expect(source.pullRequests).not.toHaveBeenCalled();
  // The preview reads no index either: the sidebar keeps its plain plugin row.
  expect(source.nativeIndexStatus).not.toHaveBeenCalled();
});

// --- local filter and a large cached list ----------------------------------------

it('filters the rows already read by title, repository or number, without another read', async () => {
  const source = trappedSource(async () => ({ rows: manyPrRows(600) }));
  mount(source);
  await loaded('feat: milestone 0');
  expect(cells()).toHaveLength(600);
  expect(meta()).toContain('600 pull requests indexed');
  const box = screen.getByRole('searchbox', { name: 'Filter pull requests' });
  const filter = (value: string) => fireEvent.change(box, { target: { value } });
  // The Shell's search shortcut reaches it, so the list is filtered from the keyboard.
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(document.activeElement).toBe(box);
  filter('  MILESTONE 5 ');
  expect(names()).toEqual([
    'feat: milestone 50octo-org/tools#1050',
    'feat: milestone 500xtrace/app#1500',
    'feat: milestone 550xtrace/app#1550',
  ]);
  expect(meta()).toBe(
    '3 of 600 shown · cached facts · lifetime link counts · by repo, then number',
  );
  filter('octo-org/');
  expect(cells()).toHaveLength(300);
  filter('#1599');
  expect(names()).toEqual(['Title not cachedxtrace/app#1599']);
  filter('tools#12');
  expect(cells()).toHaveLength(100);
  filter('no such pull request');
  expect(cells()).toEqual([['No pull request matches this filter.']]);
  expect(meta()).toContain('0 of 600 shown');
  filter('');
  expect(cells()).toHaveLength(600);
  expect(source.pullRequests).toHaveBeenCalledOnce();
  expectLocalReadsOnly(source);
});

it('keeps a large list inside one focusable scroll region under a fixed header', async () => {
  const source = trappedSource(async () => ({ rows: manyPrRows(600) }));
  mount(source);
  await loaded('feat: milestone 0');
  const region = screen.getByRole('region', { name: 'Linked pull requests scroll area' });
  expect(region.contains(table())).toBe(true);
  // Reachable and scrollable from the keyboard: the region itself takes focus.
  expect(region.tabIndex).toBe(0);
  region.focus();
  expect(document.activeElement).toBe(region);
  expect(table().querySelector('.xt-table-head')!.getAttribute('data-sticky')).toBe('true');
  expect(
    within(table())
      .getAllByRole('columnheader')
      .map((header) => header.textContent),
  ).toEqual(['pull request · repo · branch', 'state', 'size', 'sessions', 'cache status']);
  // Rows hold nothing that takes a tab stop, so Tab leaves the list in one step.
  expect(region.querySelectorAll('a, button, input, [tabindex]:not([tabindex="-1"])')).toHaveLength(
    0,
  );
});

it('opens the explanation from the keyboard-reachable summary and links only inside the app', async () => {
  const source = trappedSource(async () => structuredClone(exported.pull_requests));
  mount(source);
  await loaded('octo-org/xtrace-fixture#11');
  const counted = screen.getByTestId('prs-counted') as HTMLDetailsElement;
  const summary = counted.querySelector('summary')!;
  expect(summary.textContent).toBe('What is listed and counted');
  // A native summary: focusable and toggled by Enter or Space without script.
  expect(summary.querySelector('a, button')).toBeNull();
  fireEvent.click(summary);
  expect(counted.open).toBe(true);
  const link = within(counted).getByRole('link', { name: 'Dashboard' });
  expect(link.getAttribute('href')).toBe('/dashboard');
  expectLocalReadsOnly(source);
});

it('reads the list once under StrictMode, as the app mounts it', async () => {
  const source = trappedSource(async () => structuredClone(exported.pull_requests));
  render(
    <StrictMode>
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter initialEntries={['/prs?view=inventory']}>
            <AppRoutes />
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>
    </StrictMode>,
  );
  await loaded('octo-org/xtrace-fixture#11');
  expect(cells()).toHaveLength(3);
  // The rehearsal mount and the real one share the runtime's one client, so
  // the rehearsal's read is the read; nothing is started twice.
  expect(source.pullRequests).toHaveBeenCalledOnce();
  expectLocalReadsOnly(source);
});
