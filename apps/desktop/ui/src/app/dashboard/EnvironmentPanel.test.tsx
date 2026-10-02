import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { events, type DataEvent } from '../../data/ipc-names';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import { SYNTHETIC_UNTIMED, syntheticEnvironment } from './environment.synthetic';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const f1 = (days = 7) =>
  structuredClone(exported.environments.find((report) => report.window.days === days)!);
const synthetic = (days = 7) => syntheticEnvironment(f1(days));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

function nativeSource(report: (days: number) => EnvironmentMetrics | Promise<EnvironmentMetrics>) {
  const listeners = new Map<DataEvent, Set<() => void>>();
  const dashboard = (days: number) =>
    structuredClone(exported.dashboards.find((entry) => entry.window.days === days)!);
  const source = {
    kind: 'native' as const,
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
    sessionsList: async () => ({ rows: [], next: null }),
    dashboard: vi.fn(async (days: number) => dashboard(days)),
    tokensByHost: vi.fn(async (days: number) => ({
      window: dashboard(days).window,
      hosts: dashboard(days).tokens_by_host,
    })),
    environment: vi.fn(async (days: number) => report(days)),
    subscribe: async (event: DataEvent, listener: () => void) => {
      const set = listeners.get(event) ?? new Set();
      set.add(listener);
      listeners.set(event, set);
      return () => void set.delete(listener);
    },
    emit: (event: DataEvent) => listeners.get(event)?.forEach((listener) => listener()),
  } satisfies DataSource & { emit: (event: DataEvent) => void };
  return source;
}

