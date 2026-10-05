import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, expect, it, vi } from 'vitest';
import type { PullRequestSessionsRequest } from '../data/DataSource';
import { FixtureDataSource } from '../data/FixtureDataSource';
import { TauriDataSource } from '../data/TauriDataSource';
import { commands } from '../data/ipc-names';
import { scenarios, syntheticReport, syntheticSessions } from './pr-analytics.synthetic';
import { exported, mount, tokensByHost } from './PrsPage.harness';

/**
 * The report view drawn from the browser fixture adapter and from the native
 * adapter answering the same exported pages over IPC: the same cells, tiles
 * and drilldown rows, and only the report's local commands at the boundary.
 */
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

const TABLE = 'Merged pull requests';

/** Everything the view drew: the tiles, every row's cells, one drilldown's rows. */
async function drawn() {
  const table = await screen.findByRole('table', { name: TABLE });
  await within(table).findAllByText(/^example\//);
  const tiles = [...document.querySelectorAll('.xt-prs-tiles .xt-stat-tile')].map(
    (tile) => tile.textContent,
  );
  const rows = within(table)
    .getAllByRole('row')
    .slice(1)
    .map((row) =>
      within(row)
        .getAllByRole('cell')
        .map((cell) => cell.textContent),
    );
  const types = screen.getByRole('list', { name: 'Per-PR medians by work type' }).textContent;
  fireEvent.click(
    screen.getByRole('button', {
      name: /^\d+ linked sessions?, .*open the linked sessions of example\/atlas#102$/,
    }),
  );
  const dialog = await screen.findByRole('dialog', { name: 'Linked sessions' });
  await within(dialog).findByText('Session s-idle');
  const members = within(within(dialog).getByRole('table'))
    .getAllByRole('row')
    .slice(1)
    .map((row) => row.textContent);
  const facts = dialog.querySelector('.xt-pr-drawer-facts')!.textContent;
  return { tiles, rows, types, members, facts };
}

it('draws the same report and drilldown from the fixture and the native adapter', async () => {
  const fixture = structuredClone(exported);
  fixture.pr_analytics = scenarios.measured.pr_analytics;
  fixture.pr_sessions = scenarios.measured.pr_sessions
    .filter((page) => page.after === null)
    .map(({ repository, number, confirmed_only, window_days, window_end_ms, page }) => ({
      repository,
      number,
      confirmed_only,
      window_days,
      window_end_ms,
      page,
    }));
  const preview = new FixtureDataSource(fixture);
  const transcript = vi.spyOn(preview, 'sessionTranscript');
  const list = vi.spyOn(preview, 'sessionsList');
  mount(preview, '/prs');
  const fromFixture = await drawn();
  expect(fromFixture.rows).toHaveLength(11);
  expect(fromFixture.members).toHaveLength(2);
  expect(transcript).not.toHaveBeenCalled();
  expect(list).not.toHaveBeenCalled();
  cleanup();

  vi.mocked(listen).mockResolvedValue(() => {});
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    const input = args as Record<string, unknown>;
    if (command === commands.appInfo) return exported.app_info;
    if (command === commands.tokensByHost) return tokensByHost(input.windowDays as number);
    if (command === commands.nativeIndexStatus) return structuredClone(exported.native_index);
    if (command === commands.pullRequestAnalytics)
      return syntheticReport(
        scenarios.measured,
        input.windowDays as number,
        input.confirmedOnly as boolean,
      );
    if (command === commands.pullRequestSessions) {
      const { after, ...request } = input;
      return syntheticSessions(
        scenarios.measured,
        request as unknown as PullRequestSessionsRequest,
        after as string | null,
      );
    }
    throw new Error(`${command} is not a command the report view may invoke`);
  });
  mount(new TauriDataSource(), '/prs');
  expect(await drawn()).toEqual(fromFixture);
  // Local reads only: never a refresh, a session list, a transcript or the
  // inventory, and the drilldown names the report's own anchor.
  expect(new Set(vi.mocked(invoke).mock.calls.map(([command]) => command))).toEqual(
    new Set([
      commands.accountUsage,
      commands.appInfo,
      commands.tokensByHost,
      commands.nativeIndexStatus,
      commands.pullRequestAnalytics,
      commands.pullRequestSessions,
    ]),
  );
  expect(
    vi.mocked(invoke).mock.calls.filter(([command]) => command === commands.pullRequestSessions),
  ).toEqual([
    [
      commands.pullRequestSessions,
      {
        repository: 'example/atlas',
        number: 102,
        confirmedOnly: false,
        windowDays: 7,
        windowEndMs: syntheticReport(scenarios.measured, 7, false).window.end_ms,
        after: null,
      },
    ],
  ]);
});

it('says F1’s empty report is unknown facts, not nothing merged', async () => {
  mount(new FixtureDataSource(exported), '/prs');
  expect(
    await screen.findByText(
      /^No merged pull request is known in this range\. 3 linked pull requests have unknown cached facts, so they may have merged here\.$/,
    ),
  ).toBeTruthy();
});
