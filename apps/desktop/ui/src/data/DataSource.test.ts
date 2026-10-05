import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, expect, it, vi } from 'vitest';
import exported from '../../fixtures/F1.json';
import { createDataSource } from './createDataSource';
import { TauriDataSource } from './TauriDataSource';
import type { DataSource } from './DataSource';
import { FixtureDataSource, loadFixtureDataSource, refreshReport } from './FixtureDataSource';
import { commands, events, trayEvents } from './ipc-names';
import type { FixtureExport } from './generated/FixtureExport';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { WebviewWindow } from '@tauri-apps/api/webviewWindow';

// JSON imports widen literal unions; the export is the generated shape.
const shell = exported as unknown as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn(), invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: vi.fn() }));
afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllEnvs();
});

// Factory availability is checked separately; command assertions below start
// after that initial app_info read.
async function nativeSource() {
  const source = await createDataSource();
  expect(invoke).toHaveBeenCalledWith(commands.appInfo);
  vi.mocked(invoke).mockClear();
  return source;
}

it('enables live sessions only when native app_info confirms live mode', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  vi.mocked(invoke).mockResolvedValue({ ...shell.app_info, fixture: null });
  const source = await createDataSource();
  expect(invoke).toHaveBeenCalledExactlyOnceWith(commands.appInfo);
  expect(source.liveSessions).toBeDefined();
  const registered = { view_id: 'native-issued-token', states: [] };
  vi.mocked(invoke).mockResolvedValue(registered);
  expect(await source.liveSessions!.read([], null)).toEqual(registered);
  expect(invoke).toHaveBeenLastCalledWith(commands.liveSessionsRead, {
    sessionIds: [],
    viewId: null,
  });
  await source.liveSessions!.read(['codex-exact-id'], registered.view_id);
  expect(invoke).toHaveBeenLastCalledWith(commands.liveSessionsRead, {
    sessionIds: ['codex-exact-id'],
    viewId: registered.view_id,
  });
  await source.liveSessions!.release(registered.view_id);
  expect(invoke).toHaveBeenLastCalledWith(commands.liveSessionsRelease, {
    viewId: registered.view_id,
  });
});

it('native F1 mode has no live capability regardless of the Vite flag', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.stubEnv('VITE_XTRACE_FIXTURE', '');
  vi.mocked(invoke).mockResolvedValue({ ...shell.app_info, fixture: 'F1' });
  const source = await createDataSource();
  expect(source.kind).toBe('native');
  expect(source.liveSessions).toBeUndefined();
  expect(invoke).toHaveBeenCalledExactlyOnceWith(commands.appInfo);
});

it('unknown native availability and direct construction have no live capability', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  expect(new TauriDataSource().liveSessions).toBeUndefined();
  expect(invoke).not.toHaveBeenCalled();
  vi.mocked(invoke).mockRejectedValue(new Error('native unavailable'));
  expect((await createDataSource()).liveSessions).toBeUndefined();
  vi.mocked(invoke).mockResolvedValue({});
  expect((await createDataSource()).liveSessions).toBeUndefined();
  vi.mocked(invoke).mockResolvedValue(undefined);
  expect((await createDataSource()).liveSessions).toBeUndefined();
  expect(vi.mocked(invoke).mock.calls.every(([command]) => command === commands.appInfo)).toBe(
    true,
  );
});

it('browser fixture and preview have no live capability and perform no native reads', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('DEV', true);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  expect((await createDataSource()).liveSessions).toBeUndefined();
  vi.stubEnv('VITE_XTRACE_FIXTURE', '');
  expect((await createDataSource()).liveSessions).toBeUndefined();
  expect(invoke).not.toHaveBeenCalled();
});

it('native presence always chooses real IPC, including when Vite asks for a fixture', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === commands.appInfo ? exported.app_info : exported.db_counts,
  );
  const stop = vi.fn();
  vi.mocked(listen).mockResolvedValue(stop);
  const source = await nativeSource();
  expect(source.kind).toBe('native');
  expect(await source.appInfo()).toEqual(exported.app_info);
  expect(await source.dbCounts()).toEqual(exported.db_counts);
  expect(invoke).toHaveBeenNthCalledWith(1, 'app_info');
  expect(invoke).toHaveBeenNthCalledWith(2, 'db_counts');
  const listener = vi.fn();
  const unlisten = await source.subscribe(events.importReceived, listener);
  expect(listen).toHaveBeenCalledWith(events.importReceived, expect.any(Function));
  // The listener receives the event's payload, not the Tauri envelope.
  const [, handler] = vi.mocked(listen).mock.calls[0];
  handler({ event: events.importReceived, id: 1, payload: { phase: { phase: 'ready' } } });
  expect(listener).toHaveBeenCalledExactlyOnceWith({ phase: { phase: 'ready' } });
  unlisten();
  expect(stop).toHaveBeenCalledOnce();
});

