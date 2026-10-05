import { test, expect } from '@playwright/test';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import fixture from '../fixtures/F1.json' with { type: 'json' };
for (const colorScheme of ['light', 'dark'] as const) {
  test(`indexed Sessions design columns, filters and details in ${colorScheme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme });
    await page.goto('/sessions');
    await expect(page.getByRole('heading', { name: 'Sessions', exact: true })).toBeVisible();
    await expect(page.getByRole('table', { name: 'Indexed sessions' })).toBeVisible();
    // F1 saves no title, so its identity leads; nothing is invented. Its
    // repository and branch are unknown and say so.
    await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
    await expect(page.getByText('fixture-model-v1', { exact: true })).toBeVisible();
    const headers = await page.getByRole('columnheader').allTextContents();
    expect(headers.map((header) => header.trim()).filter(Boolean)).toEqual([
      'Details',
      'Host',
      'session · repo · branch',
      'PRs',
      'started',
      'msgs',
      'tools',
      'agent min',
      'hands-off',
      'output',
    ]);
    // F1 over the selected window: five human messages, five tool calls, a
    // 23-minute active span, a three-minute hands-off median over five
    // stretches and 150 output tokens (not the 1,100 total).
    const row = page.getByRole('row').filter({ hasText: 'Session 00000000' });
    for (const measured of ['5', '0h23m', '3', '150']) {
      await expect(row.getByText(measured, { exact: true }).first()).toBeVisible();
    }
    await expect(row.getByText('1.1K', { exact: true })).toHaveCount(0);
    // Two badges and a count; the inferred one is drawn and named as such.
    await expect(
      row.getByRole('img', { name: /Pull request #11 in octo-org\/xtrace-fixture, exact/ }),
    ).toBeVisible();
    await expect(
      row.getByRole('img', { name: /Pull request #12 in octo-org\/xtrace-fixture, commit/ }),
    ).toBeVisible();
    await expect(row.getByText('+1', { exact: true })).toBeVisible();
    // Everything fits its column in this engine: no clipped value, and no
    // horizontal page scroll at the reference width.
    const fit = await page.evaluate(() => {
      const cells = [
        ...document.querySelectorAll<HTMLElement>(
          '.xt-sessions .xt-session-mono, .xt-sessions .xt-session-pr, .xt-sessions .xt-metric-cell',
        ),
      ];
      return {
        overflowing: cells
          .filter((cell) => cell.scrollWidth > cell.clientWidth + 0.5)
          .map((cell) => cell.textContent),
        pageScroll: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      };
    });
    expect(fit.overflowing).toEqual([]);
    expect(fit.pageScroll).toBe(false);
    await expect(page.getByRole('radio', { name: '7d' })).toHaveAttribute('aria-checked', 'true');
    await page.screenshot({ path: info.outputPath(`sessions-${colorScheme}.png`) });

    // Each measured column states its own S2 rule, reachable from the keyboard.
    for (const [name, ruleId] of [
      ['Human messages, definition M-02', 'M-02'],
      ['Tool calls, definition M-17', 'M-17'],
      ['Agent minutes, definition M-05', 'M-05'],
      ['Hands-off median minutes, definition M-09', 'M-09'],
      ['Output tokens, definition M-04', 'M-04'],
    ] as const) {
      const header = page.getByRole('button', { name });
      await header.focus();
      await expect(header).toBeFocused();
      const definition = page.getByRole('tooltip');
      await expect(definition).toContainText(`${ruleId} · `);
      await page.keyboard.press('Escape');
      await expect(definition).toHaveCount(0);
    }

    // The diagnostic facts are one keyboard disclosure away, not erased.
    // The row's own disclosure; its name says Expand or Collapse by state.
    const expand = row.getByRole('button', {
      name: /^(Expand|Collapse) 00000000-0000-4000-8000-000000000001$/,
    });
    await expand.focus();
    await page.keyboard.press('Enter');
    await expect(expand).toHaveAttribute('aria-expanded', 'true');
    const details = page.locator('.xt-session-details');
    await expect(details).toContainText('First recorded');
    await expect(details).toContainText('25');
    await expect(details).toContainText('No source observation disagrees');
    await expect(details).toContainText('1.1K');
    await expect(details).toContainText('median 3 min over 5 stretches');
    await expect(details).toContainText('octo-org/xtrace-fixture#13 · inferred');
    await page.screenshot({ path: info.outputPath(`sessions-details-${colorScheme}.png`) });
    await page.keyboard.press('Enter');
    await expect(expand).toHaveAttribute('aria-expanded', 'false');

    // Search, then the compact host menu and the PR switch, by keyboard.
    await page.getByLabel('Search sessions').fill('missing');
    await expect(page.getByText('No sessions match these filters.')).toBeVisible();
    await page.getByLabel('Search sessions').fill('');
    await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
    const hosts = page.getByRole('button', { name: 'Filter by host' });
    await hosts.focus();
    await page.keyboard.press('Enter');
    const claude = page.getByRole('checkbox', { name: 'Claude Code' });
    await expect(claude).toBeChecked();
    await claude.focus();
    await page.keyboard.press('Space');
    await expect(claude).not.toBeChecked();
    await expect(page).toHaveURL(/[?&]host=codex%2Ccursor(&|$)/);
    await page.screenshot({ path: info.outputPath(`sessions-hosts-${colorScheme}.png`) });
    await page.keyboard.press('Escape');
    await expect(page.getByText('No sessions match these filters.')).toBeVisible();
    await hosts.focus();
    await page.keyboard.press('Enter');
    await claude.focus();
    await page.keyboard.press('Space');
    await page.keyboard.press('Escape');
    await expect(page).not.toHaveURL(/[?&]host=/);
    await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
    const prs = page.getByRole('switch', { name: 'With PRs only' });
    await prs.focus();
    await page.keyboard.press('Space');
    await expect(prs).toHaveAttribute('aria-checked', 'true');
    await expect(page).toHaveURL(/[?&]with_prs=1(&|$)/);
    // F1's session has recorded links, so it stays listed.
    await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
  });
}

/**
 * Agent time as hours and remaining minutes at the smallest supported window.
 * The widest a row can read is just under 720 hours: one session's active
 * spans never overlap, and the longest range is 30 days. Served in place of
 * the development F1 export; these are not real history.
 */
for (const colorScheme of ['light', 'dark'] as const) {
  test(`agent time fits as hours and minutes with its exact value in ${colorScheme}`, async ({
    page,
  }, info) => {
    const served = structuredClone(fixture as FixtureExport);
    const [first] = served.sessions[0].rows;
    const values = [
      ['00000000-0000-4000-8000-000000000001', 719 * 3_600_000 + 59.9 * 60_000],
      ['00000000-0000-4000-8000-000000000002', 11_568_000],
      ['00000000-0000-4000-8000-000000000003', 2_000],
      ['00000000-0000-4000-8000-000000000004', 0],
    ] as const;
    for (const page of served.sessions)
      page.rows = values.map(([id, agent_ms]) => ({
        ...first,
        id,
        metrics: first.metrics.state === 'indexed' ? { ...first.metrics, agent_ms } : first.metrics,
      }));
    await page.route('**/fixtures/F1.json?import', (route) =>
      route.fulfill({
        contentType: 'text/javascript',
        body: `export default ${JSON.stringify(served)};`,
      }),
    );
    await page.setViewportSize({ width: 1120, height: 720 });
    await page.emulateMedia({ colorScheme });
    await page.goto('/sessions');
    await expect(page.getByRole('table', { name: 'Indexed sessions' })).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    const expected = ['719h59.9m', '3h12.8m', '<0.1m', '0h00m'];
    for (const text of expected)
      await expect(
        page.locator('.xt-session-agent .xt-metric-cell', { hasText: text }),
      ).toHaveCount(1);
    const layout = await page.evaluate(() => {
      const header = [...document.querySelectorAll<HTMLElement>('[role="columnheader"]')].find(
        (cell) => cell.textContent?.trim() === 'agent min',
      )!;
      const cells = [...document.querySelectorAll<HTMLElement>('.xt-session-agent')].map(
        (agent) => {
          const value = agent.querySelector<HTMLElement>('.xt-metric-cell')!;
          const cell = agent.closest<HTMLElement>('[role="cell"]')!;
          return {
            text: value.textContent,
            fits: value.scrollWidth <= value.clientWidth + 0.5,
            inside:
              value.getBoundingClientRect().left >= cell.getBoundingClientRect().left - 0.5 &&
              value.getBoundingClientRect().right <= cell.getBoundingClientRect().right + 0.5,
            right: cell.getBoundingClientRect().right,
            width: cell.getBoundingClientRect().width,
          };
        },
      );
      return {
        cells,
        headerRight: header.getBoundingClientRect().right,
        pageScroll: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      };
    });
    await info.attach('agent-cells', { body: JSON.stringify(layout, null, 2) });
    await page
      .getByRole('table', { name: 'Indexed sessions' })
      .screenshot({ path: info.outputPath(`sessions-agent-time-${colorScheme}.png`) });
    expect(layout.cells.map((cell) => cell.text)).toEqual(expected);
    for (const cell of layout.cells) {
      expect(cell.fits, cell.text ?? '').toBe(true);
      expect(cell.inside, cell.text ?? '').toBe(true);
      // The column keeps its width and stays under its header.
      expect(cell.width).toBeCloseTo(64, 0);
      expect(cell.right).toBeCloseTo(layout.headerRight, 0);
    }
    expect(layout.pageScroll).toBe(false);
    // Pointer users get the exact measurement as the tooltip.
    await expect(
      page.locator('.xt-session-agent[title="Exactly 11,568,000 ms active in this range"]'),
    ).toHaveCount(1);
    // Keyboard users open the row's details and read it there.
    const expand = page.getByRole('button', {
      name: 'Expand 00000000-0000-4000-8000-000000000002',
    });
    await expand.focus();
    await expect(expand).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.locator('.xt-session-details')).toContainText(
      'Agent time3h12.8m exactly 11,568,000 ms active in this range',
    );
  });
}
