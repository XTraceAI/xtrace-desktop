import { readFileSync } from 'node:fs';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { DataSource, SessionListFilter, SessionTitleControls } from '../data/DataSource';
import type { DashboardLaneSession } from '../data/generated/DashboardLaneSession';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionPage } from '../data/generated/SessionPage';
import type { SessionParentLink } from '../data/generated/SessionParentLink';
import type { SessionRow } from '../data/generated/SessionRow';
import type { SessionTitles } from '../data/generated/SessionTitles';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';

/**
 * Verified sub-sessions on Sessions and on the Dashboard's lanes, over
 * synthetic sessions written here (not real history). Both group a child
 * under its exact parent, collapsed; opened, the child is its own row that
 * names its parent inside its own name cell, with its own link and
 * measurements. Search and page membership are the report's, and a parent the
 * report did not return is only named on a row of its own, never added. A
 * known sub-session whose creator is not verified is not listed.
 */

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));

const exported = fixture as FixtureExport;
const PARENT = '01a0aaaa-0000-7000-8000-000000000001';
const CHILD = '01a0bbbb-0000-7000-8000-000000000002';
const OTHER = '01a0cccc-0000-7000-8000-000000000003';
const enc = encodeURIComponent;

const link = (patch: Partial<SessionParentLink> = {}): SessionParentLink => ({
  session_id: PARENT,
  host: 'codex',
  title: null,
  evidence: 'native_spawn',
  ...patch,
});

/** A row with its own output count, so each row's measurement is told apart. */
function row(id: string, output: number, patch: Partial<SessionRow> = {}): SessionRow {
  const template = structuredClone(exported.sessions[0].rows[0]);
  if (template.metrics.state === 'indexed') template.metrics.tokens.counters.output_tokens = output;
  return {
    ...template,
    id,
    host: 'codex',
    title: null,
    repo: `/code/${id.slice(4, 8)}-repo`,
    branch: 'main',
    pr_links: [],
    parent: null,
    // What the native page says of a known child, unless a test says more.
    ...(patch.known_child === true ? { child_check: 'child' as const } : {}),
    ...patch,
  };
}

/** One host-title read the test answers by hand. */
interface Pending {
  ids: readonly string[];
  readId: string;
  resolve: (answer: SessionTitles) => void;
}
function titleControls() {
  const reads: Pending[] = [];
  const cancelled: string[] = [];
  const controls: SessionTitleControls = {
    read: (ids, readId) => new Promise((resolve) => reads.push({ ids: [...ids], readId, resolve })),
    cancel: async (readId) => {
      cancelled.push(readId);
    },
  };
  return { controls, reads, cancelled };
}
/** Every identity any title read named, as often as it was named. */
const readIds = (reads: readonly Pending[]) => reads.flatMap((read) => read.ids);
const answer = (read: Pending, titles: Record<string, string>) =>
  act(async () => {
    read.resolve({ titles: Object.entries(titles).map(([id, title]) => ({ id, title })) });
  });

const unavailable = async (): Promise<never> => {
  throw new Error('Not part of this test.');
};

/**
 * A source whose list filters and pages the way the store's own query does:
 * each filtered page is exactly the rows it was given, in order, and the
 * exact-row read finds any session by its whole identity, listed or not.
 */