it('invokes the native metric commands with the selected range', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const [report] = exported.dashboards;
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === commands.dashboard
      ? report
      : { window: report.window, hosts: report.tokens_by_host },
  );
  const source = await nativeSource();
  expect(await source.dashboard(14)).toEqual(report);
  expect(await source.tokensByHost(30)).toEqual({
    window: report.window,
    hosts: report.tokens_by_host,
  });
  expect(invoke).toHaveBeenNthCalledWith(1, 'metrics_dashboard', { windowDays: 14 });
  expect(invoke).toHaveBeenNthCalledWith(2, 'tokens_by_host', { windowDays: 30 });
});

it('reads account limits through one fixed native command with no range or account argument', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const unavailable = { state: 'unavailable', issue: 'no_login', checked_at: null, windows: [] };
  const answer = { claude: unavailable, codex: unavailable };
  vi.mocked(invoke).mockResolvedValue(answer);
  const source = await nativeSource();
  expect(await source.accountUsage()).toEqual(answer);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('account_usage_read');
  vi.mocked(invoke).mockClear();
  expect(await source.refreshClaudeUsage()).toEqual(answer);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('account_usage_refresh_claude');
});

it('keeps browser fixtures offline even when recorded token totals are large', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const answer = await fixture.accountUsage();
  expect(answer.claude.windows).toEqual([]);
  expect(answer.codex.windows).toEqual([]);
  expect(answer.claude.state).toBe('unavailable');
  expect(await fixture.refreshClaudeUsage()).toEqual(answer);
  expect(invoke).not.toHaveBeenCalled();
});

it('invokes the native Environment command with the selected range', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const [report] = exported.environments;
  vi.mocked(invoke).mockResolvedValue(report);
  const source = await nativeSource();
  expect(await source.environment(14)).toEqual(report);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('metrics_environment', { windowDays: 14 });
});

it('serves fixture Environment ranges as isolated copies with an unknown inventory', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const strips = new Set<number>();
  for (const days of [7, 14, 30]) {
    const report = await fixture.environment(days);
    expect(report).toEqual(exported.environments.find((entry) => entry.window.days === days));
    expect(report.window.days).toBe(days);
    // The strip is the same fixed 14 local dates whatever range is selected.
    expect(report.strip_window.days).toBe(14);
    strips.add(report.strip_window.start_ms);
    expect(report.inventory).toBe('unknown');
    expect(report.selected.hosts.every((host) => host.inventory === 'unknown')).toBe(true);
    for (const row of report.identities) expect(row.strip).toHaveLength(14);
    expect(report.identities.map((row) => row.order)).toEqual(
      report.identities.map((_, index) => index),
    );
    report.totals.selected_calls = -1;
    expect((await fixture.environment(days)).totals.selected_calls).not.toBe(-1);
  }
  expect(strips.size).toBe(1);
  for (const days of [0, 15, 90])
    await expect(fixture.environment(days)).rejects.toThrow(
      'Metric range must be 7, 14, or 30 days',
    );
  expect(invoke).not.toHaveBeenCalled();
});

it('serves fixture Dashboard ranges from the generated export as isolated copies', async () => {
  const fixture = await loadFixtureDataSource('F1');
  for (const days of [7, 14, 30]) {
    const report = await fixture.dashboard(days);
    expect(report.window.days).toBe(days);
    expect(report).toEqual(exported.dashboards.find((entry) => entry.window.days === days));
    expect(await fixture.tokensByHost(days)).toEqual({
      window: report.window,
      hosts: report.tokens_by_host,
    });
    report.tiles.sessions.value = -1;
    expect((await fixture.dashboard(days)).tiles.sessions.value).not.toBe(-1);
  }
  for (const days of [0, 15, 90]) {
    await expect(fixture.dashboard(days)).rejects.toThrow('Metric range must be 7, 14, or 30 days');
    await expect(fixture.tokensByHost(days)).rejects.toThrow('Metric range');
  }
  expect(invoke).not.toHaveBeenCalled();
});

it('loads the canonical generated F1 shapes and emits only subscribed fixture events', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  const selected = await createDataSource();
  expect(selected.kind).toBe('fixture');
  expect(await selected.appInfo()).toEqual(exported.app_info);
  expect(await selected.dbCounts()).toEqual(exported.db_counts);
  const info = await selected.appInfo();
  info.version = 'changed';
  expect((await selected.appInfo()).version).toBe(exported.app_info.version);
  const fixture = await loadFixtureDataSource('F1');
  const imported = vi.fn();
  const stop = await fixture.subscribe(events.importReceived, imported);
  fixture.emit(events.prsRefreshed);
  expect(imported).not.toHaveBeenCalled();
  fixture.emit(events.importReceived);
  expect(imported).toHaveBeenCalledOnce();
  stop();
  fixture.emit(events.importReceived);
  expect(imported).toHaveBeenCalledOnce();
  expect(invoke).not.toHaveBeenCalled();
});

it('rejects skeleton and unrecognized fixtures without falling back to invented data', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  for (const id of ['F2', 'F20', '../F1']) {
    vi.stubEnv('VITE_XTRACE_FIXTURE', id);
    await expect(createDataSource()).rejects.toThrow('Only F1 is implemented');
  }
  expect(invoke).not.toHaveBeenCalled();
});

