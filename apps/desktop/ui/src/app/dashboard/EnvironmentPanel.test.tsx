import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { events, type DataEvent } from '../../data/ipc-names';
import type { TimeRange } from '../../kit/TopBar';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { DashboardDetailsProvider } from './DashboardDetailsProvider';
import { EnvironmentPanel } from './EnvironmentPanel';
import {
  SYNTHETIC_UNTIMED,
  singleSkillEnvironment,
  syntheticEnvironment,
} from './environment.synthetic';

/** Pull-request refresh is not exercised by this test. */
const unavailable = async (): Promise<never> => {
  throw new Error('Pull requests are not part of this test.');
};

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
    dashboard: vi.fn(async (days: number) => dashboard(days)),
    tokensByHost: vi.fn(async (days: number) => ({
      window: dashboard(days).window,
      hosts: dashboard(days).tokens_by_host,
    })),
    today: async () => exported.today,
    environment: vi.fn(async (days: number) => report(days)),
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
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

/**
 * The panel is no longer on the Dashboard (Overview took its place); its
 * component and report are kept, so it is tested on its own with a range
 * control of its own.
 */
function Harness() {
  const [range, setRange] = useState<TimeRange>('7d');
  return (
    <DashboardDetailsProvider>
      <div role="radiogroup" aria-label="Range">
        {(['7d', '14d', '30d'] as const).map((value) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={value === range}
            onClick={() => setRange(value)}
          >
            {value}
          </button>
        ))}
      </div>
      <EnvironmentPanel range={range} />
    </DashboardDetailsProvider>
  );
}

function mount(source: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/']}>
          <Harness />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}
const panel = () =>
  screen.getByRole('heading', { level: 2, name: 'Environment' }).closest('section')!;