function source({
  pages = [[]],
  everyRow = pages.flat(),
  dashboard,
  titles,
  list,
}: {
  pages?: SessionRow[][];
  everyRow?: SessionRow[];
  dashboard?: (days: number) => DashboardMetrics;
  titles?: SessionTitleControls;
  /** A captured page, answered as it was, for the filter it was captured for. */
  list?: (filter: SessionListFilter) => SessionPage | undefined;
}) {
  const listed: { filter: SessionListFilter; after: string | null; days: number }[] = [];
  const opened: { id: string; days: number }[] = [];
  const data: DataSource = {
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
    sessionsList: async (filter, after, days): Promise<SessionPage> => {
      listed.push({ filter, after, days });
      const captured = list?.(filter);
      if (captured) return structuredClone(captured);
      const index = after ? Number(after) : 0;
      const matches = (candidate: SessionRow) =>
        (!filter.hosts || filter.hosts.includes(candidate.host)) &&
        [candidate.id, candidate.repo, candidate.branch, candidate.title].some((value) =>
          value?.toLowerCase().includes(filter.search.toLowerCase()),
        );
      const rows = pages[index].filter(matches);
      // As the native page does: context only for each parent a row names
      // that this page does not list, from that parent's own row.
      const named = new Map(
        rows.flatMap((candidate) =>
          candidate.parent && !rows.some((other) => other.id === candidate.parent!.session_id)
            ? [[candidate.parent.session_id, candidate.parent.host] as const]
            : [],
        ),
      );
      return {
        window: exported.sessions.find((page) => page.window.days === days)!.window,
        rows,
        next: index + 1 < pages.length ? String(index + 1) : null,
        // A verified parent is an indexed session, so the native page
        // always carries its context: from its own row when one is given,
        // else an ordinary checked main session's.
        referenced_parents: [...named].map(([id, host]) => {
          const found = everyRow.find(
            (candidate) => candidate.id === id && candidate.host === host,
          );
          return found
            ? {
                session_id: found.id,
                host: found.host,
                known_child: found.known_child ?? false,
                parent: found.parent ?? null,
                child_check: found.child_check ?? null,
              }
            : {
                session_id: id,
                host,
                known_child: false,
                parent: null,
                child_check: 'checked' as const,
              };
        }),
      };
    },
    dashboard: async (days) => {
      if (dashboard) return dashboard(days);
      const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
      // Sessions parent tests name only their loaded table rows. Dashboard
      // tests below supply explicit lanes for the activity cases.
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
    sessionRow: async (id, days) => {
      opened.push({ id, days });
      return everyRow.find((candidate) => candidate.id === id) ?? null;
    },
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    pullRequests: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    subscribe: async () => () => {},
    ...(titles ? { titles } : {}),
  };
  return { data, listed, opened };
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

/** An IntersectionObserver the test drives: lane rows are in view when it says. */
class FakeObserver {
  static all: FakeObserver[] = [];
  readonly targets = new Set<Element>();
  constructor(private readonly callback: IntersectionObserverCallback) {
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
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

const parentLabel = (name: string) => `Sub-session of ${name}, open parent session ${PARENT}`;
const markers = () => screen.queryAllByRole('link', { name: /^Sub-session of / });
const sessionsTable = () => screen.getByRole('table', { name: 'Indexed sessions' });
const findTable = () => screen.findByRole('table', { name: 'Indexed sessions' });
/** Each listed row's own identity link and its own output cell, in order. */
const listed = () =>
  within(sessionsTable())
    .queryAllByRole('link', { name: /^Open session / })
    .map((open) => {
      const cells = within(open.closest<HTMLElement>('[role="row"]')!).getAllByRole('cell');
      return [open.getAttribute('aria-label'), cells.at(-1)!.textContent];
    });

const groupButton = (name: string) => within(sessionsTable()).getByRole('button', { name });
/** Each drawn All sessions row's name cell, as read: a session's link, or a group's words. */
const drawnRows = () =>
  within(sessionsTable())
    .getAllByRole('row')
    .slice(1)
    .map((row) => {
      const name = row.querySelector<HTMLElement>('.xt-session-name')!;
      return name.dataset.group === 'absent'
        ? name.querySelector('.xt-session-heading')!.textContent
        : name.querySelector('.xt-session-open')!.textContent;
    });
/** The All sessions caption's counts, before what it says of order and range. */
const caption = () => screen.getByText(/^\d+ loaded/).textContent!.split(' · sorted')[0];

it('folds a child under its loaded parent’s row, collapsed, and lists it as its own row when opened', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link({ title: 'Saved parent' }), known_child: true });
  const parent = row(PARENT, 999, { title: 'Saved parent' });
  const { data } = source({ pages: [[child, parent]], titles: controls });
  mount(data, '/sessions?q=01a0&host=codex&range=14d');

  // Collapsed: the parent's own row, with its own measurement, where its
  // newest loaded member (the child) was. The child has no row or marker.
  await screen.findByRole('link', { name: `Open session Saved parent, ${PARENT}` });
  expect(listed()).toEqual([['Open session Saved parent, ' + PARENT, '999']]);
  expect(markers()).toHaveLength(0);
  const toggle = groupButton('1 loaded sub-session of Saved parent');
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  // The loaded count is still every row the page returned.
  expect(caption()).toBe('2 loaded · 1 sub-session grouped under main sessions');

  // Only drawn rows are read for titles: the collapsed child is not.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([PARENT]);
  await answer(reads[0], { [PARENT]: 'Parent host title' });
  expect(toggle.getAttribute('aria-label')).toBe('1 loaded sub-session of Parent host title');

  // Opened: the child's own row, link and measurement, under its parent.
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(listed()).toEqual([
    [`Open session Parent host title, ${PARENT}`, '999'],
    [`Open session ${CHILD}`, '111'],
  ]);
  const marker = screen.getByRole('link', { name: parentLabel('Parent host title') });
  expect(marker.textContent).toBe('↳Sub-session of Parent host title');
  // The exact parent route, carrying this list's own address.
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}?q=01a0&host=codex&range=14d`);
  const open = screen.getByRole('link', { name: `Open session ${CHILD}` });
  expect(open.getAttribute('href')).toBe(`/sessions/${enc(CHILD)}?q=01a0&host=codex&range=14d`);
  expect(open.closest<HTMLElement>('.xt-session-name')!.style.getPropertyValue('--depth')).toBe(
    '1',
  );
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual([CHILD]);
  await answer(reads[1], { [CHILD]: 'Child host title' });
  expect(listed()[1]).toEqual([`Open session Child host title, ${CHILD}`, '111']);

  // The child's own details open apart from its group, and the group closes
  // apart from the details.
  fireEvent.click(within(sessionsTable()).getByRole('button', { name: `Expand ${CHILD}` }));
  expect(screen.getByText('First recorded')).toBeTruthy();
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(toggle);
  expect(listed()).toEqual([[`Open session Parent host title, ${PARENT}`, '999']]);
  expect(screen.queryByText('First recorded')).toBeNull();
});

it('names a filtered-out parent on a row of its own, reads its title once and never adds it', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link({ title: 'Saved parent' }), known_child: true });
  const parent = row(PARENT, 999, { title: 'Saved parent', repo: '/code/elsewhere' });
  const { data, listed: calls } = source({ pages: [[child, parent]], titles: controls });
  mount(data, `/sessions?q=${CHILD}`);

  // The search keeps the child only. Its parent is named on one collapsed row
  // that opens the parent's own page; the child does not become a main row.
  const toggle = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Saved parent',
  });
  const header = toggle.closest<HTMLElement>('[role="row"]')!;
  expect(header.querySelector('[data-group="absent"]')!.textContent).toMatch(
    /^›1Sub-sessions ofSaved parentMain session not loaded here/,
  );
  const open = within(header).getByRole('link', {
    name: `Open parent session Saved parent, ${PARENT}`,
  });
  expect(open.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}?q=${enc(CHILD)}`);
  // No measurement, start, PR, live state, compaction or details of its own.
  const cells = within(header).getAllByRole('cell');
  expect(cells.slice(3).map((cell) => cell.textContent)).toEqual(Array(8).fill(''));
  expect(within(header).queryByRole('button', { name: /^Expand/ })).toBeNull();
  expect(header.querySelector('[data-visible-id], [data-live-status]')).toBeNull();
  expect(listed()).toEqual([]);
  expect(caption()).toBe('1 loaded · 1 sub-session grouped under main sessions');
  // Every read is the search the address holds; the parent is not searched for.
  expect(new Set(calls.map((call) => call.filter.search))).toEqual(new Set([CHILD]));

  // The row naming the parent is drawn, so its title is read; the collapsed
  // child's is not. Until it answers, the saved title stands.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([PARENT]);
  await answer(reads[0], { [PARENT]: 'Parent host title' });
  // The heading, its disclosure and its link all say the same name.
  expect(toggle.getAttribute('aria-label')).toBe('1 loaded sub-session of Parent host title');
  expect(header.querySelector('[data-group="absent"]')!.textContent).toMatch(
    /^›1Sub-sessions ofParent host titleMain session not loaded here/,
  );
  expect(open.getAttribute('aria-label')).toBe(`Open parent session Parent host title, ${PARENT}`);
  expect(open.getAttribute('title')).toBe(`Sub-sessions of Parent host title · ${PARENT}`);
  expect(listed()).toEqual([]);

  fireEvent.click(toggle);
  expect(listed()).toEqual([[`Open session ${CHILD}`, '111']]);
  // Its child's marker names it as its row does.
  const marker = screen.getByRole('link', { name: parentLabel('Parent host title') });
  expect(marker.textContent).toBe('↳Sub-session of Parent host title');
  expect(marker.getAttribute('title')).toBe(`Sub-session of Parent host title · ${PARENT}`);
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}?q=${enc(CHILD)}`);
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual([CHILD]);
  // An answer naming a row it was not asked about is not used.
  await answer(reads[1], { [PARENT]: 'Unasked parent title' });
  expect(screen.queryByText('Unasked parent title')).toBeNull();
  expect(marker.getAttribute('aria-label')).toBe(parentLabel('Parent host title'));
  // Closing and opening the group asks for nothing again.
  fireEvent.click(toggle);
  fireEvent.click(toggle);
  await act(async () => {});
  expect(readIds(reads)).toEqual([PARENT, CHILD]);
});

it('keeps a parent’s saved title, or its identity, when its read finds none', async () => {
  for (const [saved, name] of [
    ['Saved parent', 'Saved parent'],
    [null, 'Session 01a0aaaa'],
  ] as const) {
    const { controls, reads } = titleControls();
    const child = row(CHILD, 111, { parent: link({ title: saved }), known_child: true });
    const { data } = source({ pages: [[child]], titles: controls });
    const view = mount(data, `/sessions?q=${CHILD}`);
    const toggle = await within(await findTable()).findByRole('button', {
      name: `1 loaded sub-session of ${name}`,
    });
    await waitFor(() => expect(reads).toHaveLength(1));
    expect(reads[0].ids).toEqual([PARENT]);
    // Found nothing: the name is what it was, never drawn from what was said.
    await answer(reads[0], {});
    expect(toggle.getAttribute('aria-label')).toBe(`1 loaded sub-session of ${name}`);
    fireEvent.click(toggle);
    expect(screen.getByRole('link', { name: parentLabel(name) })).toBeTruthy();
    view.unmount();
  }
});

it('names an off-page parent in All sessions by the title Recent read for it, read once', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link(), known_child: true });
  const { data } = source({
    pages: [[child, row(OTHER, 5)]],
    titles: controls,
    // The parent's own recent activity, and its child's.
    dashboard: (days) =>
      lanesReport(days, [{ id: PARENT }, { id: CHILD, context: { parent: link() } }]),
  });
  mount(data, `/sessions?q=${CHILD}&range=7d`);
  const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
  const toggle = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Session 01a0aaaa',
  });
  // Recent's own row for the parent is read; the group row naming it in All
  // sessions asks for nothing more.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([PARENT]);
  await answer(reads[0], { [PARENT]: 'OSS xtrace desktop' });
  expect(
    within(recent).getByRole('link', {
      name: `Open recent Codex session OSS xtrace desktop, ${PARENT}`,
    }),
  ).toBeTruthy();
  expect(toggle.getAttribute('aria-label')).toBe('1 loaded sub-session of OSS xtrace desktop');
  expect(
    within(sessionsTable()).getByRole('link', {
      name: `Open parent session OSS xtrace desktop, ${PARENT}`,
    }),
  ).toBeTruthy();
  fireEvent.click(toggle);
  expect(
    within(sessionsTable()).getByRole('link', { name: parentLabel('OSS xtrace desktop') }),
  ).toBeTruthy();
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual([CHILD]);
  await answer(reads[1], {});
  // Recent's group for the child names the same parent the same way.
  fireEvent.click(
    within(recent).getByRole('button', { name: '1 recent sub-session of OSS xtrace desktop' }),
  );
  expect(within(recent).getByRole('link', { name: new RegExp(`session ${CHILD}$`) })).toBeTruthy();
  await act(async () => {});
  expect(readIds(reads).filter((id) => id === PARENT)).toHaveLength(1);
});

it('names a parent only Recent’s group row names, and in the All sessions group too', async () => {
  const { controls, reads } = titleControls();
  const { data } = source({
    pages: [[row(CHILD, 111, { parent: link(), known_child: true })]],
    titles: controls,
    // No recent activity for the parent: Recent only names it.
    dashboard: (days) => lanesReport(days, [{ id: CHILD, context: { parent: link() } }]),
  });
  mount(data, `/sessions?q=${CHILD}&range=7d`);
  const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
  await within(recent).findByRole('button', { name: '1 recent sub-session of Session 01a0aaaa' });
  await waitFor(() => expect(reads).toHaveLength(1));
  // Both lists' group rows name one parent: it is asked about once.
  expect(reads[0].ids).toEqual([PARENT]);
  await answer(reads[0], { [PARENT]: 'Parent host title' });
  const heading = within(recent)
    .getByRole('button', { name: '1 recent sub-session of Parent host title' })
    .closest('li')!;
  expect(
    within(heading).getByRole('link', { name: `Open parent session Parent host title, ${PARENT}` }),
  ).toBeTruthy();
  expect(
    within(sessionsTable()).getByRole('button', {
      name: '1 loaded sub-session of Parent host title',
    }),
  ).toBeTruthy();
});

it('never names a parent by the title read for a session with its identity on another host', async () => {
  const { controls, reads } = titleControls();
  const claudeParent = link({ host: 'claude', title: 'Claude parent' });
  const { data } = source({
    pages: [[row(CHILD, 111, { parent: claudeParent, known_child: true })]],
    titles: controls,
    // A Codex session with the parent's identity has recent activity of its own.
    dashboard: (days) => lanesReport(days, [{ id: PARENT }]),
  });
  mount(data, `/sessions?q=${CHILD}&range=7d`);
  const toggle = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Claude parent',
  });
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([PARENT]);
  await answer(reads[0], { [PARENT]: 'Codex host title' });
  const recent = screen.getByRole('region', { name: 'Recent indexed activity' });
  expect(
    within(recent).getByRole('link', {
      name: `Open recent Codex session Codex host title, ${PARENT}`,
    }),
  ).toBeTruthy();
  // The Claude parent keeps its own saved name everywhere it is named.
  expect(toggle.getAttribute('aria-label')).toBe('1 loaded sub-session of Claude parent');
  fireEvent.click(toggle);
  expect(
    within(sessionsTable()).getByRole('link', {
      name: `Sub-session of Claude parent, open parent session ${PARENT}`,
    }),
  ).toBeTruthy();
  expect(within(sessionsTable()).queryByText(/Codex host title/)).toBeNull();
});

it('never names a loaded parent by the title read for a session with its identity on another host', async () => {
  const { controls, reads } = titleControls();
  const claudeParent = link({ host: 'claude', title: 'Claude link title' });
  const { data } = source({
    // All sessions loads the Claude parent, collapsed under its own parent.
    pages: [
      [
        row(OTHER, 1, { host: 'claude', title: 'Claude ancestor' }),
        row(PARENT, 2, {
          host: 'claude',
          title: 'Claude row title',
          known_child: true,
          parent: link({ session_id: OTHER, host: 'claude' }),
        }),
      ],
    ],
    titles: controls,
    // Recent draws a Codex session with the parent's identity, and a group
    // naming the Claude parent, which has no activity here.
    dashboard: (days) => {
      const report = lanesReport(days, [
        { id: PARENT },
        { id: CHILD, context: { known_child: true, parent: claudeParent } },
      ]);
      report.lane_sessions.push({
        session_id: PARENT,
        host: 'claude',
        repo: null,
        branch: null,
        title: 'Claude link title',
        automated_review: false,
        started_at_ms: null,
        pr_links: 0,
        inferred_pr_links: 0,
        cost: null,
        parent: null,
        known_child: false,
        child_check: 'checked',
      });
      return report;
    },
  });
  mount(data, '/sessions?range=7d');
  const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
  const group = await within(recent).findByRole('button', {
    name: '1 recent sub-session of Claude row title',
  });
  expect(drawnRows()).toEqual(['Claude ancestor']);
  // Recent's Codex row names the identity in the one read; the group naming
  // the Claude parent asks for nothing more.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, PARENT]);
  await answer(reads[0], { [PARENT]: 'Codex host title' });
  expect(
    within(recent).getByRole('link', {
      name: `Open recent Codex session Codex host title, ${PARENT}`,
    }),
  ).toBeTruthy();
  // The loaded Claude parent keeps its own row's saved title everywhere.
  expect(group.getAttribute('aria-label')).toBe('1 recent sub-session of Claude row title');
  expect(
    within(recent).getByRole('link', { name: `Open parent session Claude row title, ${PARENT}` }),
  ).toBeTruthy();
  // Opened, the group lists its child under the same name.
  fireEvent.click(group);
  expect(within(recent).getByRole('link', { name: new RegExp(`session ${CHILD}$`) })).toBeTruthy();
  expect(group.getAttribute('aria-label')).toBe('1 recent sub-session of Claude row title');
  expect(within(recent).queryAllByText(/Codex host title/)).toHaveLength(1);
});

it('cancels a parent’s title read when the list changes and never shows its late answer', async () => {
  const { controls, reads, cancelled } = titleControls();
  const child = row(CHILD, 111, { parent: link(), known_child: true });
  const { data } = source({ pages: [[child, row(OTHER, 5)]], titles: controls });
  mount(data, `/sessions?q=${CHILD}`);
  await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Session 01a0aaaa',
  });
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([PARENT]);
  // Another search before it answers: a new list, with no group row.
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: OTHER } });
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(cancelled).toEqual([reads[0].readId]);
  expect(reads[1].ids).toEqual([OTHER]);
  await answer(reads[0], { [PARENT]: 'Stale parent title' });
  expect(screen.queryByText(/Stale parent title/)).toBeNull();
  // Back to the first search: the stale answer was not kept for it either.
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: CHILD } });
  const toggle = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Session 01a0aaaa',
  });
  await waitFor(() => expect(reads).toHaveLength(3));
  expect(reads[2].ids).toEqual([PARENT]);
  expect(toggle.getAttribute('aria-label')).toBe('1 loaded sub-session of Session 01a0aaaa');
});

it('replaces the row naming a later page’s parent with the parent’s own row, keeping the group open', async () => {
  const { controls, reads } = titleControls();
  const child = row(CHILD, 111, { parent: link(), known_child: true });
  const parent = row(PARENT, 999);
  const { data, listed: calls } = source({
    pages: [[child, row(OTHER, 5)], [parent]],
    titles: controls,
  });
  mount(data, '/sessions');

  // No saved title anywhere: the parent's short identity, on the row that
  // names it, where its child was.
  const toggle = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Session 01a0aaaa',
  });
  expect(toggle.closest('[role="row"]')!.querySelector('[data-group="absent"]')).toBeTruthy();
  expect(listed()).toEqual([[`Open session ${OTHER}`, '5']]);
  // The row naming the parent is read after the session rows; it finds none.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, PARENT]);
  await answer(reads[0], {});
  expect(toggle.getAttribute('aria-label')).toBe('1 loaded sub-session of Session 01a0aaaa');
  fireEvent.click(toggle);
  expect(listed()).toEqual([
    [`Open session ${CHILD}`, '111'],
    [`Open session ${OTHER}`, '5'],
  ]);
  const marker = screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') });
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}`);
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual([CHILD]);
  await answer(reads[1], {});

  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  // The parent arrives on its own page: its own row takes the place of the
  // row that named it, once, and the group is still open. Every row keeps its
  // own measurement.
  await waitFor(() => expect(listed()).toHaveLength(3));
  expect(listed()).toEqual([
    [`Open session ${PARENT}`, '999'],
    [`Open session ${CHILD}`, '111'],
    [`Open session ${OTHER}`, '5'],
  ]);
  expect(document.querySelector('[data-group="absent"]')).toBeNull();
  expect(
    groupButton('1 loaded sub-session of Session 01a0aaaa').getAttribute('aria-expanded'),
  ).toBe('true');
  expect(caption()).toBe('3 loaded · 1 sub-session grouped under main sessions');
  // Only the list's own pages were read, the next one by its cursor.
  expect(new Set(calls.map((call) => call.after))).toEqual(new Set([null, '1']));
  expect(new Set(calls.map((call) => call.filter.search))).toEqual(new Set(['']));
  // Its own row is read at its own version, and then names it in the child's
  // marker too.
  await waitFor(() => expect(reads).toHaveLength(3));
  expect(reads[2].ids).toEqual([PARENT]);
  await answer(reads[2], { [PARENT]: 'Parent host title' });
  expect(screen.getByRole('link', { name: parentLabel('Parent host title') })).toBe(marker);
});