it('keeps ordinary and production browsers explicitly unavailable even with a production fixture flag', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('VITE_XTRACE_FIXTURE', '');
  expect((await createDataSource()).kind).toBe('preview');
  vi.stubEnv('DEV', false);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  const source = await createDataSource();
  expect(source.kind).toBe('preview');
  await expect(source.appInfo()).rejects.toThrow('Native data is unavailable');
  await expect(source.dbCounts()).rejects.toThrow('Native data is unavailable');
  await expect(source.dashboard(7)).rejects.toThrow('Native data is unavailable');
  await expect(source.tokensByHost(7)).rejects.toThrow('Native data is unavailable');
  await expect(source.accountUsage()).rejects.toThrow('Native data is unavailable');
  await expect(source.refreshClaudeUsage()).rejects.toThrow('Native data is unavailable');
  await expect(source.environment(7)).rejects.toThrow('Native data is unavailable');
  await expect(source.pullRequests()).rejects.toThrow('Native data is unavailable');
  await expect(source.refreshPullRequests([1])).rejects.toThrow('Native data is unavailable');
  await expect(source.cancelPullRequestRefresh()).rejects.toThrow('Native data is unavailable');
  await expect(source.pullRequestAnalytics(7, true)).rejects.toThrow('Native data is unavailable');
  await expect(
    source.pullRequestSessions(
      {
        repository: 'octo-org/xtrace-fixture',
        number: 11,
        confirmedOnly: true,
        windowDays: 7,
        windowEndMs: 1,
      },
      null,
    ),
  ).rejects.toThrow('Native data is unavailable');
  expect(invoke).not.toHaveBeenCalled();
});

it('invokes the native transcript commands with the session and this open’s own name', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const status = { state: 'unavailable', reason: { reason: 'missing' } };
  vi.mocked(invoke).mockResolvedValue(status);
  const source = await nativeSource();
  expect(await source.sessionTranscript('session-1', 'read-7')).toEqual(status);
  await source.cancelSessionTranscript('read-7');
  expect(invoke).toHaveBeenNthCalledWith(1, 'session_transcript', {
    sessionId: 'session-1',
    readId: 'read-7',
  });
  // A cancel names one read, so the command carries that name and nothing
  // else: it must never be able to mean "stop reading anything".
  expect(invoke).toHaveBeenNthCalledWith(2, 'cancel_session_transcript', { readId: 'read-7' });
});

it('answers the transcript seam from a fixture without inventing a transcript', async () => {
  // A fixture is a database, not a history: it holds measurements taken from
  // sessions whose files this machine does not have. The honest answer is the
  // one the app gives with no native home — never an empty transcript, which
  // would say the session recorded nothing.
  // Asked through the declared seam, so this is the same call the app makes.
  const fixture: DataSource = await loadFixtureDataSource('F1');
  const [session] = exported.sessions[0].rows;
  expect(await fixture.sessionTranscript(session.id, 'read-1')).toEqual({
    state: 'unavailable',
    reason: { reason: 'not_indexed' },
  });
  await fixture.cancelSessionTranscript('read-1');
  expect(invoke).not.toHaveBeenCalled();
});

it('refuses a transcript in browser preview and still accepts the cancel', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  const source = await createDataSource();
  expect(source.kind).toBe('preview');
  await expect(source.sessionTranscript('session-1', 'read-1')).rejects.toThrow(
    'Native data is unavailable in browser preview',
  );
  // Nothing was started, so there is nothing to abandon — and a view that
  // cleans up after itself must not fail for having tried.
  await expect(source.cancelSessionTranscript('read-1')).resolves.toBeUndefined();
});

it('invokes the native exact-row command with the session and the selected window', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const [row] = exported.sessions[0].rows;
  vi.mocked(invoke).mockResolvedValue(row);
  const source = await nativeSource();
  expect(await source.sessionRow(row.id, 14)).toEqual(row);
  // Named, never searched for: the identity goes as an identity, and the
  // window it is measured over goes with it.
  expect(invoke).toHaveBeenCalledExactlyOnceWith('session_row', {
    sessionId: row.id,
    windowDays: 14,
  });
});

it('serves the fixture’s exact row and nothing for an identity it does not hold', async () => {
  const fixture: DataSource = await loadFixtureDataSource('F1');
  const [row] = exported.sessions[0].rows;
  expect(await fixture.sessionRow(row.id, 7)).toEqual(row);
  // Exact, as the native command is: a prefix is a different session, and an
  // identity the fixture does not hold is nothing rather than an empty row.
  expect(await fixture.sessionRow(row.id.slice(0, 8), 7)).toBeNull();
  expect(await fixture.sessionRow('no-such-session', 7)).toBeNull();
  await expect(fixture.sessionRow(row.id, 15)).rejects.toThrow('Metric range');
  // A copy, so a caller cannot edit the fixture through it.
  const copy = await fixture.sessionRow(row.id, 7);
  copy!.record_count = -1;
  expect((await fixture.sessionRow(row.id, 7))!.record_count).toBe(row.record_count);
  expect(invoke).not.toHaveBeenCalled();
});

