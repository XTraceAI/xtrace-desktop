import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { PullRequestSessionsRequest } from '../data/DataSource';
import type { PrAnalyticsPage } from '../data/generated/PrAnalyticsPage';
import type { SessionPage } from '../data/generated/SessionPage';
import { events } from '../data/ipc-names';
import { count, tokens as formatTokens } from '../kit/format';
import { agentDuration } from './agent-duration';
import { continuous } from './metric-format';
import {
  scenarios,
  syntheticReport,
  syntheticSessions,
  type SyntheticScenario,
} from './pr-analytics.synthetic';
import {
  controlEvents,
  deferred,
  expectLocalReadsOnly,
  exported,
  mount,
  trappedSource,
} from './PrsPage.harness';

/**
 * The merged-PR report and its drilldown over the Rust-generated synthetic
 * export: every value on screen is a field the app's own commands produced,
 * compared here with that same field, formatted; nothing is recomputed.
 */
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

function analyticsSource(scenario: SyntheticScenario = scenarios.measured) {
  const source = trappedSource(async () => structuredClone(exported.pull_requests));
  const report = vi.fn(async (days: number, confirmedOnly: boolean) =>
    syntheticReport(scenario, days, confirmedOnly),
  );
  const sessions = vi.fn(async (request: PullRequestSessionsRequest, after: string | null) =>
    syntheticSessions(scenario, request, after),
  );
  return Object.assign(source, { pullRequestAnalytics: report, pullRequestSessions: sessions });
}