it('leaves a row with no verified parent ordinary', async () => {
  const unset = row(OTHER, 5);
  delete unset.parent;
  const { data } = source({
    pages: [
      [
        row(CHILD, 111, { parent: null }),
        unset,
        // Never shown as its own parent, whatever a report says.
        row(PARENT, 999, { parent: link() }),
      ],
    ],
  });
  mount(data, '/sessions');
  await screen.findByRole('link', { name: `Open session ${PARENT}` });
  expect(markers()).toHaveLength(0);
  expect(screen.queryByText(/Sub-session/)).toBeNull();
  expect(document.querySelector('.xt-session-line')).toBeNull();
});

it('groups a Claude session a Codex parent launched exactly as any other verified child', async () => {
  const CLAUDE_CHILD = '0c000000-0000-4000-8000-0000000000c1';
  const child = row(CLAUDE_CHILD, 111, {
    host: 'claude',
    parent: link({ title: 'Saved parent', evidence: 'agent_launch' }),
  });
  const { data } = source({ pages: [[child, row(PARENT, 999, { title: 'Saved parent' })]] });
  mount(data, '/sessions');

  fireEvent.click(
    await within(await findTable()).findByRole('button', {
      name: '1 loaded sub-session of Saved parent',
    }),
  );
  const marker = screen.getByRole('link', { name: parentLabel('Saved parent') });
  expect(marker.textContent).toBe('↳Sub-session of Saved parent');
  expect(marker.getAttribute('href')).toBe(`/sessions/${enc(PARENT)}`);
  // The kind of proof adds no role, task or launch wording to the row.
  expect(marker.closest('[role="row"]')!.textContent).not.toMatch(/launch|review|role|task/i);
  expect(listed()).toEqual([
    ['Open session Saved parent, ' + PARENT, '999'],
    [`Open session ${CLAUDE_CHILD}`, '111'],
  ]);
  expect(markers()).toHaveLength(1);
});