it('refuses an exact row in browser preview', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  const source = await createDataSource();
  await expect(source.sessionRow('session-1', 7)).rejects.toThrow(
    'Native data is unavailable in browser preview',
  );
});

it('invokes the native stretches command with the session and the selected window', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const [entry] = exported.session_stretches;
  vi.mocked(invoke).mockResolvedValue(entry.stretches);
  const source = await nativeSource();
  expect(await source.sessionStretches(entry.session_id, 30)).toEqual(entry.stretches);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('session_stretches', {
    sessionId: entry.session_id,
    windowDays: 30,
  });
});

it('serves the fixture’s stretches for each listed session and window, as the native app answers', async () => {
  // The export was produced by the same command a running app answers, and a
  // native test asserts those bytes; here the fixture seam must hand each one
  // back for exactly its session and window.
  const fixture: DataSource = await loadFixtureDataSource('F1');
  expect(exported.session_stretches.length).toBeGreaterThan(0);
  for (const entry of exported.session_stretches) {
    expect(await fixture.sessionStretches(entry.session_id, entry.window_days)).toEqual(
      entry.stretches,
    );
  }
  // Every listed session has an answer in every window it is listed in.
  for (const page of exported.sessions) {
    for (const row of page.rows) {
      expect(
        exported.session_stretches.some(
          (entry) => entry.session_id === row.id && entry.window_days === page.window.days,
        ),
      ).toBe(true);
    }
  }
  const [entry] = exported.session_stretches;
  // Exact identity, as natively: a prefix and an unknown identity are missing.
  expect(await fixture.sessionStretches(entry.session_id.slice(0, 8), 7)).toEqual({
    state: 'missing',
  });
  expect(await fixture.sessionStretches('no-such-session', 7)).toEqual({ state: 'missing' });
  await expect(fixture.sessionStretches(entry.session_id, 15)).rejects.toThrow('Metric range');
  // A copy, so a caller cannot edit the fixture through it.
  const copy = await fixture.sessionStretches(entry.session_id, entry.window_days);
  if (copy.state === 'measured' && copy.stretches[0]) copy.stretches[0].duration_ms = -1;
  expect(await fixture.sessionStretches(entry.session_id, entry.window_days)).toEqual(
    entry.stretches,
  );
  expect(invoke).not.toHaveBeenCalled();
});

it('refuses stretches in browser preview', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  const source = await createDataSource();
  await expect(source.sessionStretches('session-1', 7)).rejects.toThrow(
    'Native data is unavailable in browser preview',
  );
});

it('invokes the native pull-request commands with stored identifiers only', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const list = { rows: [] };
  const report = {
    requested: 1,
    attempted: 1,
    succeeded: 1,
    failed: 0,
    skipped: 0,
    cancelled: false,
    committed: true,
    rows: [],
  };
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === commands.pullRequests) return list;
    if (command === commands.refreshPullRequests) return report;
    return false;
  });
  const source = await nativeSource();
  expect(await source.pullRequests()).toEqual(list);
  expect(await source.refreshPullRequests([7, 9])).toEqual(report);
  expect(await source.cancelPullRequestRefresh()).toBe(false);
  expect(invoke).toHaveBeenNthCalledWith(1, 'prs_list');
  // Only storage's own identifiers cross the boundary: no URL, repository or
  // executable path is ever sent.
  expect(invoke).toHaveBeenNthCalledWith(2, 'prs_refresh', { ids: [7, 9] });
  expect(invoke).toHaveBeenNthCalledWith(3, 'prs_refresh_cancel');
});

it('serves the exported pull requests and refreshes them without any native call', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const listed = await fixture.pullRequests();
  expect(listed).toEqual(shell.pull_requests);
  // Copies, never the export itself.
  listed.rows[0].title = 'changed';
  expect((await fixture.pullRequests()).rows[0].title).toBe(shell.pull_requests.rows[0].title);
  // Every listed row is still linked and carries a canonical identity.
  for (const row of listed.rows) {
    expect(row.linked_sessions).toBeGreaterThan(0);
    expect(row.pull_request.url).toBe(
      `https://github.com/${row.pull_request.repository}/pull/${row.pull_request.number}`,
    );
  }
  expect(await fixture.cancelPullRequestRefresh()).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
});

