import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { AccountUsage } from '../src/data/generated/AccountUsage';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * Browser evidence for the popover's layout only: this is the fixture preview
 * in a browser page, not the native menu-bar window, its click, position or
 * scale. F1 is pinned at exact UTC midnight, so its own today is the empty day;
 * the measured variant supplies synthetic hours, quota bars and recent sessions
 * (not real history or signed-in account data).
 */
const measured = structuredClone(fixture as FixtureExport);
measured.today = {
  ...measured.today,
  date: '2026-09-08',
  observed_ms: Date.parse('2026-09-08T17:42:00Z'),
  empty: false,
  output: { state: 'recorded', output_tokens: 1_523_000, selected_responses: 412, sessions: 6 },
  cost: {
    ...measured.today.cost,
    state: 'priced',
    total_usd: 24.79,
    priced_subtotal_usd: 24.79,
    selected_observations: 573,
    priced_observations: 573,
  },
  agent: { active_ms: 5.7 * 3_600_000, sessions: 7 },
  human: { active_ms: 2 * 3_600_000, break_minutes: 45 },
};

measured.sessions = measured.sessions.map((page) => ({
  ...page,
  rows: ['Build usage summary', 'Review tray changes', 'Update tray layout'].map(
    (title, index) => ({
      ...page.rows[0],
      id: `codex-synthetic-tray-${index}`,
      host: 'codex',
      title,
      repo:
        index === 1 ? `/synthetic/${'long-repository-name-'.repeat(4)}xtrace` : '/synthetic/xtrace',
      child_check: 'checked',
      known_child: false,
      parent: null,
      last_activity_at_ms: measured.today.observed_ms - index * 60_000,
    }),
  ),
  referenced_parents: [],
}));
const observed = measured.today.observed_ms / 1000;
const usage: AccountUsage = {
  claude: {
    state: 'available',
    issue: null,
    checked_at: observed,
    stale_at: observed + 300,
    windows: [
      {
        bucket_key: 'claude',
        window_key: 'five_hour',
        scope: 'all_models',
        name: 'Claude',
        window: 'Session',
        used_percent: 32,
        duration_minutes: 300,
        resets_at: observed + 3600,
      },
      {
        bucket_key: 'claude',
        window_key: 'seven_day',
        scope: 'all_models',
        name: 'Claude',
        window: 'Weekly',
        used_percent: 66,
        duration_minutes: 10080,
        resets_at: observed + 2 * 86400,
      },
    ],
  },
  codex: {
    state: 'available',
    issue: null,
    checked_at: observed,
    stale_at: observed + 300,
    windows: [
      {
        bucket_key: 'codex',
        window_key: 'primary',
        scope: 'all_models',
        name: 'Codex',
        window: 'Session',
        used_percent: 20,
        duration_minutes: 300,
        resets_at: observed + 7200,
      },
      {
        bucket_key: 'codex',
        window_key: 'secondary',
        scope: 'all_models',
        name: 'Codex',
        window: 'Weekly',
        used_percent: 54,
        duration_minutes: 10080,
        resets_at: observed + 3 * 86400,
      },
    ],
  },
};
/** Test-only module replacement; production fixture behavior is unchanged. */
async function serveMeasuredUsage(page: Page) {
  await page.clock.setFixedTime(new Date(measured.today.observed_ms));
  await page.route('**/src/data/FixtureDataSource.ts', async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    await route.fulfill({
      response,
      body: `${source}
FixtureDataSource.prototype.accountUsage = async () => (${JSON.stringify(usage)});
FixtureDataSource.prototype.liveSessions = {
  read: async (ids, token) => ({view_id: token ?? 'synthetic-tray-lease', states: ids.map(id => ({id, status: id.endsWith('-1') ? 'waiting_approval' : 'running'}))}),
  release: async () => {},
};`,
    });
  });
}

const serve = (page: Page, body: FixtureExport) =>
  page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );

const partial = structuredClone(measured);
partial.today.cost = {
  ...partial.today.cost,
  state: 'partial',
  total_usd: null,
  priced_subtotal_usd: 1234.56,
  priced_observations: 398,
};
const longValues = structuredClone(measured);
longValues.today.agent = { active_ms: 1234 * 3_600_000 + 59 * 60_000, sessions: 12345 };
longValues.today.human = { active_ms: 999 * 3_600_000 + 59 * 60_000, break_minutes: 45 };
longValues.today.cost = {
  ...longValues.today.cost,
  total_usd: 12345678.9,
  priced_subtotal_usd: 12345678.9,
};
const unknown = structuredClone(measured);
unknown.today.cost = {
  ...unknown.today.cost,
  state: 'unpriced',
  total_usd: null,
  priced_subtotal_usd: 0,
  priced_observations: 0,
};
unknown.today.human.active_ms = null;

const zero = structuredClone(measured);
zero.today.cost = { ...zero.today.cost, total_usd: 0, priced_subtotal_usd: 0 };
const tiny = structuredClone(measured);
tiny.today.cost = { ...tiny.today.cost, total_usd: 0.004, priced_subtotal_usd: 0.004 };

for (const [name, body] of [
  ['empty midnight', fixture as FixtureExport],
  ['measured', measured],
  ['partial', partial],
  ['unknown', unknown],
  ['zero', zero],
  ['tiny', tiny],
  ['long values', longValues],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`tray popover lays out the ${name} day in ${scheme}`, async ({ page }, info) => {
      await serve(page, body);
      if (name !== 'empty midnight') await serveMeasuredUsage(page);
      await page.setViewportSize({ width: 360, height: 540 });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/tray');
      const panel = page.getByRole('main', { name: 'XTrace today' });
      await expect(panel.getByRole('region', { name: 'Today’s cost' })).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      // Outside the Shell: no sidebar or top bar.
      await expect(page.getByRole('navigation')).toHaveCount(0);
      await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
      const box = await panel.boundingBox();
      expect(box).toMatchObject({ x: 0, y: 0, width: 360, height: 540 });
      // A transparent page behind the rounded panel, and nothing spilling out.
      const layout = await page.evaluate(() => {
        const main = document.querySelector('main')!;
        const panelColor = getComputedStyle(main).backgroundColor;
        return {
          body: getComputedStyle(document.body).backgroundColor,
          html: getComputedStyle(document.documentElement).backgroundColor,
          panelColor,
          overflow: main.scrollHeight > main.clientHeight || main.scrollWidth > main.clientWidth,
          clipped: [
            ...main.querySelectorAll('.xt-tray-value, .xt-tray-heading, .xt-tray-meta'),
          ].some((element) => element.scrollWidth > element.clientWidth),
        };
      });
      expect(layout.body).toBe('rgba(0, 0, 0, 0)');
      expect(layout.html).toBe('rgba(0, 0, 0, 0)');
      expect(layout.panelColor).not.toBe('rgba(0, 0, 0, 0)');
      expect(layout.overflow).toBe(false);
      expect(layout.clipped).toBe(false);
      await expect(panel.getByRole('button', { name: 'Open XTrace Desktop' })).toBeInViewport();
      await expect(panel.getByRole('region', { name: 'Account usage' })).toBeVisible();
      await expect(panel.getByRole('region', { name: 'Today’s human hours' })).toBeVisible();
      const tiles = panel.locator('.xt-tray-tiles');
      await expect(tiles.locator('section')).toHaveCount(3);
      const cards = await tiles.locator('section').evaluateAll((elements) =>
        elements.map((element) => {
          const box = element.getBoundingClientRect();
          return { top: box.top, left: box.left, right: box.right };
        }),
      );
      expect(new Set(cards.map((card) => card.top)).size).toBe(1);
      expect(cards[0].right).toBeLessThan(cards[1].left);
      expect(cards[1].right).toBeLessThan(cards[2].left);
      const usageBox = await panel.getByRole('region', { name: 'Account usage' }).boundingBox();
      const tilesBox = await tiles.boundingBox();
      expect(tilesBox!.y + tilesBox!.height).toBeLessThan(usageBox!.y);
      await expect(panel.getByRole('region', { name: 'Today’s output tokens' })).toHaveCount(0);
      await expect(panel.locator('.xt-tray-output')).toHaveCount(0);
      await expect(
        panel.getByText(/responses|API-equivalent|not a total|Today · output/),
      ).toHaveCount(0);
      const cost = panel.getByRole('region', { name: 'Today’s cost' });
      await expect(cost).toHaveText(
        `Today · cost${name === 'measured' ? '$24.79' : name === 'long values' ? '$12,345,679' : name === 'zero' ? '$0.00' : name === 'tiny' ? '<$0.01' : '—'}`,
      );
      await expect(cost).toHaveAttribute('title', /Estimated at public API prices/);
      await expect(cost).toHaveAttribute(
        'aria-description',
        (await cost.getAttribute('title')) ?? '',
      );
      if (name === 'partial')
        await expect(cost).toHaveAttribute('title', /Partial \$1,235, not a total/);
      if (name === 'unknown')
        await expect(panel.getByRole('region', { name: 'Today’s human hours' })).toContainText(
          'Unknown',
        );
      const recent = panel.getByRole('region', { name: 'Recent sessions' });
      await expect(recent).toBeInViewport();
      await expect(panel.getByText(/Today in/)).toHaveCount(0);
      await expect(panel.getByText(/Hours use the Dashboard/)).toHaveCount(0);
      await expect(panel.locator('.xt-tray-footnote')).toHaveCount(0);
      if (name !== 'empty midnight') {
        await expect(panel.locator('.xt-account-value').filter({ hasText: '34%' })).toBeVisible();
        await expect(panel.locator('.xt-account-value').filter({ hasText: '46%' })).toBeVisible();
        await expect(recent.getByRole('listitem')).toHaveCount(3);
        await expect(recent.getByText('Waiting for approval')).toBeVisible();
        await expect(recent.getByText('Running', { exact: true })).toHaveCount(0);
        await expect(recent.getByText(/^Codex/)).toHaveCount(0);
        const running = recent.getByRole('img', { name: 'Codex · Running' });
        await expect(running).toHaveCount(2);
        await expect(recent.getByRole('img', { name: 'Codex', exact: true })).toHaveCount(1);
        const icons = await recent.locator('.xt-tray-session-meta').evaluateAll((rows) =>
          rows.map((row) => {
            const icon = row.firstElementChild!;
            const bounds = icon.getBoundingClientRect();
            const parent = row.getBoundingClientRect();
            return {
              width: bounds.width,
              height: bounds.height,
              inside: bounds.left >= parent.left && bounds.right <= parent.right,
            };
          }),
        );
        for (const icon of icons) expect(icon).toEqual({ width: 18, height: 18, inside: true });
        const animation = () =>
          running.first().evaluate((icon) => {
            const style = getComputedStyle(icon, '::before');
            return {
              name: style.animationName,
              duration: style.animationDuration,
              iterations: style.animationIterationCount,
            };
          });
        expect(await animation()).toEqual({
          name: 'xt-live-logo-spin',
          duration: '1.4s',
          iterations: 'infinite',
        });
        await page.emulateMedia({ reducedMotion: 'reduce' });
        expect((await animation()).name).toBe('none');
        await page.screenshot({ path: info.outputPath(`tray-reduced-motion-${scheme}.png`) });
        await page.emulateMedia({ reducedMotion: 'no-preference' });
      }
      await page.screenshot({
        path: info.outputPath(`tray-initial-${name.replace(' ', '-')}-${scheme}.png`),
      });
      // Expanded usage and long content scroll, while Open remains reachable.
      const provider = panel.locator('.xt-account-provider').first();
      if (await provider.count()) await provider.locator('summary').click();
      await expect(panel.getByRole('button', { name: 'Open XTrace Desktop' })).toBeInViewport();
      await page.screenshot({
        path: info.outputPath(`tray-${name.replace(' ', '-')}-${scheme}.png`),
      });
    });