it('opens the exact parent from the keyboard and comes back to the same list', async () => {
  const child = row(CHILD, 111, { parent: link() });
  const parent = row(PARENT, 999, { repo: '/code/parent-repo', branch: 'trunk' });
  const { data, opened } = source({ pages: [[child]], everyRow: [child, parent] });
  mount(data, '/sessions?q=01a0bbbb&host=codex&range=30d');

  // The parent is outside the search: its row names it, and the child is one
  // disclosure away.
  const group = async () =>
    within(await findTable()).findByRole('button', {
      name: '1 loaded sub-session of Session 01a0aaaa',
    });
  fireEvent.click(await group());
  const open = screen.getByRole('link', { name: `Open session ${CHILD}` });
  const marker = screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') });
  // A real link in the tab order, right after the row's own.
  expect(open.compareDocumentPosition(marker) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(marker.tabIndex).toBe(0);
  marker.focus();
  expect(document.activeElement).toBe(marker);
  fireEvent.click(marker);

  await screen.findByRole('heading', { name: 'parent-repo · trunk' });
  await waitFor(() => expect(opened).toEqual([{ id: PARENT, days: 30 }]));
  const back = screen.getByRole('link', { name: '← All sessions' });
  expect(back.getAttribute('href')).toBe('/sessions?q=01a0bbbb&host=codex&range=30d');
  fireEvent.click(back);
  // The same list, its group collapsed again; opened, the same child.
  const again = await group();
  expect(again.getAttribute('aria-expanded')).toBe('false');
  expect((screen.getByLabelText('Search sessions') as HTMLInputElement).value).toBe('01a0bbbb');
  fireEvent.click(again);
  expect(screen.getByRole('link', { name: `Open session ${CHILD}` })).toBeTruthy();
  expect(screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') })).toBeTruthy();
});

/** A lane report of these sessions, newest first, with this context. */
/** A lane session's cost, every response priced. */
const laneCost = (total: number) => ({
  total_usd: total,
  priced_subtotal_usd: total,
  selected_observations: 1,
  priced_observations: 1,
  unpriced_observations: 0,
  assumed_tier_observations: 0,
  unpriced: [],
});

function lanesReport(
  days: number,
  lanes: { id: string; context?: Partial<DashboardLaneSession> | null }[],
  cap?: { total: number },
): DashboardMetrics {
  const report = structuredClone(exported.dashboards.find((r) => r.window.days === days)!);
  const minute = 60_000;
  report.lanes = lanes.map(({ id }, index) => ({
    session_id: id,
    host: 'codex',
    start_ms: report.lane_end_ms - (index + 1) * 30 * minute,
    end_ms: report.lane_end_ms - (index + 1) * 30 * minute + 10 * minute,
  }));
  report.lanes_total = cap?.total ?? lanes.length;
  report.lanes_truncated = cap !== undefined;
  report.lane_sessions = lanes
    .filter(({ context }) => context !== null)
    .map(({ id, context }) => ({
      session_id: id,
      host: 'codex',
      repo: `/code/${id.slice(4, 8)}-repo`,
      branch: 'main',
      title: null,
      started_at_ms: null,
      pr_links: 0,
      inferred_pr_links: 0,
      cost: laneCost(id === CHILD ? 1.11 : 9.99),
      parent: null,
      // Checked, or a known child: what the native report says once the
      // display check finished. A test that means otherwise says so.
      child_check: context?.known_child === true ? ('child' as const) : ('checked' as const),
      ...context,
      automated_review: context?.automated_review ?? false,
    }));
  // As the native report does: a context-only entry for each verified parent
  // the lanes name that has no lane here, an ordinary checked main session's
  // unless a test says otherwise ([`withReferenced`]).
  for (const session of [...report.lane_sessions]) {
    const parent = session.parent;
    if (
      parent &&
      parent.session_id !== session.session_id &&
      !report.lane_sessions.some((entry) => entry.session_id === parent.session_id)
    )
      report.lane_sessions.push({
        session_id: parent.session_id,
        host: parent.host,
        repo: null,
        branch: null,
        title: parent.title,
        automated_review: false,
        started_at_ms: null,
        pr_links: 0,
        inferred_pr_links: 0,
        cost: null,
        parent: null,
        known_child: false,
        child_check: 'checked',
      });
  }
  report.lane_sessions.sort((a, b) => a.session_id.localeCompare(b.session_id));
  return report;
}

const laneTable = () => screen.getByRole('table', { name: 'Session lanes' });
/** Each lane's own name link and cost, in order. */
const laneRows = () =>
  within(laneTable())
    .getAllByRole('link', { name: /^Open session / })
    .map((open) => {
      const cells = within(open.closest<HTMLElement>('[role="row"]')!).getAllByRole('cell');
      return [open.textContent, cells.at(-1)!.textContent];
    });

const SECOND = '01a0dddd-0000-7000-8000-000000000004';
const groupToggle = (name: string) => within(laneTable()).getByRole('button', { name });
/** Every drawn body row: its text, and whether it has a lane track of its own. */
const drawn = () =>
  within(laneTable())
    .getAllByRole('row')
    .slice(1)
    .map((row) => row.querySelector('.xt-lane-name')!.firstChild!.textContent);

it('collapses a returned parent’s children under its own row and lists each as it was when opened', async () => {
  const lanes = (parent: SessionParentLink | null) => [
    { id: CHILD, context: { parent } },
    { id: OTHER },
    { id: PARENT, context: { title: 'Saved parent lane' } },
    { id: SECOND, context: { parent, cost: laneCost(2.22) } },
  ];
  // The same capped report without the relation, for comparison.
  const plain = source({ dashboard: (days) => lanesReport(days, lanes(null), { total: 40 }) });
  const first = mount(plain.data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  const before = laneRows();
  first.unmount();
  expect(before).toEqual([
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0cccc', '$9.99'],
    ['Saved parent lane', '$9.99'],
    ['Session 01a0dddd', '$2.22'],
  ]);

  const { controls, reads } = titleControls();
  const { data } = source({
    dashboard: (days) => lanesReport(days, lanes(link()), { total: 40 }),
    titles: controls,
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // Collapsed: the parent keeps its own row and link, placed where its newest
  // returned child was, and its cost is the whole group's: its own $9.99 and
  // its children's $1.11 and $2.22, the rows opening it lists. Neither child
  // has a link in the page.
  expect(laneRows()).toEqual([
    ['Saved parent lane', 'Σ $13.32'],
    ['Session 01a0cccc', '$9.99'],
  ]);
  expect(markers()).toHaveLength(0);
  expect(within(laneTable()).queryByRole('link', { name: new RegExp(CHILD) })).toBeNull();
  const toggle = groupToggle('2 sub-sessions of Saved parent lane');
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  // Its children are rows of the same table, not one element it could name.
  expect(toggle.hasAttribute('aria-controls')).toBe(false);
  const parentRow = toggle.closest<HTMLElement>('[role="row"]')!;
  expect(parentRow.querySelector('.sr-only')!.textContent).toContain(
    '2 sub-sessions collapsed under this row. Total with its sub-sessions: $13.32 API-equivalent cost of 3 responses.',
  );
  const groupCell = within(parentRow).getAllByRole('cell').at(-1)!;
  expect(groupCell.querySelector('.xt-metric-cell')!.getAttribute('title')).toBe(
    'Total for these 3 sessions: $13.32 at public API prices.',
  );
  // No caption under the rows states the counts or where a group sits.
  expect(screen.queryByTestId('lanes-disclosure')).toBeNull();
  // Titles are read only for the rows drawn.
  FakeObserver.all.at(-1)!.show([CHILD, OTHER, PARENT, SECOND]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, PARENT]);
  await answer(reads[0], { [PARENT]: 'Parent host title' });
  expect(toggle.getAttribute('aria-label')).toBe('2 sub-sessions of Parent host title');

  // Opened: each row its own whole cost — the parent's own $9.99 again — and
  // the rows the group lists add up to the collapsed $13.32.
  toggle.focus();
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(document.activeElement).toBe(toggle);
  expect(laneRows()).toEqual([
    ['Parent host title', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0dddd', '$2.22'],
    ['Session 01a0cccc', '$9.99'],
  ]);
  expect(markers()).toHaveLength(2);
  for (const marker of markers())
    expect(marker.getAttribute('aria-label')).toBe(parentLabel('Parent host title'));
  expect(screen.getByRole('link', { name: `Open session ${SECOND}` }).getAttribute('href')).toBe(
    `/sessions/${enc(SECOND)}?q=${enc(SECOND)}&host=codex&range=7d`,
  );
  const childName = screen
    .getByRole('link', { name: `Open session ${CHILD}` })
    .closest<HTMLElement>('.xt-lane-name')!;
  expect(childName.style.getPropertyValue('--depth')).toBe('1');
  // The lane table's sized rows follow what is drawn.
  const lanesBox = document.querySelector<HTMLElement>('.xt-lanes')!;
  expect(lanesBox.style.getPropertyValue('--count')).toBe('4');

  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(markers()).toHaveLength(0);
  expect(lanesBox.style.getPropertyValue('--count')).toBe('2');
});

const OLD = '01a0f001-0000-7000-8000-000000000011';
const OLDER = '01a0f002-0000-7000-8000-000000000012';
/** A sub-session with no span in the lane window, as the report carries it. */
const olderSub = (id: string, parent: string, usd: number): DashboardLaneSession => ({
  session_id: id,
  host: 'codex',
  repo: '/code/old-repo',
  branch: 'main',
  title: null,
  automated_review: false,
  started_at_ms: null,
  pr_links: 0,
  inferred_pr_links: 0,
  cost: laneCost(usd),
  parent: link({ session_id: parent }),
  known_child: true,
  child_check: 'child',
});

it('lists a group’s older sub-sessions too, so its Σ total is the rows it opens to', async () => {
  const report =
    (notShown: number | null, cutShort: boolean | null = null) =>
    (days: number) => {
      const out = lanesReport(days, [
        { id: CHILD, context: { parent: link() } },
        {
          id: PARENT,
          context: {
            title: 'Saved parent lane',
            sub_sessions_not_shown: notShown,
            sub_sessions_cut_short: cutShort,
          },
        },
      ]);
      out.lane_sub_sessions = [
        // The grandchild comes before its own parent: placement must not
        // depend on the report's order.
        olderSub(OLDER, OLD, 0.5),
        olderSub(OLD, PARENT, 2),
        // Its parent is not listed, so it is never placed, nor a row of its own.
        olderSub(OTHER, '01a0dead-0000-7000-8000-000000000000', 7),
      ];
      return out;
    };
  const first = mount(source({ dashboard: report(null) }).data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // Collapsed: $9.99 + $1.11 + $2.00 + $0.50, marked as a total; the count
  // is of the rows it opens to, the older sub-session included.
  expect(laneRows()).toEqual([['Saved parent lane', 'Σ $13.60']]);
  const toggle = groupToggle('2 sub-sessions of Saved parent lane');
  fireEvent.click(toggle);
  // Open: each row its own whole cost, and the older sub-session's own group
  // its total, so the cost cells in view still add up to $13.60.
  expect(laneRows()).toEqual([
    ['Saved parent lane', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0f001', 'Σ $2.50'],
  ]);
  const old = screen
    .getByRole('link', { name: `Open session ${OLD}` })
    .closest<HTMLElement>('[role="row"]')!;
  // No activity in the window: an empty track, and the row says so.
  expect(old.querySelector('.xt-lane-track')).not.toBeNull();
  expect(old.querySelector('.xt-lane-span')).toBeNull();
  expect(old.querySelector('.sr-only')!.textContent).toContain('no activity in the last 48 hours');
  fireEvent.click(groupToggle('1 sub-session of Session 01a0f001'));
  expect(laneRows()).toEqual([
    ['Saved parent lane', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0f001', '$2.00'],
    ['Session 01a0f002', '$0.50'],
  ]);
  expect(screen.queryByRole('link', { name: `Open session ${OTHER}` })).toBeNull();
  first.unmount();

  // Sub-sessions the report left out keep the total a floor, and say so.
  const costTitle = () =>
    within(laneTable())
      .getAllByRole('row')[1]
      .querySelector<HTMLElement>('[role="cell"]:last-child .xt-metric-cell')!
      .getAttribute('title');
  const second = mount(source({ dashboard: report(4) }).data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(laneRows()).toEqual([['Saved parent lane', 'Σ $13.60+']]);
  expect(costTitle()).toBe(
    'Total for these 4 sessions: at least $13.60: 4 more sub-sessions not shown, not included.',
  );
  // Open, the parent row shows its own cost, and still says what is left out.
  fireEvent.click(groupToggle('2 sub-sessions of Saved parent lane'));
  expect(laneRows()[0]).toEqual(['Saved parent lane', '$9.99']);
  expect(costTitle()).toBe(
    '$9.99 at public API prices. 4 more sub-sessions under this session are not shown or counted.',
  );
  second.unmount();

  // A walk the report cut short makes every count it gives a floor.
  const third = mount(source({ dashboard: report(4, true) }).data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(costTitle()).toBe(
    'Total for these 4 sessions: at least $13.60: at least 4 more sub-sessions not shown, not included.',
  );
  third.unmount();
  mount(source({ dashboard: report(null, true) }).data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(laneRows()).toEqual([['Saved parent lane', 'Σ $13.60+']]);
  expect(costTitle()).toContain('more sub-sessions may not be shown, not included');
});

it('says what is left out under a listed session with no sub-session in view', async () => {
  mount(
    source({
      dashboard: (days) =>
        lanesReport(days, [{ id: PARENT, context: { sub_sessions_not_shown: 1 } }]),
    }).data,
    '/dashboard',
  );
  await screen.findByRole('table', { name: 'Session lanes' });
  const row = within(laneTable()).getAllByRole('row')[1];
  expect(within(row).queryByRole('button')).toBeNull();
  expect(row.querySelector('[role="cell"]:last-child .xt-metric-cell')!.getAttribute('title')).toBe(
    '$9.99 at public API prices. 1 more sub-session under this session is not shown or counted.',
  );
  expect(row.querySelector('.sr-only')!.textContent).toContain(
    '1 more sub-session under this session is not shown or counted.',
  );
});

it('names an absent parent on one collapsed row with blank cells, reads nothing for it and adds no lane', async () => {
  const { controls, reads } = titleControls();
  const parent = link({ title: 'Saved parent' });
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [
        { id: OTHER },
        { id: CHILD, context: { parent } },
        { id: SECOND, context: { parent, cost: laneCost(2.22) } },
      ]),
    titles: controls,
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // The group sits at its newest returned child, after the newer ordinary row.
  expect(drawn()).toEqual(['Session 01a0cccc', '›2']);
  const toggle = groupToggle('2 sub-sessions of Saved parent');
  const row = toggle.closest<HTMLElement>('[role="row"]')!;
  expect(row.querySelector('.xt-lane-name')!.textContent).toMatch(
    /^›2Sub-sessions of Saved parent/,
  );
  // It opens the parent's own page, and claims no measurement, start or span.
  const open = within(row).getByRole('link', {
    name: `Open parent session Saved parent, ${PARENT}`,
  });
  expect(open.getAttribute('href')).toBe(
    `/sessions/${enc(PARENT)}?q=${enc(PARENT)}&host=codex&range=7d`,
  );
  // Collapsed, its only number is its sub-sessions' cost added up.
  const cells = within(row).getAllByRole('cell');
  expect(cells.slice(2).map((cell) => cell.textContent)).toEqual(['', '', '', '', 'Σ $3.33']);
  expect(row.querySelector('.xt-lane-track, .xt-lane-span, time')).toBeNull();
  expect(row.querySelector('.sr-only')!.textContent).toContain(
    `which is not listed here: it is only named, with no numbers of its own. Its sub-sessions active in the last 48 hours are listed, with their own sub-sessions. 2 sub-sessions collapsed under this row. Total: $3.33 API-equivalent cost of 2 responses.`,
  );
  expect(laneRows()).toEqual([['Session 01a0cccc', '$9.99']]);

  fireEvent.click(toggle);
  // Open, the named row has no cost of its own; its sub-sessions show theirs.
  expect(within(row).getAllByRole('cell').at(-1)!.textContent).toBe('');
  expect(laneRows()).toEqual([
    ['Session 01a0cccc', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0dddd', '$2.22'],
  ]);
  expect(markers().map((marker) => marker.getAttribute('aria-label'))).toEqual([
    parentLabel('Saved parent'),
    parentLabel('Saved parent'),
  ]);
  // The absent parent is never read, drawn or observed.
  FakeObserver.all.at(-1)!.show([CHILD, SECOND, OTHER, PARENT]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, CHILD, SECOND]);
});

it('leaves a lane with no verified parent ordinary, and one not known to be checked out', async () => {
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [
        { id: CHILD, context: { parent: null } },
        { id: PARENT, context: { parent: link() } },
        // No context, a context without its display check, and one still
        // being checked: none is known to be shown.
        { id: OTHER, context: null },
        { id: SECOND, context: { child_check: null } },
        { id: THIRD, context: { child_check: 'checking' } },
      ]),
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(laneRows()).toHaveLength(2);
  absentFrom(laneTable(), OTHER, SECOND, THIRD);
  expect(markers()).toHaveLength(0);
  expect(document.querySelector('.xt-lane-name[data-subsession]')).toBeNull();
  // Nothing is grouped, so nothing is behind a disclosure.
  expect(within(laneTable()).queryByRole('button', { name: /sub-session/ })).toBeNull();
});

it('opens an absent Dashboard parent from the keyboard with a Back that finds it', async () => {
  const parent = row(PARENT, 999, { repo: '/code/parent-repo', branch: 'trunk' });
  const { data, opened } = source({
    everyRow: [parent],
    dashboard: (days) => lanesReport(days, [{ id: CHILD, context: { parent: link() } }]),
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  const open = await waitFor(() => {
    const found = screen.getByRole('link', {
      name: `Open parent session Session 01a0aaaa, ${PARENT}`,
    });
    expect(found.getAttribute('href')).toContain('range=14d');
    return found;
  });
  // The one child is behind its disclosure; opened, its marker says the same.
  fireEvent.click(groupToggle('1 sub-session of Session 01a0aaaa'));
  expect(screen.getByRole('link', { name: parentLabel('Session 01a0aaaa') })).toBeTruthy();
  open.focus();
  expect(document.activeElement).toBe(open);
  fireEvent.click(open);
  await screen.findByRole('heading', { name: 'parent-repo · trunk' });
  await waitFor(() => expect(opened).toEqual([{ id: PARENT, days: 14 }]));
  expect(screen.getByRole('link', { name: '← All sessions' }).getAttribute('href')).toBe(
    `/sessions?q=${enc(PARENT)}&host=codex&range=14d`,
  );
});

const THIRD = '01a0eeee-0000-7000-8000-000000000005';

/** Every live-state, compaction and title read the Dashboard asks for. */
function spyReads(data: DataSource) {
  const live: string[][] = [];
  const compactions: string[][] = [];
  Object.assign(data, {
    liveSessions: {
      read: async (ids: readonly string[], viewId: string | null) => {
        live.push([...ids]);
        return {
          view_id: viewId ?? 'view',
          states: ids.map((id) => ({ id, status: 'unknown' })),
        };
      },
      release: async () => {},
    },
    compactions: {
      read: async (ids: readonly string[]) => {
        compactions.push([...ids]);
        return { counts: [] };
      },
      cancel: async () => {},
    },
  });
  return { live, compactions };
}

/** The report with context-only entries for parents the lanes do not name. */
function withReferenced(
  report: DashboardMetrics,
  referenced: Partial<DashboardLaneSession> & { session_id: string },
): DashboardMetrics {
  report.lane_sessions = report.lane_sessions.filter(
    (entry) => entry.session_id !== referenced.session_id,
  );
  report.lane_sessions.push({
    host: 'codex',
    repo: null,
    branch: null,
    title: 'Saved referenced title',
    automated_review: false,
    started_at_ms: null,
    pr_links: 0,
    inferred_pr_links: 0,
    cost: null,
    parent: null,
    child_check: referenced.known_child === true ? 'child' : 'checked',
    ...referenced,
  });
  report.lane_sessions.sort((a, b) => a.session_id.localeCompare(b.session_id));
  return report;
}

/** No trace of these sessions anywhere in the page: row, link, label or text. */
function absentFromPage(...ids: string[]) {
  for (const id of ids) {
    expect(document.body.innerHTML).not.toContain(id);
    expect(document.body.innerHTML).not.toContain(id.slice(0, 8));
  }
}

it('lists nothing for a known sub-session with no verified parent, nor its returned children, and reads nothing for them', async () => {
  const { controls, reads } = titleControls();
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [
        // The newest: a known child whose parent is unknown or ambiguous.
        { id: CHILD, context: { known_child: true, parent: null } },
        // Its own returned child, which would otherwise name it as a parent.
        { id: SECOND, context: { known_child: true, parent: link({ session_id: CHILD }) } },
        // An ordinary untitled session with no parent, and one from an older
        // report without the bit.
        { id: OTHER, context: { known_child: false } },
        { id: PARENT },
      ]),
    titles: controls,
  });
  const { live, compactions } = spyReads(data);
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(drawn()).toEqual(['Session 01a0cccc', 'Session 01a0aaaa']);
  expect(laneRows()).toEqual([
    ['Session 01a0cccc', '$9.99'],
    ['Session 01a0aaaa', '$9.99'],
  ]);
  expect(within(laneTable()).queryByRole('button', { name: /sub-session/ })).toBeNull();
  expect(markers()).toHaveLength(0);
  absentFromPage(CHILD, SECOND);
  // Neither is observed, so no title, live state or compactions are read.
  FakeObserver.all.at(-1)!.show([CHILD, SECOND, OTHER, PARENT]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, PARENT]);
  await waitFor(() => expect(live.flat()).toContain(OTHER));
  await waitFor(() => expect(compactions.flat()).toContain(OTHER));
  for (const asked of [live.flat(), compactions.flat()]) {
    expect(asked).not.toContain(CHILD);
    expect(asked).not.toContain(SECOND);
  }
});

it('lists nothing under an unreturned parent that is a known sub-session with no verified parent', async () => {
  const lanes = [
    { id: OTHER },
    { id: CHILD, context: { known_child: true, parent: link({ title: 'Saved parent' }) } },
    { id: SECOND, context: { known_child: true, parent: link({ session_id: CHILD }) } },
    { id: THIRD, context: { known_child: true, parent: link({ title: 'Saved parent' }) } },
  ];
  const { controls, reads } = titleControls();
  const { data } = source({
    dashboard: (days) =>
      withReferenced(lanesReport(days, lanes), {
        session_id: PARENT,
        known_child: true,
        parent: null,
      }),
    titles: controls,
  });
  const { live, compactions } = spyReads(data);
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // No placeholder for the parent, and none of its returned branch.
  expect(drawn()).toEqual(['Session 01a0cccc']);
  expect(within(laneTable()).queryByRole('button', { name: /sub-session/ })).toBeNull();
  absentFromPage(PARENT, CHILD, SECOND, THIRD);
  expect(screen.queryByText(/Saved parent/)).toBeNull();
  FakeObserver.all.at(-1)!.show([OTHER, PARENT, CHILD, SECOND, THIRD]);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER]);
  await waitFor(() => expect(compactions.flat()).toContain(OTHER));
  for (const asked of [live.flat(), compactions.flat()])
    for (const id of [PARENT, CHILD, SECOND, THIRD]) expect(asked).not.toContain(id);
});

it('names an ordinary unreturned parent exactly as before when the report carries its context', async () => {
  const parent = link({ title: 'Saved parent' });
  const lanes = [
    { id: OTHER },
    { id: CHILD, context: { known_child: true, parent } },
    { id: SECOND, context: { known_child: true, parent, cost: laneCost(2.22) } },
  ];
  // The same report, without and with the parent's context-only entry: a
  // parent that is not itself an unresolved sub-session, including one that
  // is a known sub-session with a verified parent of its own.
  const page = (report: (days: number) => DashboardMetrics) => {
    const { data } = source({ dashboard: report });
    const view = mount(data, '/dashboard');
    return screen.findByRole('table', { name: 'Session lanes' }).then(() => {
      fireEvent.click(groupToggle('2 sub-sessions of Saved parent'));
      const rows = within(laneTable())
        .getAllByRole('row')
        .map((row) => [row.textContent, row.querySelector('.sr-only')?.textContent]);
      view.unmount();
      return rows;
    });
  };
  const before = await page((days) => lanesReport(days, lanes));
  for (const context of [
    { known_child: false, parent: null },
    { known_child: true, parent: link({ session_id: THIRD, title: 'Grandparent' }) },
  ])
    expect(
      await page((days) =>
        withReferenced(lanesReport(days, lanes), {
          session_id: PARENT,
          cost: null,
          ...context,
        }),
      ),
    ).toEqual(before);
  // The header, the ordinary row, the parent's named row and its two children.
  expect(before).toHaveLength(5);
  expect(before[2][0]).toMatch(/^›2Sub-sessions of Saved parent/);
  expect(before[2][1]).toContain('it is only named, with no numbers of its own');
});

it('keeps a known sub-session with a verified returned parent in its group, and ordinary rows listed', async () => {
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [
        { id: CHILD, context: { known_child: true, parent: link() } },
        { id: OTHER, context: { known_child: null } },
        { id: PARENT, context: { known_child: false, title: 'Saved parent lane' } },
        { id: SECOND, context: { known_child: false, title: null } },
      ]),
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  // Collapsed, the group's cost adds its one sub-session's $1.11.
  expect(laneRows()).toEqual([
    ['Saved parent lane', 'Σ $11.10'],
    ['Session 01a0cccc', '$9.99'],
    ['Session 01a0dddd', '$9.99'],
  ]);
  fireEvent.click(groupToggle('1 sub-session of Saved parent lane'));
  expect(laneRows()).toEqual([
    ['Saved parent lane', '$9.99'],
    ['Session 01a0bbbb', '$1.11'],
    ['Session 01a0cccc', '$9.99'],
    ['Session 01a0dddd', '$9.99'],
  ]);
  expect(markers()).toHaveLength(1);
});

it('says there is no session to list when every returned session is left out', async () => {
  const { data } = source({
    dashboard: (days) =>
      lanesReport(days, [{ id: CHILD, context: { known_child: true, parent: null } }]),
  });
  mount(data, '/dashboard');
  await screen.findByRole('table', { name: 'Session lanes' });
  expect(within(laneTable()).getByText(/^No session to list in the last 48 hours/)).toBeTruthy();
  absentFromPage(CHILD);
});

/** No trace of these sessions in this part of the page. */
function absentFrom(element: HTMLElement, ...ids: string[]) {
  for (const id of ids) {
    expect(element.innerHTML).not.toContain(id);
    expect(element.innerHTML).not.toContain(id.slice(0, 8));
  }
}
const FOURTH = '01a0ffff-0000-7000-8000-000000000006';
const MIDDLE = '01a01111-0000-7000-8000-000000000007';
const GRAND = '01a02222-0000-7000-8000-000000000008';

it('hides a sub-session whose main session is unknown from both Sessions lists, even when searched for', async () => {
  const { controls, reads } = titleControls();
  const { data } = source({
    pages: [[row(CHILD, 111, { known_child: true, parent: null }), row(OTHER, 999)]],
    titles: controls,
    dashboard: (days) =>
      lanesReport(days, [
        { id: CHILD, context: { known_child: true, parent: null } },
        { id: OTHER },
      ]),
  });
  const { live, compactions } = spyReads(data);
  mount(data, `/sessions?q=${CHILD.slice(0, 8)}&range=7d`);
  // The search matches the child only; it is loaded, counted and not listed.
  await within(await findTable()).findByText(
    'No session to list: every loaded session is a sub-session whose main session is unknown.',
  );
  expect(caption()).toBe('1 loaded · 1 sub-session hidden: main session unknown');
  const recent = screen.getByRole('region', { name: 'Recent indexed activity' });
  await within(recent).findByRole('link', { name: new RegExp(`session ${OTHER}$`) });
  absentFrom(sessionsTable(), CHILD);
  // Its links carry the search the address holds, so only its whole ID is absent.
  expect(recent.innerHTML).not.toContain(CHILD);
  // Nothing is read for it: no title, live state or compactions.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER]);
  await waitFor(() => expect(compactions.flat()).toContain(OTHER));
  for (const asked of [live.flat(), compactions.flat()]) expect(asked).not.toContain(CHILD);
});

it('hides each loaded branch under an unknown main session, across pages, and keeps ordinary rows', async () => {
  const { controls, reads } = titleControls();
  const { data } = source({
    pages: [
      [
        // A known sub-session whose parent is unknown, and its own child.
        row(CHILD, 1, { known_child: true, parent: null }),
        row(SECOND, 2, { known_child: true, parent: link({ session_id: CHILD }) }),
        // A child whose parent, on the next page, is itself such a session.
        row(THIRD, 3, { known_child: true, parent: link() }),
        // An ordinary main session with no title.
        row(OTHER, 4, { known_child: false }),
        // A child whose parent, on the next page, has a verified parent.
        row(FOURTH, 5, {
          known_child: true,
          parent: link({ session_id: MIDDLE, title: 'Middle' }),
        }),
      ],
      [
        row(PARENT, 6, { known_child: true, parent: null }),
        row(MIDDLE, 7, {
          title: 'Middle',
          known_child: true,
          parent: link({ session_id: GRAND, title: 'Grand' }),
        }),
      ],
    ],
    titles: controls,
  });
  const { live, compactions } = spyReads(data);
  mount(data, '/sessions');
  const middle = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Middle',
  });
  // The ordinary untitled session, then the row naming the off-page parent
  // whose own parent is verified. No row names the unknown branch's parents.
  expect(drawnRows()).toEqual([`Session ${OTHER.slice(0, 8)}`, '›1Sub-sessions ofMiddle']);
  expect(listed()).toEqual([[`Open session ${OTHER}`, '4']]);
  expect(caption()).toBe(
    '5 loaded · 1 sub-session grouped under main sessions · 3 sub-sessions hidden: main session unknown',
  );
  absentFrom(sessionsTable(), CHILD, SECOND, THIRD, PARENT);
  FakeObserver.all.at(-1)!.show([CHILD, SECOND, THIRD, OTHER, FOURTH, PARENT, MIDDLE]);
  // The ordinary row, then the parent the drawn group row names.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([OTHER, MIDDLE]);
  fireEvent.click(middle);
  expect(listed()).toEqual([
    [`Open session ${OTHER}`, '4'],
    [`Open session ${FOURTH}`, '5'],
  ]);

  // The next page: the unknown branch's parent is hidden too; the verified
  // parent arrives under the row naming its own parent, its group still open.
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  const grand = await within(await findTable()).findByRole('button', {
    name: '1 loaded sub-session of Grand',
  });
  expect(caption()).toBe(
    '7 loaded · 2 sub-sessions grouped under main sessions · 4 sub-sessions hidden: main session unknown',
  );
  fireEvent.click(grand);
  expect(listed()).toEqual([
    [`Open session ${OTHER}`, '4'],
    [`Open session Middle, ${MIDDLE}`, '7'],
    [`Open session ${FOURTH}`, '5'],
  ]);
  expect(groupButton('1 loaded sub-session of Middle').getAttribute('aria-expanded')).toBe('true');
  absentFrom(sessionsTable(), CHILD, SECOND, THIRD, PARENT);
  FakeObserver.all.at(-1)!.show([CHILD, SECOND, THIRD, OTHER, FOURTH, PARENT, MIDDLE]);
  await waitFor(() => expect(compactions.flat()).toContain(OTHER));
  for (const asked of [readIds(reads), live.flat(), compactions.flat()])
    for (const id of [CHILD, SECOND, THIRD, PARENT]) expect(asked).not.toContain(id);
  // The parent the new group row names is only ever asked for its title.
  await waitFor(() => expect(readIds(reads)).toContain(GRAND));
  for (const asked of [live.flat(), compactions.flat()]) expect(asked).not.toContain(GRAND);
});

it('groups by exact host: a parent named on another host is not the loaded row with its identity', async () => {
  const { data } = source({
    pages: [
      [
        row(CHILD, 1, {
          known_child: true,
          parent: link({ host: 'claude', title: 'Claude parent' }),
        }),
        row(PARENT, 2, { title: 'Codex session' }),
        // A known sub-session with no parent, hidden, and a child naming the
        // same identity on another host, which is not it.
        row(OTHER, 3, { known_child: true, parent: null }),
        row(SECOND, 4, {
          known_child: true,
          parent: link({ session_id: OTHER, host: 'claude', title: 'Claude other' }),
        }),
      ],
    ],
  });
  mount(data, '/sessions');
  await within(await findTable()).findByText('Codex session');
  // Neither child is grouped under the Codex row with its parent's identity.
  // The page carries no context for a parent on another host whose identity
  // a listed row has, so, not known to be shown, each child is left out.
  expect(drawnRows()).toEqual(['Codex session']);
  expect(within(sessionsTable()).queryByRole('button', { name: /of Codex session$/ })).toBeNull();
  expect(caption()).toBe('4 loaded · 3 sub-sessions hidden: main session unknown');
});

it('limits recent activity to eight main sessions and groups after grouping and hiding', async () => {
  const roots = Array.from(
    { length: 9 },
    (_, index) => `01a1000${index}-0000-7000-8000-00000000000${index}`,
  );
  const KID = '01a2bbbb-0000-7000-8000-000000000011';
  const LATE = '01a2cccc-0000-7000-8000-000000000012';
  const HIDDEN = '01a2dddd-0000-7000-8000-000000000013';
  const { controls, reads } = titleControls();
  const { data } = source({
    titles: controls,
    dashboard: (days) =>
      lanesReport(days, [
        { id: HIDDEN, context: { known_child: true, parent: null } },
        { id: KID, context: { known_child: true, parent: link({ session_id: roots[0] }) } },
        { id: roots[0], context: { title: 'Root 0' } },
        { id: LATE, context: { known_child: true, parent: link({ title: 'Saved parent' }) } },
        ...roots.slice(1).map((id, index) => ({ id, context: { title: `Root ${index + 1}` } })),
      ]),
  });
  const { live, compactions } = spyReads(data);
  mount(data, '/sessions');
  const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
  await within(recent).findByRole('link', { name: new RegExp(`${roots[0]}$`) });
  // Ten main sessions and groups remain once the unknown branch is hidden and
  // children are grouped; the first eight are listed, each group collapsed.
  expect(
    within(recent)
      .getAllByRole('link')
      .map((link) => link.getAttribute('aria-label')),
  ).toEqual([
    `Open recent Codex session Root 0, ${roots[0]}`,
    `Open parent session Saved parent, ${PARENT}`,
    ...roots.slice(1, 7).map((id, index) => `Open recent Codex session Root ${index + 1}, ${id}`),
  ]);
  expect(
    within(recent).getByText(
      '8 most recent main sessions and groups with activity in the last 48 hours; opening a group also lists its sub-sessions',
    ),
  ).toBeTruthy();
  const header = within(recent)
    .getByRole('button', { name: '1 recent sub-session of Saved parent' })
    .closest('li')!;
  // The report returned no span for it; that is not a claim it had none.
  expect(header.textContent).toContain('Main session: no activity returned here');
  expect(header.querySelector('time, [data-live-status], .xt-compaction')).toBeNull();
  absentFrom(recent, HIDDEN, KID, LATE, roots[7], roots[8]);
  // The listed sessions, then the parent the listed group row names; nothing
  // past the limit.
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads[0].ids).toEqual([roots[0], ...roots.slice(1, 7), PARENT]);
  await answer(reads[0], {});

  // Opening a group adds its child below its parent; the limit still counts
  // main sessions and groups.
  fireEvent.click(within(recent).getByRole('button', { name: '1 recent sub-session of Root 0' }));
  const kid = within(recent).getByRole('link', { name: `Open recent Codex session ${KID}` });
  expect(kid.closest('li')!.dataset.depth).toBe('1');
  expect(within(recent).getAllByRole('link')).toHaveLength(9);
  absentFrom(recent, HIDDEN, roots[7], roots[8]);
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1].ids).toEqual([KID]);
  await waitFor(() => expect(live.flat()).toContain(KID));
  for (const asked of [live.flat(), compactions.flat()])
    for (const id of [HIDDEN, LATE, PARENT, roots[7], roots[8]]) expect(asked).not.toContain(id);
  // A sub-session is not read for compactions; its parent is.
  expect(compactions.flat()).not.toContain(KID);
});