/** The loaded body: the identity list's column header, or the empty-state line. */
const loaded = async () => {
  await screen.findByRole('heading', { level: 2, name: 'Environment' });
  return within(panel()).findByTestId(/^environment-(columns|empty)$/);
};
const topList = () => within(panel()).getByRole('list', { name: /^Most-called identities/ });
/** The display name with its note; the compact card row's kind and host are its siblings. */
const nameOf = (row: Element) => row.querySelector('.xt-env-name')!.textContent!;
const rows = (list: HTMLElement) => within(list).getAllByRole('listitem').map(nameOf);
/** The text in view, without the words only assistive technology reads. */
const visibleText = (element: Element) =>
  [...element.childNodes]
    .filter((node) => !(node instanceof HTMLElement && node.classList.contains('sr-only')))
    .map((node) => node.textContent)
    .join('');
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
/** The header's unresolved triangle; its name carries the count. */
const flag = () => within(panel()).getByRole('button', { name: /^Unresolved attribution: / });
async function unresolvedDialog() {
  fireEvent.click(flag());
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

it('shows a synthetic skill with an unknown inventory beside separately labelled configured facts', async () => {
  const source = nativeSource((days) => singleSkillEnvironment(f1(days)));
  mount(source);
  await loaded();
  // The card shows no summary line of call and identity totals.
  expect(within(panel()).queryByTestId('environment-summary')).toBeNull();
  expect(panel().textContent).not.toContain('observed calls');
  expect(source.environment).toHaveBeenCalledWith(7);
  // The header carries no inventory pill.
  expect(within(panel()).queryByText(/^Inventory /)).toBeNull();
  expect(panel().textContent).not.toContain('Environment data is unavailable');
  expect(topList().getAttribute('aria-label')).toBe('Most-called identities, top 1 of 1');
  const read = row('Review');
  expect(read.querySelector('.xt-env-calls')!.textContent).toBe('5 calls');
  // The compact card row, in the header's column order: the tool (its name, then its kind, then
  // its host as a muted suffix), the fixed strip, then the calls at the right edge; the full
  // context is the tool's title and, visibly, in the observed dialog.
  expect(read.querySelector('.xt-env-context')).toBeNull();
  expect([...read.children].map((cell) => cell.className.split(' ')[0])).toEqual([
    'xt-env-tool',
    'xt-day-strip',
    'xt-env-calls',
  ]);
  const tool = read.querySelector('.xt-env-tool')!;
  expect([...tool.children].map((part) => part.className.split(' ')[0])).toEqual([
    'xt-env-name',
    'xt-kind',
    'xt-env-host',
  ]);
  expect(read.querySelector('.xt-kind')!.textContent).toBe('skill');
  expect(read.querySelector('.xt-env-name')!.textContent).toBe('Review');
  expect(read.querySelector('.xt-env-host')!.textContent).toBe(' · claude');
  expect(tool.getAttribute('title')).toBe('Review · skill · claude · cli');
  expect(topList().getAttribute('tabindex')).toBe('0');
  // One column header sits over the list, outside the rows that scroll, and describes it: the
  // tool with its kind and host, the fixed 14-day strips, then the selected range's calls. The
  // calls track is the longest count shown ("5 calls": 7 characters) in the row font's width.
  const columns = within(panel()).getByTestId('environment-columns');
  expect(columns.nextElementSibling).toBe(topList());
  expect(topList().getAttribute('aria-describedby')).toBe(columns.id);
  expect(columns.textContent).toBe('Tool · kind · host14 daysCalls · last 7d');
  expect([...columns.children].map((cell) => visibleText(cell))).toEqual([
    'Tool · kind',
    '14 days',
    'Calls',
  ]);
  expect(columns.parentElement!.style.getPropertyValue('--calls')).toBe('7');
  const strip = within(read).getByRole('group', {
    name: 'Review, claude: 5 calls per local day, last 14 days',
  });
  const days = within(strip).getAllByRole('img');
  expect(days).toHaveLength(14);
  expect(days[0].getAttribute('aria-label')).toBe('Aug 25: 0');
  expect(days[13].getAttribute('aria-label')).toBe('Sep 7: 5');
  expect(within(panel()).queryByTestId('environment-unresolved')).toBeNull();
  const observed = await observedDialog(1);
  expect(row('Review', observed).querySelector('.xt-env-context')!.textContent).toBe(
    'skill claude · cli',
  );
  fireEvent.keyDown(observed, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  // The strip column's header is the strips' legend control: the fixed scope stays in view
  // whatever range is selected, and the dates, zone, shade steps and the observed-only limit are
  // one dialog away from it, over the page. The footer keeps only the dialog controls.
  const legend = within(panel()).getByTestId('environment-strip-legend');
  expect(legend.tagName).toBe('BUTTON');
  expect(legend.parentElement).toBe(columns);
  expect(legend).toBe(
    within(panel()).getByRole('button', {
      name: '14 days · activity strips, fixed 14 local days',
    }),
  );
  expect(legend.getAttribute('aria-expanded')).toBe('false');
  expect(within(panel()).queryByTestId('environment-strip-note')).toBeNull();
  expect(panel().textContent).not.toContain('shades at');
  expect(panel().querySelector('.xt-env-actions')!.textContent).toBe('Observed · 1Configured · 14');
  fireEvent.click(legend);
  const strips = await screen.findByRole('dialog', { name: 'Activity strips' });
  expect(within(strips).getByTestId('environment-strip-note').textContent).toMatch(
    /fixed 14 local days Aug 25\s–\sSep 7, 2026, UTC, any range/,
  );
  expect(strips.textContent).toContain('installed components are unknown');
  // The shade steps moved into the dialog with the control, read aloud as one phrase.
  const scale = strips.querySelector('.xt-env-legend-scale')!;
  expect(scale.textContent).toBe('Shades138+ at 1, 3 and 8 or more calls a day');
  expect(scale.querySelectorAll('.xt-env-legend-swatch')).toHaveLength(3);
  expect(strips.textContent).not.toMatch(inventoryClaims);
  fireEvent.keyDown(strips, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(legend).toBe(document.activeElement);
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
    'Hook summary events · name absent from index',
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
  expect(patch.querySelector('.xt-env-context')!.textContent).toBe('skill codex · cli');
  // The dialog keeps its richer row: name, calls, then kind with host and surface over the strip.
  expect([...patch.children].map((cell) => cell.className.split(' ')[0])).toEqual([
    'xt-env-name',
    'xt-env-calls',
    'xt-env-context',
    'xt-day-strip',
  ]);
  expect(patch.querySelector('.xt-env-tool, .xt-env-host')).toBeNull();
});

it('sizes the calls column to the longest count shown and keeps a long name with its kind and host', async () => {
  mount(
    nativeSource((days) => {
      const report = synthetic(days);
      // Test-only shapes, in report order: a six-figure count on a name longer than any card
      // line, with the totals moved by the same amount so nothing shown disagrees.
      const [first, ...rest] = report.identities;
      const calls = 123_456;
      const identity = {
        ...first.identity,
        name: 'Bash-with-a-tool-name-longer-than-any-card-line-can-show-at-once',
        skill: 'Bash-with-a-tool-name-longer-than-any-card-line-can-show-at-once',
      };
      report.identities = [{ ...first, calls, identity }, ...rest];
      // The surfaces that observed it name the same identity, as Rust's report would.
      for (const usage of [report.selected, report.strip])
        for (const surface of usage.observed)
          for (const item of surface.by_identity)
            if (item.identity.name === first.identity.name) item.identity = identity;
      report.totals = {
        ...report.totals,
        selected_calls: report.totals.selected_calls - first.calls + calls,
      };
      return report;
    }),
  );
  await loaded();
  const columns = within(panel()).getByTestId('environment-columns');
  // "123,456 calls" is 13 characters wide in the row's monospace font.
  expect(columns.parentElement!.style.getPropertyValue('--calls')).toBe('13');
  const long = row('Bash-with-a-tool-name');
  expect(long.querySelector('.xt-env-calls')!.textContent).toBe('123,456 calls');
  expect(long.querySelector('.xt-kind')!.textContent).toBe('skill');
  expect(long.querySelector('.xt-env-host')!.textContent).toBe(' · claude');
  expect(long.querySelector('.xt-env-tool')!.getAttribute('title')).toBe(
    'Bash-with-a-tool-name-longer-than-any-card-line-can-show-at-once · skill · claude · cli',
  );
  // Hook rows count events; the column takes the longest text, whatever its unit.
  expect(row('Hook summary events').querySelector('.xt-env-calls')!.textContent).toBe('20 events');
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
  expect(rows(topList()).slice(0, 2)).toEqual([
    'Hook summary events · name absent from index',
    'Bash',
  ]);
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
  // A triangle at the header's end, not a line in the body.
  expect(flag().getAttribute('aria-label')).toBe(
    'Unresolved attribution: 38 unresolved observations, including untimed',
  );
  expect(flag().closest('header')).toBeTruthy();
  expect(flag().parentElement!.lastElementChild).toBe(flag());
  expect(panel().querySelector('.xt-section-panel')!.textContent).not.toContain('unresolved');
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

it('reads saved hook names only on open and states partial and unnamed coverage', async () => {
  const report = synthetic();
  const read = vi.fn(async (days: number, end: number, readId: string) => {
    expect([days, end, readId]).toEqual([
      report.window.days,
      report.window.end_ms,
      expect.any(String),
    ]);
    return {
      requested_summaries: 20,
      checked_summaries: 18,
      unavailable_summaries: 2,
      summaries_with_unnamed_commands: 1,
      labels: [
        {
          script_basename: 'brain_brief.py',
          display_label: 'Brain brief',
          summaries_mentioning: 17,
        },
      ],
    };
  });
  const cancel = vi.fn(async () => {});
  mount({ ...nativeSource(() => report), hookNames: { read, cancel } });
  await loaded();
  expect(read).not.toHaveBeenCalled();
  await observedDialog(11);
  const dialog = screen.getByRole('dialog', { name: 'Observed identities · last 7d' });
  await waitFor(() => expect(dialog.textContent).toContain('Checked 18 of 20 saved summaries'));
  expect(read).toHaveBeenCalledWith(report.window.days, report.window.end_ms, expect.any(String));
  expect(dialog.textContent).toContain('Names unavailable for 2 summaries');
  expect(dialog.textContent).toContain('1 checked summary also contains unnamed commands');
  expect(dialog.textContent).toContain('17 summaries mentioning this name');
  expect(dialog.textContent).toContain('does not prove that a hook ran');
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(cancel).toHaveBeenCalledWith(read.mock.calls[0]![2]));
});

it('keeps an empty or late hook-name answer out of a closed dialog', async () => {
  let finish!: (value: {
    requested_summaries: number;
    checked_summaries: number;
    unavailable_summaries: number;
    summaries_with_unnamed_commands: number;
    labels: [];
  }) => void;
  const read = vi.fn((days: number, end: number, readId: string) => {
    expect([days, end, readId]).toEqual([7, expect.any(Number), expect.any(String)]);
    return new Promise<{
      requested_summaries: number;
      checked_summaries: number;
      unavailable_summaries: number;
      summaries_with_unnamed_commands: number;
      labels: [];
    }>((resolve) => {
      finish = resolve;
    });
  });
  const cancel = vi.fn(async () => {});
  mount({ ...nativeSource((days) => synthetic(days)), hookNames: { read, cancel } });
  await loaded();
  await observedDialog(11);
  const dialog = screen.getByRole('dialog', { name: 'Observed identities · last 7d' });
  expect(dialog.textContent).toContain('Reading saved hook summaries');
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(cancel).toHaveBeenCalledTimes(1));
  await act(async () =>
    finish({
      requested_summaries: 20,
      checked_summaries: 20,
      unavailable_summaries: 0,
      summaries_with_unnamed_commands: 20,
      labels: [],
    }),
  );
  expect(screen.queryByRole('dialog')).toBeNull();
  await observedDialog(11);
  await act(async () =>
    finish({
      requested_summaries: 20,
      checked_summaries: 20,
      unavailable_summaries: 0,
      summaries_with_unnamed_commands: 20,
      labels: [],
    }),
  );
  expect(screen.getByRole('dialog').textContent).toContain(
    'No approved names found in checked summaries',
  );
});

it('states unresolved counts without a denominator and keeps untimed observations apart', async () => {
  mount(nativeSource((days) => synthetic(days)));
  await loaded();
  // selected_unresolved_calls (38) counts 7 untimed observations that selected_calls (104) does
  // not, so no "of 104" fraction or share of the observed calls may be shown.
  expect(flag().getAttribute('aria-label')).not.toMatch(/\bof\b|%|104/);
  // Hovering or focusing the triangle reads the count and each scope's reasons, the untimed ones
  // apart, in a passive tooltip that holds no control.
  fireEvent.focus(flag());
  const gist = await screen.findByRole('tooltip');
  expect(gist.textContent).toMatch(/^38 unresolved observations, including untimed: cannot be/);
  expect(gist.textContent).toContain('In the last 7d: claude · 20 hook summary events');
  expect(gist.textContent).toContain(
    `Untimed, in no selected range: codex · ${SYNTHETIC_UNTIMED} untimed observations`,
  );
  expect(gist.textContent).not.toMatch(/\d+ of \d+|%/);
  expect(within(gist).queryByRole('button')).toBeNull();
  fireEvent.blur(flag());
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
  expect(flag().getAttribute('aria-label')).toBe(
    'Unresolved attribution: 3 unresolved observations, including untimed',
  );
  expect(flag().getAttribute('aria-label')).not.toContain('5');
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
  expect(context('shell')).toBe('skill codex · future.app');
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
  for (const [range, calls] of [
    ['14d', '84 calls'],
    ['30d', '168 calls'],
  ] as const) {
    fireEvent.click(screen.getByRole('radio', { name: range }));
    await waitFor(() =>
      expect(row('Bash').querySelector('.xt-env-calls')!.textContent).toBe(calls),
    );
    expect(firstDay()).toBe('Aug 25: 0');
    expect(within(row('Bash')).getAllByRole('img')).toHaveLength(14);
    // The calls header follows the selected range; the strip header keeps its fixed scope.
    expect(within(panel()).getByTestId('environment-columns').textContent).toBe(
      `Tool · kind · host14 daysCalls · last ${range}`,
    );
  }
  expect(new Set(source.environment.mock.calls.map(([days]) => days))).toEqual(
    new Set([7, 14, 30]),
  );
  expect(source.environment).toHaveBeenLastCalledWith(30);
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

it('shows loading, then a safe error with retry', async () => {
  let fail = true;
  let release!: () => void;
  const gate = new Promise<void>((done) => (release = done));
  const source = nativeSource(async (days) => {
    await gate;
    if (fail) throw new Error('backend-specific /Users/someone/.claude.json detail');
    return f1(days);
  });
  mount(source);
  await screen.findByRole('heading', { level: 2, name: 'Environment' });
  expect(within(panel()).getByRole('status').textContent).toBe(
    'Reading environment usage for the last 7d…',
  );
  act(() => release());
  const alert = await within(panel()).findByRole('alert');
  expect(alert.textContent).toContain('Environment usage could not be loaded.');
  expect(document.body.textContent).not.toMatch(/backend-specific|\/Users|\.claude\.json/);
  fail = false;
  fireEvent.click(within(alert).getByRole('button', { name: 'Retry environment usage' }));
  expect((await loaded()).textContent).toBe(
    'No non-built-in tool calls were observed in the last 7d or the fixed 14 local days.',
  );
});

it('states an empty observation without inventing zero-call components', async () => {
  mount(nativeSource((days) => f1(days)));
  await loaded();
  expect(within(panel()).getByTestId('environment-empty').textContent).toBe(
    'No non-built-in tool calls were observed in the last 7d or the fixed 14 local days.',
  );
  // With no strip to draw there is no column header; the strips' legend keeps its own line
  // above the dialog controls, its note one dialog away.
  expect(within(panel()).queryByTestId('environment-columns')).toBeNull();
  const legend = within(panel()).getByTestId('environment-strip-legend');
  expect(legend.parentElement!.className).toBe('xt-env-actions');
  expect(legend.textContent).toBe('Activity strips · fixed 14 local days');
  expect(legend.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(legend);
  const strips = await screen.findByRole('dialog', { name: 'Activity strips' });
  expect(strips.textContent).toContain('installed components are unknown');
  fireEvent.keyDown(strips, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(within(panel()).queryByRole('list')).toBeNull();
  // Configured facts are still one step away, and still not a usage claim.
  expect(within(panel()).getByRole('button', { name: 'Configured components · 14' })).toBeTruthy();
  expect(panel().textContent).not.toMatch(inventoryClaims);
});

it('does not read environment usage in a browser preview', async () => {
  const source = { ...nativeSource((days) => f1(days)), kind: 'preview' as const };
  mount(source);
  await screen.findByRole('heading', { level: 2, name: 'Environment' });
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