it('reports the exported outcome of each selected pull request and updates only those rows', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const [first, , third] = shell.pull_requests.rows;
  const unknown = Math.max(...shell.pull_requests.rows.map((row) => row.pull_request.id)) + 1;
  const refreshed = vi.fn();
  await fixture.subscribe(events.prsRefreshed, refreshed);
  const report = await fixture.refreshPullRequests([
    first.pull_request.id,
    unknown,
    third.pull_request.id,
  ]);
  // The selection's order is the report's order, and an unknown identifier is
  // skipped rather than invented.
  expect(report.rows.map((row) => row.id)).toEqual([
    first.pull_request.id,
    unknown,
    third.pull_request.id,
  ]);
  expect(report.rows[1]).toEqual({
    id: unknown,
    pull_request: null,
    outcome: { outcome: 'skipped', reason: 'not_stored' },
  });
  expect([report.requested, report.attempted, report.skipped]).toEqual([3, 2, 1]);
  expect(report.committed).toBe(true);
  expect(refreshed).toHaveBeenCalledOnce();
  // Only the selected rows move; the rest still read as they were exported.
  const after = await fixture.pullRequests();
  const row = (list: typeof after, id: number) =>
    list.rows.find((entry) => entry.pull_request.id === id);
  for (const id of [first.pull_request.id, third.pull_request.id])
    expect(row(after, id)).toEqual(row(shell.pull_requests_refreshed, id));
  const untouched = shell.pull_requests.rows[1].pull_request.id;
  expect(row(after, untouched)).toEqual(row(shell.pull_requests, untouched));
  expect(invoke).not.toHaveBeenCalled();
  await expect(fixture.refreshPullRequests([])).rejects.toThrow('Select at least one');
});

it('derives the same report the exporting batch derived', async () => {
  // The preview counts rows itself; for the whole selection that must equal
  // the report Rust produced, so the two accountings cannot drift apart.
  expect(refreshReport(shell.pr_refresh.rows, shell.pr_refresh.cancelled)).toEqual(
    shell.pr_refresh,
  );
  const fixture = await loadFixtureDataSource('F1');
  expect(
    await fixture.refreshPullRequests(shell.pull_requests.rows.map((row) => row.pull_request.id)),
  ).toEqual(shell.pr_refresh);
  expect((await fixture.pullRequests()).rows).toEqual(shell.pull_requests_refreshed.rows);
});

it('replays a repeated refresh as unchanged, exactly as native fixture mode does', async () => {
  // The same sequence and expectations as the native Rust test
  // `a_repeated_fixture_refresh_is_unchanged_and_emits_nothing`.
  const fixture = await loadFixtureDataSource('F1');
  const [first, second] = shell.pull_requests.rows.map((row) => row.pull_request.id);
  const event = vi.fn();
  await fixture.subscribe(events.prsRefreshed, event);
  const written = (report: Awaited<ReturnType<typeof fixture.refreshPullRequests>>) =>
    report.rows.map((row) =>
      row.outcome.outcome === 'skipped'
        ? row.outcome.reason
        : row.outcome.persistence.persistence === 'recorded'
          ? `${row.outcome.outcome}:${row.outcome.persistence.write}`
          : `${row.outcome.outcome}:not_recorded`,
    );

  const once = await fixture.refreshPullRequests([first]);
  expect(written(once)).toEqual(['succeeded:applied']);
  expect(once.committed).toBe(true);
  expect(event).toHaveBeenCalledTimes(1);

  // A partial overlap: the repeat is unchanged, the new one applies.
  const overlap = await fixture.refreshPullRequests([first, second]);
  expect(written(overlap)).toEqual(['succeeded:unchanged', 'failed:applied']);
  expect(overlap.committed).toBe(true);
  expect(event).toHaveBeenCalledTimes(2);

  // A full repeat commits nothing and emits nothing.
  const again = await fixture.refreshPullRequests([first, second]);
  expect(written(again)).toEqual(['succeeded:unchanged', 'failed:unchanged']);
  expect([again.attempted, again.unrecorded]).toEqual([2, 0]);
  expect(again.committed).toBe(false);
  expect(event).toHaveBeenCalledTimes(2);
  // The export itself is never altered by a replay.
  expect(
    shell.pr_refresh.rows.every(
      (row) =>
        row.outcome.outcome === 'skipped' ||
        row.outcome.persistence.persistence !== 'recorded' ||
        row.outcome.persistence.write === 'applied',
    ),
  ).toBe(true);
});

