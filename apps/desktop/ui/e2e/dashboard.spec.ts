import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

const reports = Object.fromEntries(
  fixture.dashboards.map((report) => [report.window.days, report]),
);
const sizes = [
  { width: 1440, height: 900 },
  { width: 1120, height: 720 },
] as const;
const labels = ['Agent h/day', 'Concurrency', 'Merged PRs', 'Hands-off median'];

async function open(page: Page, width: number, height: number, scheme: 'light' | 'dark') {
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

/** Every rendered text box inside the Dashboard must fit its own box: no clipped labels. */
async function measure(page: Page) {
  return page.evaluate(() => {
    const main = document.querySelector('.xt-shell-main')!.getBoundingClientRect();
    const clipped = [...document.querySelectorAll<HTMLElement>('.xt-dashboard *')]
      .filter((element) => {
        if (element.closest('.xt-dash-table-scroll, .xt-lanes-scroll, .sr-only')) return false;
        const style = getComputedStyle(element);
        const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
        return clips && element.scrollWidth > element.clientWidth + 1;
      })
      .map((element) => `${element.className}: ${element.textContent}`);
    const tiles = [...document.querySelectorAll('.xt-stat-tile')].map((tile) => {
      const label = tile.querySelector<HTMLElement>('.xt-stat-label')!;
      return {
        text: label.textContent,
        fits: label.scrollWidth <= label.clientWidth,
        label: label.getBoundingClientRect().width,
        tile: tile.getBoundingClientRect(),
      };
    });
    const h1 = document.querySelector('.xt-dashboard h1')!.getBoundingClientRect();
    return {
      scrollWidth: document.documentElement.scrollWidth,
      outletOverflow:
        document.querySelector('.xt-shell-outlet')!.scrollWidth -
        document.querySelector('.xt-shell-outlet')!.clientWidth,
      sidebar: document.querySelector('.xt-sidebar')!.getBoundingClientRect().width,
      topbar: document.querySelector('.xt-topbar')!.getBoundingClientRect().height,
      main: main.width,
      h1: { x: h1.x, y: h1.y, height: h1.height },
      clipped,
      tiles,
    };
  });
}

for (const { width, height } of sizes)
  for (const scheme of ['light', 'dark'] as const)
    test(`fits F1 Dashboard at ${width}x${height} ${scheme} with full labels`, async ({
      page,
    }, info) => {
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      page.on('console', (message) => {
        if (message.type() === 'error') errors.push(message.text());
      });
      await open(page, width, height, scheme);
      await expect(page.getByTestId('report-period')).toBeVisible();
      const layout = await page.evaluate(() => {
        const box = (element: Element | null | undefined) => element!.getBoundingClientRect();
        const heading = (name: string) =>
          [...document.querySelectorAll('.xt-dashboard h2')]
            .find((h) => h.textContent === name)!
            .closest('section');
        const range = box(document.querySelector('.xt-range'));
        const period = box(document.querySelector('[data-testid="report-period"]'));
        return {
          periodBeforeRange: period.right <= range.left && period.right > range.left - 16,
          periodRowMatches:
            Math.abs(period.top + period.height / 2 - (range.top + range.height / 2)) < 2,
          hero: box(heading('Agent / human hours')),
          rules: box(heading('Caught by your rules')),
          tiles: box(document.querySelector('.xt-dash-tiles')),
          effort: box(heading('Effort by type')),
          environment: box(heading('Environment')),
          sessions: box(heading('Sessions')),
          tokens: box(heading('Tokens per day')),
          openDetails: document.querySelectorAll('.xt-dashboard details[open]').length,
        };
      });
      expect(layout.periodBeforeRange).toBe(true);
      expect(layout.periodRowMatches).toBe(true);
      // Hero and rules share a row with equal height, then tiles, effort/environment and sessions.
      expect(layout.rules.top).toBe(layout.hero.top);
      expect(layout.rules.height).toBe(layout.hero.height);
      expect(layout.hero.height).toBeLessThanOrEqual(120);
      expect(layout.tiles.top).toBeGreaterThan(layout.hero.bottom);
      expect(layout.effort.top).toBeGreaterThan(layout.tiles.bottom);
      expect(layout.environment.top).toBe(layout.effort.top);
      expect(layout.effort.width / layout.environment.width).toBeCloseTo(1.6, 1);
      expect(layout.sessions.top).toBeGreaterThan(layout.effort.bottom);
      expect(layout.tokens.top).toBeGreaterThan(layout.sessions.bottom);
      expect(layout.openDetails).toBe(0);
      // At 1440x900 the primary composition, through the sessions lanes, fits in the first view.
      if (width === 1440) expect(layout.sessions.bottom).toBeLessThanOrEqual(height);
      const result = await measure(page);
      expect(result.scrollWidth).toBe(width);
      expect(result.outletOverflow).toBeLessThanOrEqual(0);
      expect(result.sidebar).toBe(228);
      expect(result.topbar).toBe(44);
      expect(result.main).toBe(width - 228);
      expect(result.h1).toEqual({ x: 245, y: 57, height: 29 });
      expect(result.clipped).toEqual([]);
      expect(result.tiles.map((tile) => tile.text)).toEqual(labels);
      for (const tile of result.tiles) expect(tile.fits, tile.text!).toBe(true);
      // Four across when there is room, otherwise a two-by-two grid.
      const rows = new Set(result.tiles.map((tile) => Math.round(tile.tile.y))).size;
      expect(rows).toBe(width === 1440 ? 1 : 2);
      await info.attach('measurements', { body: JSON.stringify(result, null, 2) });
      await page.screenshot({ path: info.outputPath(`dashboard-${width}-${scheme}.png`) });
      await page.screenshot({
        path: info.outputPath(`dashboard-${width}-${scheme}-full.png`),
        fullPage: true,
      });
      // The outlet scrolls vertically; everything down to the lanes is reachable.
      await page.getByTestId('lanes-disclosure').scrollIntoViewIfNeeded();
      await expect(page.getByTestId('lanes-disclosure')).toBeInViewport();
      await page.screenshot({ path: info.outputPath(`dashboard-${width}-${scheme}-end.png`) });
      // Expanded supplementary details keep real data readable without clipping.
      for (const summary of await page.locator('.xt-dash-extra-details > summary').all())
        await summary.click();
      await expect(page.getByTestId('cost-total')).toBeVisible();
      await expect(page.getByTestId('usage-coverage')).toBeVisible();
      await expect(page.getByRole('group', { name: 'Output tokens per day' })).toBeVisible();
      const expanded = await measure(page);
      expect(expanded.scrollWidth).toBe(width);
      expect(expanded.outletOverflow).toBeLessThanOrEqual(0);
      expect(expanded.clipped).toEqual([]);
      await page.getByTestId('dashboard-index').scrollIntoViewIfNeeded();
      await page.screenshot({
        path: info.outputPath(`dashboard-${width}-${scheme}-expanded.png`),
      });
      expect(errors).toEqual([]);
    });

test('range presets update the Dashboard and sidebar together; custom stays disabled', async ({
  page,
}) => {
  await open(page, 1440, 900, 'light');
  const range = page.getByRole('radiogroup', { name: 'Date range' });
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(page.getByRole('button', { name: 'Custom range' })).toBeDisabled();
  const caption = page.locator('.xt-token-heading > span');
  await expect(caption).toHaveText('Recorded tokens · 7d');
  const bars = page.getByRole('group', { name: 'Output tokens per day' }).getByRole('img');
  await page.getByText('Show daily tokens', { exact: true }).click();
  const period = page.getByTestId('report-period');
  for (const days of [30, 14, 7] as const) {
    await range.getByRole('radio', { name: `${days}d` }).click();
    await expect(range.getByRole('radio', { name: `${days}d` })).toBeChecked();
    await expect(caption).toHaveText(`Recorded tokens · ${days}d`);
    await expect(bars).toHaveCount(reports[days].days.length);
    const [first, last] = [reports[days].days[0].date, reports[days].days.at(-1)!.date];
    const short = (date: string) =>
      new Date(`${date}T00:00:00Z`).toLocaleDateString('en-US', {
        month: 'short',
        day: 'numeric',
        timeZone: 'UTC',
      });
    await expect(period).toContainText(short(first));
    await expect(period).toContainText(short(last).split(' ')[1]);
    const total = reports[days].tokens_by_host[0].tokens.counters.total_tokens;
    await expect(page.getByLabel('Claude Code tokens: ' + total)).toBeVisible();
  }
  // Lanes use their fixed recent axis whatever range is selected.
  await expect(page.getByTestId('lanes-disclosure')).toHaveText(
    '1 active span in the last 48 hours, whatever range is selected. A session can have several spans.',
  );
  // Other routes keep the selection without drawing a range control.
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await expect(page.getByRole('radiogroup', { name: 'Date range' })).toHaveCount(0);
  await expect(caption).toHaveText('Recorded tokens · 7d');
  await expect(period).toHaveCount(0);
});

test('keyboard focus opens tile definitions and reaches the Sessions link', async ({ page }) => {
  await open(page, 1120, 720, 'dark');
  const tile = page.locator('.xt-stat-tile').filter({ hasText: 'Hands-off median' });
  await tile.focus();
  const tip = page.getByRole('tooltip');
  await expect(tip).toContainText('M-09');
  await expect(tip).toContainText('How long your agents run before they need you.');
  await expect(tip).toContainText('No surface is excluded for timestamp health.');
  await page.keyboard.press('Escape');
  const link = page.getByRole('link', { name: 'View sessions' });
  await link.focus();
  await expect(link).toBeFocused();
  await link.press('Enter');
  await expect(page).toHaveURL(/\/sessions$/);
});