it('reads a filtered-out parent’s context from the fixture export as the native page does', async () => {
  // The fixture's own source over an export whose one page lists a known
  // sub-session with no verified parent, its child, an ordinary session, and
  // a child naming that ordinary session's identity on another host.
  const changed = structuredClone(exported);
  for (const page of changed.sessions)
    page.rows = [
      row(PARENT, 1, { known_child: true, parent: null }),
      row(CHILD, 2, { known_child: true, parent: link() }),
      row(OTHER, 3, { title: 'Codex other' }),
      row(SECOND, 4, {
        known_child: true,
        parent: link({ session_id: OTHER, host: 'claude', title: 'Claude other' }),
      }),
    ];
  const data = new FixtureDataSource(changed);
  const all = { search: '', hosts: null, withPrs: false };
  // Listed together, every parent is a row and none is context.
  expect((await data.sessionsList(all, null, 7)).referenced_parents).toEqual([]);
  // Searched for alone, the child keeps its parent's own exact context.
  expect((await data.sessionsList({ ...all, search: CHILD }, null, 7)).referenced_parents).toEqual([
    { session_id: PARENT, host: 'codex', known_child: true, parent: null, child_check: 'child' },
  ]);
  // A parent on another host is not the row with that identity.
  expect((await data.sessionsList({ ...all, search: SECOND }, null, 7)).referenced_parents).toEqual(
    [],
  );

  const view = mount(data, `/sessions?q=${CHILD}&range=7d`);
  await within(await findTable()).findByText(
    'No session to list: every loaded session is a sub-session whose main session is unknown.',
  );
  expect(caption()).toBe('1 loaded · 1 sub-session hidden: main session unknown');
  expect(sessionsTable().innerHTML).not.toContain(PARENT);
  view.unmount();

  // Its parent has no context in the export: what is not known is not
  // shown, so the child is left out with it.
  mount(data, `/sessions?q=${SECOND}&range=7d`);
  await within(await findTable()).findByText(
    'No session to list: every loaded session is a sub-session whose main session is unknown.',
  );
  expect(caption()).toBe('1 loaded · 1 sub-session hidden: main session unknown');
});