function mount(source: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/dashboard']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}
const panel = () =>
  screen.getByRole('heading', { level: 2, name: 'Environment' }).closest('section')!;
const loaded = async () => {
  await screen.findByRole('heading', { level: 2, name: 'Environment' });
  return within(panel()).findByTestId('environment-summary');
};
const topList = () => within(panel()).getByRole('list', { name: /^Most-called identities/ });
/** The display name, without the host the compact card row appends. */
const nameOf = (row: Element) =>
  [...row.querySelector('.xt-env-name')!.childNodes]
    .filter((node) => !(node instanceof HTMLElement && node.classList.contains('xt-env-host')))
    .map((node) => node.textContent)
    .join('');
const rows = (list: HTMLElement) => within(list).getAllByRole('listitem').map(nameOf);
const row = (name: string, list = topList()) =>
  within(list)
    .getAllByRole('listitem')
    .find((item) => nameOf(item).startsWith(name))!;
/** Opens the observed dialog, which holds each identity's kind, host and surface context. */
async function observedDialog(count: number) {
  fireEvent.click(within(panel()).getByRole('button', { name: `Observed identities · ${count}` }));
  const dialog = await screen.findByRole('dialog', { name: /^Observed identities · last/ });
  return within(dialog).getByRole('list', { name: `All ${count} observed` });
}
async function unresolvedDialog() {
  fireEvent.click(within(panel()).getByRole('button', { name: 'Unresolved attribution details' }));
  return screen.findByRole('dialog', { name: 'Unresolved attribution' });
}
async function configuredDialog(count: number) {
  fireEvent.click(
    within(panel()).getByRole('button', { name: `Configured components · ${count}` }),
  );
  return screen.findByRole('dialog', { name: 'Configured components' });
}
/** Claims an unknown inventory can never support. */
const inventoryClaims =
  /not installed|never called|never used|unused|installed:|\d+\s*\/\s*\d+\s*installed|% (used|installed)/i;

it('shows F1 observed usage with an unknown inventory beside separately labelled configured facts', async () => {
  const source = nativeSource((days) => f1(days));
  mount(source);
  await screen.findByTestId('dashboard-summary');
  expect((await loaded()).textContent).toBe(
    '5 observed calls · 1 identity · last 7dInventory unknown',
  );
  expect(source.environment).toHaveBeenCalledWith(7);
  const pill = within(panel()).getByText('Inventory unknown');
  expect(pill.closest('[data-tone]')!.getAttribute('data-tone')).toBe('meta');
  expect(panel().textContent).not.toContain('Environment data is unavailable');
  expect(topList().getAttribute('aria-label')).toBe('Most-called identities, top 1 of 1');
  const read = row('Read');
  expect(read.querySelector('.xt-env-calls')!.textContent).toBe('5 calls');
  // The compact card row: kind, name with host, calls and strip on one line; the full context is
  // its title and, visibly, in the observed dialog.
  expect(read.querySelector('.xt-env-context')).toBeNull();
  expect(read.querySelector('.xt-kind')!.textContent).toBe('builtin');
  expect(read.querySelector('.xt-env-name')!.textContent).toBe('Read · claude');
  expect(read.querySelector('.xt-env-name')!.getAttribute('title')).toBe(
    'Read · builtin · claude · cli',
  );
  expect(topList().getAttribute('tabindex')).toBe('0');
  const strip = within(read).getByRole('group', {
    name: 'Read, claude: 5 calls per local day, last 14 days',
  });
  const days = within(strip).getAllByRole('img');
  expect(days).toHaveLength(14);
  expect(days[0].getAttribute('aria-label')).toBe('Aug 25: 0');
  expect(days[13].getAttribute('aria-label')).toBe('Sep 7: 5');
  expect(within(panel()).queryByTestId('environment-unresolved')).toBeNull();
  const observed = await observedDialog(1);
  expect(row('Read', observed).querySelector('.xt-env-context')!.textContent).toBe(
    'builtin claude · cli',
  );
  fireEvent.keyDown(observed, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(within(panel()).getByTestId('environment-strip-note').textContent).toMatch(
    /fixed 14 local days Aug 25\s–\sSep 7, 2026, UTC, any range/,
  );
  expect(panel().textContent).toContain('installed components are unknown');
  expect(panel().textContent).not.toMatch(inventoryClaims);

  // Configured components are a separate dialog, never presented as installed or callable.
  const dialog = await configuredDialog(14);
  const text = dialog.textContent!;
  expect(text).toContain(
    'Being configured does not show that a component is installed, can be called, or has been called.',
  );
  const configured = within(dialog).getByRole('list', { name: 'Configured components' });
  expect(within(configured).getAllByRole('listitem')).toHaveLength(14);
  expect(text).toContain('claude · plugin · cloudflare@cloudflare');
  expect(text).toContain('Claude plugin enablement settings · home · disabled');
  expect(text).toContain('Claude skills directory · home · enablement not stated');
  const sources = within(dialog).getByRole('list', { name: 'Configuration sources' });
  expect(sources.textContent).toContain(
    'claude · Claude skills directory · home 1incomplete · 2 stated, 1 skipped (entries without the documented shape)',
  );
  expect(sources.textContent).toContain('cursor · Cursor MCP config · repository 1not present');
  expect(text).toContain('Roots: home 1 read · repository 1 read · repository 2 read.');
  // Cache observations are labelled cache only, apart from configured components.
  const cache = within(dialog).getByRole('list', { name: 'Plugin cache observations' });
  expect(cache.closest('section')!.querySelector('h3')!.textContent).toBe('Cache only');
  expect(cache.textContent).toContain('codex · Codex plugin cache · home 1read · 1 stated');
  expect(within(configured).queryByText(/cache/i)).toBeNull();
  // No path, command, URL or credential-shaped value is rendered.
  expect(text).not.toMatch(/SYNTHETIC-FIXTURE-VALUE|https?:|\/Users|\.json|\.toml|\.claude|npx|--/);
  expect(text).not.toMatch(inventoryClaims);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
});

it('shows the top eight identities in Rust order and every identity in a dialog', async () => {
  mount(nativeSource((days) => synthetic(days)));
  await loaded();
  const top8 = [
    'Bash',
    'Hook summary events · no hook named',
    'memhub · search_memory',
    'shell',
    'Skill · skill name not stated',
    'Composer',
    'LegacyTool · kind not recorded',
    'review',
  ];
  expect(rows(topList())).toEqual(top8);
  expect(topList().getAttribute('aria-label')).toBe('Most-called identities, top 8 of 11');
  const all = await observedDialog(11);
  expect(all.closest('[role="dialog"]')!.getAttribute('aria-labelledby')).toBeTruthy();
  expect(screen.getByRole('dialog', { name: 'Observed identities · last 7d' })).toBeTruthy();
  expect(rows(all)).toEqual([
    ...top8,
    'Task',
    'mcp__broken · MCP server and tool not stated',
    'apply_patch',
  ]);
  // A strip-only identity keeps its selected-range zero and its strip surface.
  const patch = row('apply_patch', all);
  expect(patch.querySelector('.xt-env-calls')!.textContent).toBe('0 calls');
  expect(patch.querySelector('.xt-env-context')!.textContent).toBe('builtin codex · cli');
});

it('never reranks: the panel keeps the report order even against the counts', async () => {
  mount(
    nativeSource((days) => {
      const report = synthetic(days);
      // Swap the first two rows' positions only; Rust's order is the order shown.
      report.identities = [
        report.identities[1],
        report.identities[0],
        ...report.identities.slice(2),
      ];
      return report;
    }),
  );
  await loaded();
  expect(rows(topList()).slice(0, 2)).toEqual(['Hook summary events · no hook named', 'Bash']);
});

it('shades the fixed strip at 0, under 3, under 8 and 8 or more calls', async () => {
  mount(nativeSource((days) => synthetic(days)));
  await loaded();
  const days = within(within(row('Bash')).getByRole('group')).getAllByRole('img');
  expect(days.slice(0, 4).map((day) => day.getAttribute('aria-label'))).toEqual([
    'Aug 25: 0',
    'Aug 26: 2',
    'Aug 27: 5',
    'Aug 28: 9',
  ]);
  expect(days.slice(0, 4).map((day) => day.getAttribute('data-level'))).toEqual([
    '0',
    '1',
    '2',
    '3',
  ]);
});

it('labels hook summaries as events and shows what cannot be attributed', async () => {
  mount(nativeSource((days) => synthetic(days)));
  await loaded();
  const hook = row('Hook summary events');
  expect(hook.querySelector('.xt-env-calls')!.textContent).toBe('20 events');
  expect(hook.textContent).not.toMatch(/stop_hook_summary|calls/);
  const observed = await observedDialog(11);
  expect(row('Hook summary events', observed).querySelector('.xt-env-context')!.textContent).toBe(
    'hook claude · cli',
  );
  fireEvent.keyDown(observed, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  const note = within(panel()).getByRole('note', { name: 'Unresolved attribution' });
  expect(note.textContent).toContain('38 unresolved observations, including untimed');
  expect(panel().textContent).not.toMatch(inventoryClaims);
  const dialog = await unresolvedDialog();
  expect(
    within(within(dialog).getByRole('list', { name: 'Unresolved in the last 7d' }))
      .getAllByRole('listitem')
      .map((item) => item.textContent),
  ).toEqual([
    'claude · 20 hook summary events: a summary says hooks ran, not which one',
    'claude · 6 skill calls: skill name not stated',
    'claude · 4 calls: kind not recorded',
    'claude · 1 MCP call: server and tool not stated',
  ]);
  expect(dialog.textContent).not.toMatch(inventoryClaims);
});

it('states unresolved counts without a denominator and keeps untimed observations apart', async () => {
  mount(nativeSource((days) => synthetic(days)));
  await loaded();
  // selected_unresolved_calls (38) counts 7 untimed observations that selected_calls (104) does
  // not, so no "of 104" fraction or share of the observed calls may be shown.
  const note = within(panel()).getByRole('note', { name: 'Unresolved attribution' });
  expect(note.textContent).not.toMatch(/\bof\b|%|104/);
  const dialog = await unresolvedDialog();
  expect(dialog.textContent).not.toMatch(/\d+ of \d+|%/);
  expect(dialog.textContent).toContain('none is a share of the observed calls');
  const ranged = within(dialog).getByRole('list', { name: 'Unresolved in the last 7d' });
  expect(ranged.textContent).not.toMatch(/timestamp|untimed/);
  const untimed = within(dialog).getByRole('list', { name: 'Untimed observations' });
  expect(untimed.closest('section')!.querySelector('h3')!.textContent).toBe('Untimed');
  expect(
    within(untimed)
      .getAllByRole('listitem')
      .map((item) => item.textContent),
  ).toEqual([
    `codex · ${SYNTHETIC_UNTIMED} untimed observations: no timestamp, so no selected range can hold it`,
  ]);
  expect(untimed.closest('section')!.textContent).toContain(
    'cannot be assigned to any selected range',
  );
});

it('names only untimed observations when every unresolved one lacks a timestamp', async () => {
  mount(
    nativeSource((days) => {
      const report = f1(days);
      report.selected.unresolved = [{ host: 'claude', reason: 'missing_timestamp', calls: 3 }];
      report.totals = { ...report.totals, selected_unresolved_calls: 3 };
      return report;
    }),
  );
  await loaded();
  const note = within(panel()).getByRole('note', { name: 'Unresolved attribution' });
  expect(note.textContent).toContain('3 unresolved observations');
  expect(note.textContent).not.toContain('5');
  const dialog = await unresolvedDialog();
  expect(within(dialog).queryByRole('list', { name: /Unresolved in the last/ })).toBeNull();
  expect(within(dialog).getByRole('list', { name: 'Untimed observations' }).textContent).toBe(
    'claude · 3 untimed observations: no timestamp, so no selected range can hold it',
  );
});

it('keeps unknown kinds, unknown details and raw future surfaces visible', async () => {
  mount(nativeSource((days) => synthetic(days)));
  await loaded();
  const unknown = row('LegacyTool').querySelector('.xt-kind')!;
  expect(unknown.textContent).toBe('unknown kind');
  expect(unknown.getAttribute('data-tone')).toBe('warning');
  expect(row('Composer').querySelector('.xt-kind')!.textContent).toBe('future_kind');
  const all = await observedDialog(11);
  const context = (name: string) => row(name, all).querySelector('.xt-env-context')!.textContent;
  expect(context('shell')).toBe('builtin codex · future.app');
  expect(context('Composer')).toBe('future_kind cursor · unknown surface');
  expect(context('LegacyTool')).toBe('unknown kind claude · cli');
  expect(context('review')).toBe('command claude · cli');
});

it('updates selected totals with the range while the strip keeps its fixed 14 dates', async () => {
  const source = nativeSource((days) => synthetic(days));
  mount(source);
  await loaded();
  expect(row('Bash').querySelector('.xt-env-calls')!.textContent).toBe('42 calls');
  const firstDay = () =>
    within(within(row('Bash')).getByRole('group'))
      .getAllByRole('img')[0]
      .getAttribute('aria-label');
  expect(firstDay()).toBe('Aug 25: 0');
  for (const [range, calls, total] of [
    ['14d', '84 calls', '208 observed calls'],
    ['30d', '168 calls', '416 observed calls'],
  ] as const) {
    fireEvent.click(screen.getByRole('radio', { name: range }));
    await waitFor(() =>
      expect(within(panel()).getByTestId('environment-summary').textContent).toContain(
        `${total} · 11 identities · last ${range}`,
      ),
    );
    expect(row('Bash').querySelector('.xt-env-calls')!.textContent).toBe(calls);
    expect(firstDay()).toBe('Aug 25: 0');
    expect(within(row('Bash')).getAllByRole('img')).toHaveLength(14);
  }
  expect(new Set(source.environment.mock.calls.map(([days]) => days))).toEqual(
    new Set([7, 14, 30]),
  );
  expect(source.environment).toHaveBeenLastCalledWith(30);
  // The sidebar keeps its own recorded-token caption for the same range.
  expect(screen.getByText('Recorded tokens · 30d')).toBeTruthy();
});

it('refreshes on enrichment through the shared metrics invalidation', async () => {
  let enriched = false;
  const source = nativeSource((days) => {
    const report = synthetic(days);
    if (enriched) report.identities[0].calls += 1;
    return report;
  });
  mount(source);
  await loaded();
  expect(row('Bash').querySelector('.xt-env-calls')!.textContent).toBe('42 calls');
  enriched = true;
  act(() => source.emit(events.nativeIndexStatus));
  await waitFor(
    () => expect(row('Bash').querySelector('.xt-env-calls')!.textContent).toBe('43 calls'),
    { timeout: 3000 },
  );
});

it('shows loading, then a safe error with retry, without hiding the rest of the Dashboard', async () => {
  let fail = true;
  let release!: () => void;
  const gate = new Promise<void>((done) => (release = done));
  const source = nativeSource(async (days) => {
    await gate;
    if (fail) throw new Error('backend-specific /Users/someone/.claude.json detail');
    return f1(days);
  });
  mount(source);
  await screen.findByTestId('dashboard-summary');
  expect(within(panel()).getByRole('status').textContent).toBe(
    'Reading environment usage for the last 7d…',
  );
  act(() => release());
  const alert = await within(panel()).findByRole('alert');
  expect(alert.textContent).toContain('Environment usage could not be loaded.');
  expect(document.body.textContent).not.toMatch(/backend-specific|\/Users|\.claude\.json/);
  expect(screen.getByRole('heading', { name: 'Sessions' })).toBeTruthy();
  fail = false;
  fireEvent.click(within(alert).getByRole('button', { name: 'Retry environment usage' }));
  expect((await loaded()).textContent).toContain('5 observed calls');
});

it('states an empty observation without inventing zero-call components', async () => {
  mount(
    nativeSource((days) => {
      const report = f1(days);
      report.identities = [];
      report.selected.observed = [];
      report.strip.observed = [];
      report.totals = { ...report.totals, selected_calls: 0, strip_calls: 0, identities: 0 };
      return report;
    }),
  );
  await loaded();
  expect(within(panel()).getByTestId('environment-empty').textContent).toBe(
    'No tool calls were observed in the last 7d or the fixed 14 local days.',
  );
  expect(within(panel()).queryByRole('list')).toBeNull();
  // Configured facts are still one step away, and still not a usage claim.
  expect(within(panel()).getByRole('button', { name: 'Configured components · 14' })).toBeTruthy();
  expect(panel().textContent).not.toMatch(inventoryClaims);
});

it('does not read environment usage in a browser preview', async () => {
  const source = { ...nativeSource((days) => f1(days)), kind: 'preview' as const };
  mount(source);
  await screen.findByText('Open the desktop app to read local Dashboard metrics.');
  expect(source.environment).not.toHaveBeenCalled();
});

it('says no configured component was verified when a source was not fully read', async () => {
  mount(
    nativeSource((days) => {
      const report = f1(days);
      report.configured = [];
      report.totals = { ...report.totals, configured_components: 0 };
      report.sources = [
        {
          host: 'claude',
          source: 'claude_skill_directory',
          scope: 'home',
          root_index: 0,
          status: { state: 'incomplete', stated: 257, skipped: 257, reason: 'entry_limit' },
        },
        {
          host: 'claude',
          source: 'claude_user_mcp_config',
          scope: 'home',
          root_index: 0,
          status: { state: 'unreadable' },
        },
        {
          host: 'codex',
          source: 'codex_mcp_config',
          scope: 'home',
          root_index: 0,
          status: { state: 'unsupported', reason: 'file_too_large' },
        },
        {
          host: 'cursor',
          source: 'cursor_mcp_config',
          scope: 'home',
          root_index: 0,
          status: { state: 'incomplete', stated: 300, skipped: 300, reason: 'entry_limit' },
        },
      ];
      report.cache = [
        {
          host: 'codex',
          source: 'codex_plugin_cache',
          scope: 'home',
          root_index: 0,
          status: { state: 'incomplete', stated: 257, skipped: 257, reason: 'entry_limit' },
        },
      ];
      return report;
    }),
  );
  await loaded();
  const dialog = await configuredDialog(0);
  expect(within(dialog).getByTestId('environment-configured-empty').textContent).toBe(
    'No configured components were verified. 4 sources were not fully read, so components may be configured that this does not show.',
  );
  expect(dialog.textContent).not.toMatch(/name no component|names no component|states one/);
  // A directory stops listing one past the bound, so its counts are lower bounds; a file over the
  // bound states its full count.
  const sources = within(dialog).getByRole('list', { name: 'Configuration sources' });
  const lines = within(sources)
    .getAllByRole('listitem')
    .map((item) => item.textContent);
  expect(lines[0]).toBe(
    'claude · Claude skills directory · home 1incomplete · at least 257 entries, at least 257 skipped (over the entry limit; the listing stopped, so both are lower bounds)',
  );
  expect(lines[3]).toBe(
    'cursor · Cursor MCP config · home 1incomplete · 300 stated, 300 skipped (over the entry limit)',
  );
  const cache = within(dialog).getByRole('list', { name: 'Plugin cache observations' });
  expect(cache.textContent).toContain('at least 257 entries');
  expect(cache.textContent).toContain('lower bounds');
  expect(dialog.textContent).not.toMatch(inventoryClaims);
});

it('says no configured component was verified when every source was read and states none', async () => {
  mount(
    nativeSource((days) => {
      const report = f1(days);
      report.configured = [];
      report.sources = report.sources.map((item) => ({ ...item, status: { state: 'missing' } }));
      return report;
    }),
  );
  await loaded();
  const dialog = await configuredDialog(0);
  expect(within(dialog).getByTestId('environment-configured-empty').textContent).toBe(
    'No configured components were verified. No supported source that was read states one.',
  );
});