it('serves the M-19 state Rust read after refreshing exactly what the preview refreshed', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const byNumber = new Map(
    shell.pull_requests.rows.map((row) => [row.pull_request.number, row.pull_request.id]),
  );
  const [exact, sha, inferred] = [11, 12, 13].map((number) => byNumber.get(number)!);
  const section = async (days: number) => {
    const report = await fixture.dashboard(days);
    return { merged_prs: report.tiles.merged_prs, pr_effort: report.pr_effort };
  };
  const before = (days: number) => {
    const report = shell.dashboards.find((entry) => entry.window.days === days)!;
    return { merged_prs: report.tiles.merged_prs, pr_effort: report.pr_effort };
  };
  const state = (ids: number[], days: number) => {
    const found = shell.pr_effort_states.find(
      (entry) =>
        JSON.stringify(entry.refreshed.map((reference) => reference.id)) ===
        JSON.stringify([...ids].sort((a, b) => a - b)),
    )!;
    const { merged_prs, pr_effort } = found.sections.find((entry) => entry.days === days)!;
    return { merged_prs, pr_effort };
  };
  // Every non-empty subset of the three listed pull requests has a state.
  expect(shell.pr_effort_states.map((entry) => entry.refreshed.length).sort()).toEqual([
    1, 1, 1, 2, 2, 2, 3,
  ]);
  for (const days of [7, 14, 30]) expect(await section(days)).toEqual(before(days));
  // Only the exact link #11: it is refreshed, and the SHA link #12 still has
  // no facts, so the count stays unknown with one never-refreshed pull request.
  await fixture.refreshPullRequests([exact]);
  for (const days of [7, 14, 30]) expect(await section(days)).toEqual(state([exact], days));
  const onlyExact = state([exact], 7).pr_effort.current.tile;
  expect([onlyExact.merged, onlyExact.unknown_facts]).toEqual([null, 1]);
  expect([onlyExact.freshness.refreshed, onlyExact.freshness.never_attempted]).toEqual([1, 1]);
  // Both confirmed links refreshed, the inferred one never: confirmed-only
  // M-19 ignores inferred links, so this is exactly the whole-refresh section.
  await fixture.refreshPullRequests([sha]);
  for (const days of [7, 14, 30]) {
    expect(await section(days)).toEqual(state([exact, sha], days));
    expect(state([exact, sha], days)).toEqual(state([exact, sha, inferred], days));
  }
  // Everything else is still the one exported report, and the export is untouched.
  const report = await fixture.dashboard(7);
  expect({ ...report, tiles: { ...report.tiles, merged_prs: null }, pr_effort: null }).toEqual({
    ...shell.dashboards[0]!,
    tiles: { ...report.tiles, merged_prs: null },
    pr_effort: null,
  });
  expect(shell.dashboards[0]!.pr_effort).toEqual(before(7).pr_effort);
});

it('reads today and, only in the tray window, drives the popover through its own commands', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.mocked(invoke).mockResolvedValue(exported.today);
  // The main window: today is readable, and there are no popover controls.
  vi.mocked(getCurrentWebviewWindow).mockReturnValue({ label: 'main' } as WebviewWindow);
  const main = await nativeSource();
  expect(main.tray).toBeUndefined();
  expect(await main.today()).toEqual(exported.today);
  expect(invoke).toHaveBeenLastCalledWith(commands.today);

  vi.mocked(getCurrentWebviewWindow).mockReturnValue({ label: 'tray' } as WebviewWindow);
  const popover = await nativeSource();
  const tray = popover.tray!;
  vi.mocked(invoke).mockResolvedValue(true);
  expect(await tray.visible()).toBe(true);
  expect(invoke).toHaveBeenLastCalledWith('tray_visible');
  await tray.hide();
  expect(invoke).toHaveBeenLastCalledWith('tray_hide');
  await tray.openMain();
  expect(invoke).toHaveBeenLastCalledWith('tray_open_main');
  const stop = vi.fn();
  vi.mocked(listen).mockResolvedValue(stop);
  const shown = vi.fn();
  (await tray.onShown(shown))();
  await tray.onHidden(vi.fn());
  const target = { target: { kind: 'WebviewWindow', label: 'tray' } };
  expect(listen).toHaveBeenNthCalledWith(1, trayEvents.shown, expect.any(Function), target);
  expect(listen).toHaveBeenNthCalledWith(2, trayEvents.hidden, expect.any(Function), target);
  expect(stop).toHaveBeenCalledOnce();
});

it('a fixture serves its exported today, and the browser preview has none', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const today = await fixture.today();
  expect(today).toEqual(exported.today);
  // A copy: the export itself cannot be changed through it.
  today.output.sessions = 99;
  expect((await fixture.today()).output.sessions).toBe(0);
  expect((fixture as DataSource).tray).toBeUndefined();
  vi.mocked(isTauri).mockReturnValue(false);
  const preview = await createDataSource();
  await expect(preview.today()).rejects.toThrow('Native data is unavailable');
});

it('sends the whole session filter to the native list command', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const [page] = shell.sessions;
  vi.mocked(invoke).mockResolvedValue(page);
  const source = await nativeSource();
  await source.sessionsList(
    { search: 'atlas', hosts: ['claude', 'codex'], withPrs: true },
    'cursor-1',
    14,
  );
  expect(invoke).toHaveBeenCalledExactlyOnceWith('sessions_list', {
    search: 'atlas',
    hosts: ['claude', 'codex'],
    withPrs: true,
    after: 'cursor-1',
    windowDays: 14,
  });
});

it('filters the fixture list the way the store does: hosts, recorded links and saved titles', async () => {
  const source = new FixtureDataSource(shell);
  const [row] = shell.sessions[0].rows;
  const ids = async (filter: Parameters<DataSource['sessionsList']>[0]) =>
    (await source.sessionsList(filter, null, 7)).rows.map((r) => r.id);
  const all = { search: '', hosts: null, withPrs: false };
  expect(await ids(all)).toEqual([row.id]);
  expect(await ids({ ...all, hosts: ['codex'] })).toEqual([]);
  expect(await ids({ ...all, hosts: [row.host] })).toEqual([row.id]);
  // F1's session records links, so the pull-request filter keeps it.
  expect(row.pr_links.length).toBeGreaterThan(0);
  expect(await ids({ ...all, withPrs: true })).toEqual([row.id]);
  expect(await ids({ ...all, search: row.id.slice(2, 10).toUpperCase() })).toEqual([row.id]);
  expect(await ids({ ...all, search: 'not-present' })).toEqual([]);
  await expect(source.sessionsList(all, 'next', 7)).rejects.toThrow('one page');
});