describe('a session whose display check is not finished', () => {
  // One session as its check runs: still checking (or a report without the
  // field), then a child of a loaded parent, or a checked main session.
  const stages = [
    ['checking', { child_check: 'checking', known_child: false, parent: null }],
    ['without a display check', { child_check: undefined, known_child: false, parent: null }],
    ['a child', { child_check: 'child', known_child: true, parent: link() }],
    ['checked', { child_check: 'checked', known_child: false, parent: null }],
  ] as const;
  const data = (patch: (typeof stages)[number][1]) =>
    source({
      pages: [[row(CHILD, 111, { ...patch }), row(PARENT, 999, { title: 'Saved parent' })]],
      dashboard: (days) =>
        lanesReport(days, [
          { id: CHILD, context: { ...patch } },
          { id: PARENT, context: { title: 'Saved parent' } },
        ]),
    }).data;

  it.each(stages)(
    'is %s: never a main row unless checked, in either Sessions list',
    async (stage, patch) => {
      mount(data(patch), '/sessions?range=7d');
      const table = await findTable();
      await within(table).findByRole('link', { name: `Open session Saved parent, ${PARENT}` });
      const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
      await within(recent).findByRole('link', { name: new RegExp(`${PARENT}$`) });
      if (stage === 'checked') {
        expect(drawnRows()).toContain(`Session ${CHILD.slice(0, 8)}`);
        expect(recent.innerHTML).toContain(CHILD);
        expect(caption()).toBe('2 loaded');
      } else if (stage === 'a child') {
        // Collapsed under its verified parent, in both lists.
        const group = groupButton('1 loaded sub-session of Saved parent');
        expect(group.getAttribute('aria-expanded')).toBe('false');
        absentFrom(sessionsTable(), CHILD);
        expect(recent.innerHTML).not.toContain(CHILD);
        expect(caption()).toBe('2 loaded · 1 sub-session grouped under main sessions');
      } else {
        // Loaded and counted, but listed nowhere, and never as a main row.
        absentFrom(sessionsTable(), CHILD);
        expect(recent.innerHTML).not.toContain(CHILD);
        expect(caption()).toBe('2 loaded · 1 session hidden while checking who started it');
      }
    },
  );

  it.each(stages)(
    'is %s: never a main row unless checked, on the Dashboard',
    async (stage, patch) => {
      mount(data(patch), '/dashboard');
      await screen.findByRole('table', { name: 'Session lanes' });
      if (stage === 'checked') {
        expect(laneRows().map(([name]) => name)).toEqual([
          `Session ${CHILD.slice(0, 8)}`,
          'Saved parent',
        ]);
      } else if (stage === 'a child') {
        const toggle = groupToggle('1 sub-session of Saved parent');
        expect(toggle.getAttribute('aria-expanded')).toBe('false');
        absentFrom(laneTable(), CHILD);
      } else {
        expect(laneRows().map(([name]) => name)).toEqual(['Saved parent']);
        absentFrom(laneTable(), CHILD);
      }
    },
  );
});