const TABLE = 'Merged pull requests';
const report7 = () => syntheticReport(scenarios.measured, 7, false);
const loaded = async () =>
  within(await screen.findByRole('table', { name: TABLE })).findAllByText(/example\//);
const tile = (label: string) => screen.getByText(label).closest('.xt-stat-tile') as HTMLElement;
/** The body row naming exactly this pull request. */
const rowOf = (identity: string) =>
  within(screen.getByRole('table', { name: TABLE }))
    .getAllByRole('row')
    .find((row) => within(row).queryByText(identity, { exact: true }))!;
const unmeasured = (element: HTMLElement) =>
  [...element.querySelectorAll('.xt-unmeasured')].map((item) => item.getAttribute('title'));
/** The linked-session count's button; the token total opens the same drilldown. */
const openSessions = (identity: string) =>
  screen.getByRole('button', {
    name: new RegExp(`^\\d+ linked sessions?, .*open the linked sessions of ${identity}$`),
  });

it('shows the report with its measured window, overlap wording and the range control', async () => {
  const source = analyticsSource();
  mount(source, '/prs');
  await loaded();
  expect(source.pullRequestAnalytics.mock.calls).toEqual([[7, false]]);
  expect(screen.getByRole('radio', { name: 'Merged effort' }).getAttribute('aria-checked')).toBe(
    'true',
  );
  expect(screen.getByRole('radiogroup', { name: 'Date range' })).toBeTruthy();
  expect(screen.getByText(/Linked-session effort; may overlap other PRs/)).toBeTruthy();
  expect(screen.getByTestId('prs-scope').textContent).toMatch(
    /^Merged Sep 1\s–\s7, 2026 · all links/,
  );
  // No cost, no authorship, no refresh, no external link.
  expect(document.querySelector('main')!.textContent).not.toMatch(
    /\$|API-equivalent|cost per|authored by|wrote the/i,
  );
  expect(
    within(document.querySelector('main')!).queryByRole('button', { name: /refresh/i }),
  ).toBeNull();
  expect(document.querySelectorAll('main a[href^="http"]')).toHaveLength(0);
  // The inventory is not read by this view.
  expect(source.pullRequests).not.toHaveBeenCalled();
  expectLocalReadsOnly(source);
});

it('labels measured-only medians apart from complete ones, with the report samples', async () => {
  mount(analyticsSource(), '/prs');
  await loaded();
  const { summary } = report7().report;
  // Tokens: one PR unknown, so only the measured-PR median exists.
  expect(summary.tokens.median.median).toBeNull();
  expect(tile('Tokens / PR').textContent).toContain(
    formatTokens(summary.tokens.median.measured_median!),
  );
  expect(tile('Tokens / PR').textContent).toContain(
    `measured PRs · ${summary.tokens.median.measured_prs} of ${summary.tokens.median.eligible_prs}`,
  );
  // Agent time: every PR measured, so the complete median with n = PRs.
  expect(summary.agent_ms.median).not.toBeNull();
  expect(tile('Agent time / PR').textContent).toContain(
    agentDuration(summary.agent_ms.median!).visible,
  );
  expect(tile('Agent time / PR').textContent).toContain(`n = ${summary.agent_ms.eligible_prs} PRs`);
  // Hands-off samples differ from messages: an unknown and a no-stretch PR.
  expect(tile('Hands-off / PR').textContent).toContain(
    `measured PRs · ${summary.hands_off_min.measured_prs} of ${summary.hands_off_min.eligible_prs}`,
  );
  expect(tile('Human msgs / PR').textContent).toContain(
    `${continuous(summary.human_messages.measured_median!)}`,
  );
  expect(summary.hands_off_min.measured_prs).not.toBe(summary.human_messages.measured_prs);
});

it('keeps unknown, measured zero, no sample, unresolved, stale and excluded apart in rows', async () => {
  mount(analyticsSource(), '/prs');
  await loaded();
  const rows = report7().report.rows;
  const table = screen.getByRole('table', { name: TABLE });
  // Every report row, newest merge first: the report's order reversed.
  const identities = within(table)
    .getAllByText(/^example\/[a-z]+#\d+$/)
    .map((cell) => cell.textContent);
  expect(identities).toEqual([...rows].reverse().map((row) => `${row.repository}#${row.number}`));

  // Unknown tokens: an idle member without selected usage. Not zero.
  const idle = rowOf('example/atlas#102');
  const tokens = within(idle).getByRole('button', { name: /no selected usage/ });
  expect(tokens.textContent).toBe('—');
  expect(tokens.getAttribute('title')).toMatch(
    /^Unknown: 1 of 2 linked sessions has no selected usage/,
  );
  // A measured zero token total and a measured zero message count stay zero.
  expect(
    within(rowOf('example/atlas#105')).getByRole('button', { name: /^0 tokens/ }),
  ).toBeTruthy();
  const docs = rowOf('example/harbor#7');
  expect(within(docs).getAllByRole('cell')[4].textContent).toBe('0');
  // No stretch is a known absence, not an unknown.
  expect(unmeasured(docs).join()).toContain('No hands-off stretch in this range, a known absence');
  // Unknown human classification and an unresolved type.
  const untitled = rowOf('example/harbor#9');
  expect(within(untitled).getByText('Title not cached')).toBeTruthy();
  expect(within(untitled).getByText('unresolved')).toBeTruthy();
  expect(unmeasured(untitled).join()).toContain('human or agent origin is unknown');
  expect(unmeasured(untitled).join()).toContain('stretch boundary unknown');
  // Facts kept after a failed refresh are marked stale.
  expect(within(rowOf('example/harbor#8')).getByText('stale').getAttribute('title')).toMatch(
    /last refresh failed \(rate limited\); earlier facts kept/,
  );
  // An excluded surface is named where the row's hands-off leaves it out.
  const desk = rowOf('example/atlas#103');
  expect(within(desk).getByText('excl')).toBeTruthy();
  expect(desk.querySelector('[title*="excluded surface"]')).toBeTruthy();
  // Mixed evidence is disclosed beside the strongest link.
  const one = rowOf('example/atlas#101');
  expect(within(one).getByText('mixed')).toBeTruthy();
  expect(within(one).getByText(/linked sessions: 2 exact · 1 commit/)).toBeTruthy();
  expect(one.textContent).not.toMatch(/NaN|undefined|null/);
});

it('filters rows only: medians, counts and the report read stay the whole report', async () => {
  const source = analyticsSource();
  mount(source, '/prs');
  await loaded();
  const before = tile('Tokens / PR').textContent;
  fireEvent.change(screen.getByRole('searchbox', { name: 'Filter merged pull requests' }), {
    target: { value: 'harbor' },
  });
  expect(
    within(screen.getByRole('table', { name: TABLE })).getAllByText(/^example\/harbor#/),
  ).toHaveLength(3);
  expect(screen.getByText(/3 of 11 PRs shown · medians cover all/)).toBeTruthy();
  expect(tile('Tokens / PR').textContent).toBe(before);
  fireEvent.change(screen.getByRole('searchbox', { name: 'Filter merged pull requests' }), {
    target: { value: 'nothing matches' },
  });
  expect(screen.getByText('No merged pull request matches this filter.')).toBeTruthy();
  expect(source.pullRequestAnalytics).toHaveBeenCalledOnce();
});

it('re-reads the report for Confirmed only and for the range, never mixing their rows', async () => {
  const source = analyticsSource();
  mount(source, '/prs');
  await loaded();
  fireEvent.click(screen.getByRole('switch', { name: 'Confirmed only' }));
  await waitFor(() =>
    expect(source.pullRequestAnalytics.mock.calls).toEqual([
      [7, false],
      [7, true],
    ]),
  );
  const confirmed = syntheticReport(scenarios.measured, 7, true).report;
  const row = confirmed.rows.find(
    (item) => item.repository === 'example/atlas' && item.number === 102,
  )!;
  // Without its inferred idle member the row's tokens are measured.
  await waitFor(() =>
    expect(
      within(rowOf('example/atlas#102')).getByRole('button', {
        name: new RegExp(`^${count(row.tokens.counters.total_tokens!)} tokens`),
      }),
    ).toBeTruthy(),
  );
  expect(screen.getByTestId('prs-scope').textContent).toContain('exact and commit links only');
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(source.pullRequestAnalytics).toHaveBeenLastCalledWith(14, true));
  await waitFor(() => expect(screen.getByText('example/atlas#106')).toBeTruthy());
  // The mode survives switching views.
  fireEvent.click(screen.getByRole('radio', { name: 'Cached inventory' }));
  await screen.findByRole('table', { name: 'Linked pull requests' });
  expect(screen.queryByRole('radiogroup', { name: 'Date range' })).toBeNull();
  fireEvent.click(screen.getByRole('radio', { name: 'Merged effort' }));
  expect(screen.getByRole('switch', { name: 'Confirmed only' }).getAttribute('aria-checked')).toBe(
    'true',
  );
});

it('says no merged PR is known when facts are unknown, withholds medians, and reaches the inventory', async () => {
  const source = analyticsSource(scenarios.sparse);
  mount(source, '/prs');
  await screen.findByText(
    /^No merged pull request is known in this range\. 2 linked pull requests have unknown cached facts, so they may have merged here; 2 are open, closed or merged outside it\.$/,
  );
  for (const label of ['Tokens / PR', 'Human msgs / PR', 'Agent time / PR', 'Hands-off / PR'])
    expect(unmeasured(tile(label))).toEqual([
      'No pull request is known to have merged in this range',
    ]);
  expect(screen.getByTestId('prs-eligibility').textContent).toContain(
    '0 linked PRs merged in range · 2 open, closed or merged outside · 2 with unknown cached facts',
  );
  // Read successfully, not failed: nothing known merged, some facts unknown.
  expect(screen.getByTestId('prs-types').textContent).toBe(
    'No known merged PR to group; 2 linked PRs have unknown cached facts.',
  );
  fireEvent.click(screen.getByRole('button', { name: 'Cached inventory' }));
  await screen.findByRole('table', { name: 'Linked pull requests' });
  expect(source.pullRequests).toHaveBeenCalledOnce();
});

it('withholds token medians when the fixed gate fails, and only them', async () => {
  const source = analyticsSource(scenarios.sparse);
  mount(source, '/prs');
  await screen.findByRole('table', { name: TABLE });
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await screen.findByText('example/harbor#20');
  const report = syntheticReport(scenarios.sparse, 30, false).report;
  expect(report.summary.tokens.withheld).toBe('gate_failed');
  expect(unmeasured(tile('Tokens / PR'))[0]).toMatch(
    /^Withheld: Token\+model coverage over the fixed 14 days Aug 25\s–\sSep 7, 2026: 0 of 2 sessions measured with a model \(0%\); token medians need at least 90%$/,
  );
  // Row totals are not gated, and the other medians stand.
  const row = rowOf('example/harbor#20');
  expect(
    within(row).getByRole('button', {
      name: new RegExp(`^${count(report.rows[0].tokens.counters.total_tokens!)} tokens`),
    }),
  ).toBeTruthy();
  expect(unmeasured(tile('Agent time / PR'))).toEqual([]);
});

it('keeps a pending, a failed and a recovered report apart in tiles, type panel and table', async () => {
  const source = analyticsSource();
  const first = deferred<PrAnalyticsPage>();
  source.pullRequestAnalytics.mockImplementationOnce(() => first.promise);
  mount(source, '/prs');
  // Pending: every companion surface says it is reading.
  await screen.findByRole('table', { name: TABLE });
  for (const label of ['Tokens / PR', 'Human msgs / PR', 'Agent time / PR', 'Hands-off / PR'])
    expect(unmeasured(tile(label))).toEqual(['Reading the report…']);
  expect(screen.getByTestId('prs-types').textContent).toBe('Reading the report…');
  // Failed: never read as no data, and nothing claims to be loading.
  await act(async () => first.reject(new Error('adapter detail')));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('The pull-request report could not be read.');
  expect(alert.textContent).not.toContain('adapter detail');
  const failed = 'Not available: the report could not be read';
  for (const label of ['Tokens / PR', 'Human msgs / PR', 'Agent time / PR', 'Hands-off / PR'])
    expect(unmeasured(tile(label))).toEqual([failed]);
  expect(screen.getByTestId('prs-types').textContent).toBe(`${failed}.`);
  expect(screen.queryByText(/No merged pull request/)).toBeNull();
  expect(screen.queryByTestId('prs-eligibility')).toBeNull();
  // Recovered on Retry: the report's own values everywhere.
  fireEvent.click(within(alert).getByRole('button', { name: 'Retry' }));
  await loaded();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(unmeasured(tile('Agent time / PR'))).toEqual([]);
  expect(screen.getByRole('list', { name: 'Per-PR medians by work type' })).toBeTruthy();
  expect(screen.getByTestId('prs-eligibility')).toBeTruthy();
  expect(source.pullRequestAnalytics).toHaveBeenCalledTimes(2);
  expectLocalReadsOnly(source);
});

it('discloses each type metric’s own sample, and a published median’s n is its measured PRs', async () => {
  mount(analyticsSource(), '/prs');
  await loaded();
  const types = syntheticReport(scenarios.measured, 7, false).report.by_type;
  const summary = (type: string | null) => types.find((group) => group.work_type === type)!.summary;
  // `test`: hands-off published over one of its two PRs; the other has no stretch.
  expect(summary('test').hands_off_min).toMatchObject({
    eligible_prs: 2,
    measured_prs: 1,
    no_sample_prs: 1,
    unknown_prs: 0,
  });
  expect(summary('test').hands_off_min.median).not.toBeNull();
  const item = (label: string) =>
    within(screen.getByRole('list', { name: 'Per-PR medians by work type' }))
      .getAllByRole('listitem')
      .find((entry) => entry.querySelector('.xt-pr-type')!.textContent === label)!;
  const handsOff = within(item('test')).getByTestId('type-hands-off');
  expect(handsOff.textContent).toContain(' 1/2');
  expect(handsOff.textContent).toContain('n = 1 of 2 merged PRs: 1 with no stretch');
  expect(within(item('test')).getByTestId('type-msgs').textContent).not.toContain('/');
  // `unresolved`: messages unknown, agent time measured, hands-off unknown.
  const unresolved = item('unresolved');
  expect(within(unresolved).getByTestId('type-msgs').textContent).toContain(
    'of 1 merged PR, 1 unknown',
  );
  expect(within(unresolved).getByTestId('type-agent').textContent).not.toContain('/');
});

it('opens exact linked members over the report’s pinned window, and closes back to the page', async () => {
  const source = analyticsSource();
  mount(source, '/prs');
  await loaded();
  fireEvent.click(screen.getByRole('switch', { name: 'Confirmed only' }));
  fireEvent.click(screen.getByRole('switch', { name: 'Confirmed only' }));
  fireEvent.change(screen.getByRole('searchbox', { name: 'Filter merged pull requests' }), {
    target: { value: 'atlas' },
  });
  // Back to all links is the cached report already read: no third read.
  await waitFor(() => expect(source.pullRequestAnalytics).toHaveBeenCalledTimes(2));
  const trigger = openSessions('example/atlas#101');
  fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  const page = report7();
  expect(source.pullRequestSessions.mock.calls).toEqual([
    [
      {
        repository: 'example/atlas',
        number: 101,
        confirmedOnly: false,
        windowDays: 7,
        windowEndMs: page.window.end_ms,
      },
      null,
    ],
  ]);
  const members = await within(dialog).findAllByText(/^Session s-/);
  expect(members.map((item) => item.textContent).sort()).toEqual([
    'Session s-alpha',
    'Session s-beta',
    'Session s-shared',
  ]);
  // The selected pull request's own link evidence, per member.
  const beta = within(dialog).getByText('Session s-beta').closest('[role="row"]') as HTMLElement;
  expect(within(beta).getByText('commit')).toBeTruthy();
  expect(within(dialog).getByRole('note').textContent).toContain(
    'Linked-session effort; sessions may contribute to other PRs.',
  );
  expect(within(dialog).getByText(/1 link to a non-user session, not measured/)).toBeTruthy();
  expect(
    within(dialog).getByText('3 of 3 linked sessions shown · Sessions list order'),
  ).toBeTruthy();
  // No transcript or session read happens for the drilldown.
  expectLocalReadsOnly(source);
  // Close by keyboard: focus returns to the count, the filter and mode stay.
  fireEvent.keyDown(document.activeElement ?? dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(
    (screen.getByRole('searchbox', { name: 'Filter merged pull requests' }) as HTMLInputElement)
      .value,
  ).toBe('atlas');
  expect(screen.getByRole('switch', { name: 'Confirmed only' }).getAttribute('aria-checked')).toBe(
    'false',
  );
});

it('reaches the members from an unknown token total and lists the idle one', async () => {
  const source = analyticsSource();
  mount(source, '/prs');
  await loaded();
  fireEvent.click(
    within(rowOf('example/atlas#102')).getByRole('button', { name: /no selected usage/ }),
  );
  const dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  const idle = (await within(dialog).findByText('Session s-idle')).closest(
    '[role="row"]',
  ) as HTMLElement;
  expect(within(idle).getByText('idle')).toBeTruthy();
  expect(within(idle).getByText('inferred')).toBeTruthy();
  expect(unmeasured(idle)).toContain('No selected usage in this window');
});

it('pages past 50 members with the returned cursor, over the same window', async () => {
  const source = analyticsSource();
  mount(source, '/prs');
  await loaded();
  fireEvent.click(openSessions('example/atlas#104'));
  const dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  await within(dialog).findByText('50 of 53 linked sessions shown · Sessions list order');
  const first = source.pullRequestSessions.mock.results[0].value as Promise<SessionPage>;
  const cursor = (await first).next;
  expect(cursor).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Show 50 more' }));
  await within(dialog).findByText('53 of 53 linked sessions shown · Sessions list order');
  expect(source.pullRequestSessions.mock.calls[1]).toEqual([
    source.pullRequestSessions.mock.calls[0][0],
    cursor,
  ]);
  const ids = within(dialog)
    .getAllByText(/^Session /)
    .map((item) => item.textContent);
  expect(new Set(ids).size).toBe(53);
  expect(within(dialog).queryByRole('button', { name: 'Show 50 more' })).toBeNull();
});

it('never paints a previous pull request’s late answer under another’s heading', async () => {
  const source = analyticsSource();
  const answers: {
    request: PullRequestSessionsRequest;
    done: ReturnType<typeof deferred<SessionPage>>;
  }[] = [];
  source.pullRequestSessions.mockImplementation((request) => {
    const done = deferred<SessionPage>();
    answers.push({ request, done });
    return done.promise;
  });
  mount(source, '/prs');
  await loaded();
  fireEvent.click(openSessions('example/atlas#101'));
  let dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(openSessions('example/harbor#7'));
  dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  expect(within(dialog).getByText(/example\/harbor#7/)).toBeTruthy();
  // The second answer lands first, then the first one late.
  await act(async () => {
    answers[1].done.resolve(syntheticSessions(scenarios.measured, answers[1].request, null));
  });
  await within(dialog).findByText('Session s-tools');
  await act(async () => {
    answers[0].done.resolve(syntheticSessions(scenarios.measured, answers[0].request, null));
  });
  expect(within(dialog).queryByText('Session s-alpha')).toBeNull();
  expect(
    within(dialog)
      .getAllByText(/^Session /)
      .map((item) => item.textContent),
  ).toEqual(['Session s-tools']);
});

it('suppresses an open drilldown’s report totals and members while the report is unavailable', async () => {
  const source = analyticsSource();
  const control = controlEvents(source);
  let reads = 0;
  source.pullRequestAnalytics.mockImplementation(async (days, confirmedOnly) => {
    // The first read succeeds; the background re-read after the commit fails.
    if (reads++ === 1) throw new Error('adapter detail');
    return syntheticReport(scenarios.measured, days, confirmedOnly);
  });
  mount(source, '/prs');
  control.resolveAll();
  await loaded();
  const trigger = openSessions('example/atlas#101');
  fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  await within(dialog).findByText('Session s-alpha');
  expect(within(dialog).getByText(/^3 linked sessions · 3 active/)).toBeTruthy();
  const members = source.pullRequestSessions.mock.calls.length;

  // A committed import re-reads the report, and that read fails.
  control.emit(events.importReceived);
  await waitFor(() => expect(source.pullRequestAnalytics).toHaveBeenCalledTimes(2), {
    timeout: 3000,
  });
  // The drilldown stays open on its pull request, but claims nothing of the
  // report that is no longer shown: no totals, no member rows, no paging.
  await within(dialog).findByText(
    /^The report could not be read, so the linked sessions of example\/atlas#101 are not shown\./,
  );
  expect(within(dialog).queryByText(/^3 linked sessions · 3 active/)).toBeNull();
  expect(within(dialog).queryByText(/2\.06K tokens/)).toBeNull();
  expect(within(dialog).queryByText('Session s-alpha')).toBeNull();
  expect(within(dialog).queryByRole('table')).toBeNull();
  expect(within(dialog).queryByRole('button', { name: 'Show 50 more' })).toBeNull();
  // The pull request stays selected, by identity alone.
  expect(within(dialog).getByText('example/atlas#101')).toBeTruthy();
  // Its query is disabled while no report is shown: whatever the commit's
  // own refresh of the still-accepted key read, nothing is read from here on.
  const idle = source.pullRequestSessions.mock.calls.length;
  await act(async () => {
    await Promise.resolve();
  });
  expect(source.pullRequestSessions).toHaveBeenCalledTimes(idle);
  expect(members).toBeLessThanOrEqual(idle);

  // Retry from the drilldown recovers it under the recovered report's own
  // pinned key, from its first page.
  fireEvent.click(within(dialog).getByRole('button', { name: 'Retry' }));
  await within(dialog).findByText('Session s-alpha');
  expect(within(dialog).getByText(/^3 linked sessions · 3 active/)).toBeTruthy();
  const anchor = syntheticReport(scenarios.measured, 7, false).window.end_ms;
  expect(
    source.pullRequestSessions.mock.calls
      .slice(members)
      .map(([request, after]) => [
        request.repository,
        request.number,
        request.windowDays,
        request.windowEndMs,
        request.confirmedOnly,
        after,
      ]),
  ).toEqual([['example/atlas', 101, 7, anchor, false, null]]);
  // Closing still returns focus to the count that opened it — the one the
  // recovered table drew, since the failed read removed the original.
  expect(trigger.isConnected).toBe(false);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(openSessions('example/atlas#101')));
});

it('restarts the drilldown at its first page under a newly read report’s anchor', async () => {
  const source = analyticsSource();
  const control = controlEvents(source);
  const later = 60_000;
  // The same report read a minute later: only its anchor moves.
  const moved = (page: PrAnalyticsPage): PrAnalyticsPage => ({
    ...page,
    window: {
      ...page.window,
      start_ms: page.window.start_ms + later,
      end_ms: page.window.end_ms + later,
    },
    report: { ...page.report, now_ms: page.report.now_ms + later },
  });
  let reads = 0;
  source.pullRequestAnalytics.mockImplementation(async (days, confirmedOnly) => {
    const page = syntheticReport(scenarios.measured, days, confirmedOnly);
    return reads++ === 0 ? page : moved(page);
  });
  const original = report7().window.end_ms;
  source.pullRequestSessions.mockImplementation(async (request, after) => {
    const page = syntheticSessions(
      scenarios.measured,
      { ...request, windowEndMs: original },
      after,
    );
    return request.windowEndMs === original
      ? page
      : { ...page, window: { ...page.window, end_ms: request.windowEndMs } };
  });
  mount(source, '/prs');
  control.resolveAll();
  await loaded();
  fireEvent.click(openSessions('example/atlas#104'));
  const dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  await within(dialog).findByText('50 of 53 linked sessions shown · Sessions list order');
  fireEvent.click(within(dialog).getByRole('button', { name: 'Show 50 more' }));
  await within(dialog).findByText('53 of 53 linked sessions shown · Sessions list order');
  const calls = source.pullRequestSessions.mock.calls.length;

  // A committed import re-reads the report; its new anchor replaces the
  // drilldown whole, from the first page, never mixing the old pages in.
  control.emit(events.importReceived);
  await waitFor(() => expect(source.pullRequestAnalytics).toHaveBeenCalledTimes(2), {
    timeout: 3000,
  });
  await waitFor(() =>
    expect(
      source.pullRequestSessions.mock.calls
        .slice(calls)
        .some(([request, after]) => request.windowEndMs === original + later && after === null),
    ).toBe(true),
  );
  await within(dialog).findByText('50 of 53 linked sessions shown · Sessions list order');
  expect(
    source.pullRequestSessions.mock.calls
      .slice(calls)
      .filter(([request]) => request.windowEndMs === original + later)
      .map(([, after]) => after),
  ).toEqual([null]);
});