it('titles each listed link from the current PR state: before, one, failed and all refreshed', async () => {
  const source = new FixtureDataSource(shell);
  const [row] = shell.sessions[0].rows;
  const all = { search: '', hosts: null, withPrs: false };
  /** The list's, the exact row's and the PR dialog's titles, by canonical identity. */
  const titles = async () => {
    const listed = (await source.sessionsList(all, null, 7)).rows[0].pr_links;
    const exact = (await source.sessionRow(row.id, 7))!.pr_links;
    const dialog = new Map(
      (await source.pullRequests()).rows.map((pr) => [
        `${pr.pull_request.repository}#${pr.pull_request.number}`,
        pr.title,
      ]),
    );
    const shown = listed.map((link) => link.title);
    expect(exact.map((link) => link.title)).toEqual(shown);
    // The list and the PR dialog always agree.
    expect(shown).toEqual(listed.map((link) => dialog.get(`${link.repository}#${link.number}`)));
    return shown;
  };
  // The Rust export's sessions are read before any refresh: no stored title.
  expect(row.pr_links.map((link) => link.title)).toEqual([null, null, null]);
  expect(await titles()).toEqual([null, null, null]);
  // One selected pull request.
  expect((await source.refreshPullRequests([1])).committed).toBe(true);
  expect(await titles()).toEqual(['Synthetic fixture pull request 11', null, null]);
  // A failed refresh stores no title.
  const failed = await source.refreshPullRequests([2]);
  expect(failed.rows[0].outcome.outcome).toBe('failed');
  expect(await titles()).toEqual(['Synthetic fixture pull request 11', null, null]);
  // The full selection leaves exactly the Rust-authored refreshed state.
  await source.refreshPullRequests([1, 2, 3]);
  const refreshed = new Map(
    shell.pull_requests_refreshed.rows.map((pr) => [pr.pull_request.number, pr.title]),
  );
  expect(await titles()).toEqual(row.pr_links.map((link) => refreshed.get(link.number) ?? null));
  expect(await titles()).toEqual([
    'Synthetic fixture pull request 11',
    null,
    'Synthetic fixture pull request 13',
  ]);
  // The export itself is never mutated by the projection.
  expect(shell.sessions[0].rows[0].pr_links.map((link) => link.title)).toEqual([null, null, null]);
});

it('invokes the native PRs report and drilldown commands with exactly the request', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const [page] = shell.pr_analytics;
  const [drill] = shell.pr_sessions;
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === commands.pullRequestAnalytics ? page : drill!.page,
  );
  const source = await nativeSource();
  expect(await source.pullRequestAnalytics(14, true)).toEqual(page);
  const request = {
    repository: 'octo-org/xtrace-fixture',
    number: 11,
    confirmedOnly: false,
    windowDays: 30,
    windowEndMs: 1_788_825_600_000,
  };
  expect(await source.pullRequestSessions(request, 'cursor')).toEqual(drill!.page);
  expect(invoke).toHaveBeenNthCalledWith(1, 'prs_analytics', {
    windowDays: 14,
    confirmedOnly: true,
  });
  // The anchor and the cursor are passed through untouched: nothing here
  // reads a clock or re-derives the window.
  expect(invoke).toHaveBeenNthCalledWith(2, 'prs_sessions', { ...request, after: 'cursor' });
  expect(invoke).toHaveBeenCalledTimes(2);
});

it('serves the PRs report Rust read for each range, mode and refresh state', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const refresh = vi.spyOn(fixture, 'refreshPullRequests');
  const exportedPage = (pages: typeof shell.pr_analytics, days: number, confirmed: boolean) =>
    pages.find((page) => page.window.days === days && page.report.confirmed_only === confirmed);
  // Six pages per state: three ranges, both confidence modes.
  expect(shell.pr_analytics).toHaveLength(6);
  for (const state of shell.pr_effort_states) expect(state.analytics).toHaveLength(6);
  for (const days of [7, 14, 30])
    for (const confirmed of [false, true]) {
      const page = await fixture.pullRequestAnalytics(days, confirmed);
      expect(page).toEqual(exportedPage(shell.pr_analytics, days, confirmed));
      // Before any refresh a fixture database has no cached merge facts.
      expect(page.report.rows).toEqual([]);
      expect(page.report.eligibility.unknown_facts).toBeGreaterThan(0);
      page.report.eligibility.unknown_facts = -1;
      expect(
        (await fixture.pullRequestAnalytics(days, confirmed)).report.eligibility.unknown_facts,
      ).not.toBe(-1);
    }
  // After a refresh, the state the export read after refreshing exactly that.
  const id11 = shell.pull_requests.rows.find((row) => row.pull_request.number === 11)!.pull_request
    .id;
  await fixture.refreshPullRequests([id11]);
  const state = shell.pr_effort_states.find(
    (entry) => entry.refreshed.length === 1 && entry.refreshed[0]!.id === id11,
  )!;
  for (const days of [7, 14, 30])
    for (const confirmed of [false, true])
      expect(await fixture.pullRequestAnalytics(days, confirmed)).toEqual(
        exportedPage(state.analytics, days, confirmed),
      );
  for (const days of [0, 15])
    await expect(fixture.pullRequestAnalytics(days, true)).rejects.toThrow(
      'The fixture export holds no PR report',
    );
  expect(refresh).toHaveBeenCalledOnce();
  expect(invoke).not.toHaveBeenCalled();
});