/**
 * Public snapshots captured from the app's own Sessions and Dashboard
 * responses, after each stage of the native index's work
 * (`cargo run -p xtrace-desktop --example visibility_snapshots`), replayed
 * through the production Dashboard, All sessions and Recent views. A watched
 * session that becomes a sub-session must never be drawn as a row of its own
 * in any frame. `XT_VISIBILITY_FRAMES` names a captured file; without it, a
 * synthetic sequence of the same shape is replayed. `XT_VISIBILITY_CHILDREN`
 * (comma-separated) names the sessions held to that, else every watched one.
 */
describe('replayed public snapshots', () => {
  interface Frames {
    days: number;
    watch: string[];
    frames: {
      at: string;
      responses: { searched: Record<string, SessionPage>; dashboard: DashboardMetrics };
    }[];
  }
  const synthetic = (): Frames => {
    const page = (patch: Partial<SessionRow>, parent: boolean): SessionPage => ({
      ...structuredClone(exported.sessions.find((entry) => entry.window.days === 7)!),
      rows: [row(CHILD, 1, { title: null, ...patch })],
      next: null,
      referenced_parents: parent
        ? [
            {
              session_id: PARENT,
              host: 'codex',
              known_child: false,
              parent: null,
              child_check: 'checked',
            },
          ]
        : [],
    });
    const stage = (patch: Partial<SessionRow>, parent: boolean, at: string) => ({
      at,
      responses: {
        searched: { [CHILD]: page(patch, parent) },
        dashboard: lanesReport(7, [
          { id: CHILD, context: { ...patch } as Partial<DashboardLaneSession> },
          { id: PARENT, context: { title: null } },
        ]),
      },
    });
    return {
      days: 7,
      watch: [CHILD],
      frames: [
        stage({ child_check: 'checking', known_child: false, parent: null }, false, 'indexed'),
        stage({ child_check: undefined, known_child: false, parent: null }, false, 'no field'),
        stage({ child_check: 'child', known_child: true, parent: link() }, true, 'pass 1'),
      ],
    };
  };
  const file = process.env.XT_VISIBILITY_FRAMES;
  const captured: Frames = file ? (JSON.parse(readFileSync(file, 'utf8')) as Frames) : synthetic();
  const children = process.env.XT_VISIBILITY_CHILDREN?.split(',').filter(Boolean) ?? captured.watch;
  const drawnAlone = (container: HTMLElement, id: string) =>
    within(container)
      .queryAllByRole('link')
      .some((link) => {
        const name = link.getAttribute('aria-label') ?? link.textContent ?? '';
        return (
          !name.startsWith('Open parent session') &&
          !name.startsWith('Sub-session of') &&
          (link.getAttribute('href') ?? '').includes(encodeURIComponent(id)) &&
          name.endsWith(id)
        );
      });
  const cases = captured.frames.flatMap((frame, index) =>
    children.map((id) => [`${index} ${frame.at}`, id, frame] as const),
  );

  // The check itself can fail: a checked main session is a row of its own.
  it('finds a checked main session drawn on its own in every view', async () => {
    const main = synthetic().frames[0];
    const patch = { child_check: 'checked' as const, known_child: false, parent: null };
    main.responses.searched[CHILD].rows = [row(CHILD, 1, { title: null, ...patch })];
    main.responses.dashboard = lanesReport(7, [
      { id: CHILD, context: patch },
      { id: PARENT, context: { title: null } },
    ]);
    const { data } = source({
      list: (filter) => (filter.search === CHILD ? main.responses.searched[CHILD] : undefined),
      dashboard: () => structuredClone(main.responses.dashboard),
    });
    const view = mount(data, `/dashboard?range=7d`);
    const lanes = await screen.findByRole('table', { name: 'Session lanes' });
    await waitFor(() => expect(drawnAlone(lanes, CHILD)).toBe(true));
    view.unmount();
    mount(data, `/sessions?q=${CHILD}&range=7d`);
    const table = await findTable();
    await waitFor(() => expect(drawnAlone(table, CHILD)).toBe(true));
    const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
    await waitFor(() => expect(drawnAlone(recent, CHILD)).toBe(true));
  });

  it.each(cases)('frame %s: %s is no main row on the Dashboard', async (_, id, frame) => {
    const { data } = source({ dashboard: () => structuredClone(frame.responses.dashboard) });
    mount(data, `/dashboard?range=${captured.days}d`);
    const table = await screen.findByRole('table', { name: 'Session lanes' });
    expect(drawnAlone(table, id)).toBe(false);
  });

  it.each(cases)('frame %s: %s is no main row in All or Recent', async (_, id, frame) => {
    const { data } = source({
      list: (filter) => (filter.search === id ? frame.responses.searched[id] : undefined),
      dashboard: () => structuredClone(frame.responses.dashboard),
    });
    mount(data, `/sessions?q=${encodeURIComponent(id)}&range=${captured.days}d`);
    const table = await findTable();
    await waitFor(() => expect(screen.getByText(/^\d+ loaded/)).toBeTruthy());
    expect(drawnAlone(table, id)).toBe(false);
    const recent = await screen.findByRole('region', { name: 'Recent indexed activity' });
    expect(drawnAlone(recent, id)).toBe(false);
  });
});