it('serves exactly the exported drilldown, titled from the current PR state, and nothing else', async () => {
  const fixture = await loadFixtureDataSource('F1');
  const anchor = shell.pr_analytics[0]!.window.end_ms;
  const request = {
    repository: 'octo-org/xtrace-fixture',
    number: 13,
    confirmedOnly: false,
    windowDays: 7,
    windowEndMs: anchor,
  };
  const exportedFor = (number: number, confirmed: boolean) =>
    shell.pr_sessions.find(
      (entry) =>
        entry.number === number &&
        entry.confirmed_only === confirmed &&
        entry.window_days === 7 &&
        entry.window_end_ms === anchor,
    )!.page;
  // Every listed pull request, range and mode, pinned to the report's end.
  expect(shell.pr_sessions).toHaveLength(3 * 3 * 2);
  expect(shell.pr_sessions.every((entry) => entry.window_end_ms === anchor)).toBe(true);
  expect(await fixture.pullRequestSessions(request, null)).toEqual(exportedFor(13, false));
  // #13's only link is inferred: confirmed only, it has no members.
  expect(
    (await fixture.pullRequestSessions({ ...request, confirmedOnly: true }, null)).rows,
  ).toEqual([]);
  // Titles are refresh-owned: after refreshing #13, its link carries the title.
  const id13 = shell.pull_requests.rows.find((row) => row.pull_request.number === 13)!.pull_request
    .id;
  await fixture.refreshPullRequests([id13]);
  const titled = await fixture.pullRequestSessions(request, null);
  expect(titled.window).toEqual(exportedFor(13, false).window);
  expect(titled.rows[0]!.pr_links.find((link) => link.number === 13)!.title).toBe(
    'Synthetic fixture pull request 13',
  );
  // A request the export does not hold fails; nothing is filtered here.
  for (const refused of [
    { ...request, windowEndMs: anchor + 1 },
    { ...request, windowDays: 15 },
    { ...request, number: 99 },
    { ...request, repository: 'octo-org/xtrace' },
  ])
    await expect(fixture.pullRequestSessions(refused, null)).rejects.toThrow(
      'The fixture export holds no linked sessions',
    );
  // Canonical casing names the same pull request, as the native identity does.
  expect(
    await fixture.pullRequestSessions({ ...request, repository: 'OCTO-ORG/xtrace-fixture' }, null),
  ).toEqual(titled);
  await expect(fixture.pullRequestSessions(request, 'cursor')).rejects.toThrow(
    'The fixture has one page.',
  );
  expect(invoke).not.toHaveBeenCalled();
});

it('invokes the native rule activity commands with this read’s own name and nothing else', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const result = { state: 'busy', reason: 'another_read' };
  vi.mocked(invoke).mockResolvedValue(result);
  const source = await nativeSource();
  expect(await source.ruleActivity?.read('read-3')).toEqual(result);
  await source.ruleActivity?.cancel('read-3');
  // No root, path, range or filter crosses: the native side chooses the
  // source and anchors the window.
  expect(invoke).toHaveBeenNthCalledWith(1, commands.ruleActivityRead, { readId: 'read-3' });
  expect(invoke).toHaveBeenNthCalledWith(2, commands.ruleActivityCancel, { readId: 'read-3' });
  expect(commands.ruleActivityRead).toBe('rule_activity_read');
  expect(commands.ruleActivityCancel).toBe('rule_activity_cancel');
});

it('answers rule activity from a fixture exactly as fixture mode does, never as a read', async () => {
  const fixture: DataSource = await loadFixtureDataSource('F1');
  const first = await fixture.ruleActivity?.read('read-1');
  expect(first).toEqual({
    state: 'unavailable',
    source: 'default_local_rulebook',
    part: 'root',
    reason: 'not_configured',
    found_version: null,
  });
  expect(first).toEqual(shell.rule_activity);
  // An isolated copy: a view that edits its answer changes no later one.
  if (first?.state === 'unavailable') first.reason = 'missing';
  expect(await fixture.ruleActivity?.read('read-2')).toEqual(shell.rule_activity);
  await fixture.ruleActivity?.cancel('read-2');
  expect(invoke).not.toHaveBeenCalled();
});

it('offers no rule activity in browser preview', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  const source = await createDataSource();
  expect(source.kind).toBe('preview');
  expect(source.ruleActivity).toBeUndefined();
  expect(invoke).not.toHaveBeenCalled();
});
