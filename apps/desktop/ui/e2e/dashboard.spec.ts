import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

const METHOD = 'How effort is counted · daily values';

const reports = Object.fromEntries(
  fixture.dashboards.map((report) => [report.window.days, report]),
);
const sizes = [
  { width: 1440, height: 900 },
  { width: 1120, height: 720 },
] as const;
const labels = ['Leverage', 'Concurrency', 'Merged PRs', 'Hands-off median'];

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
        if (element.closest('.xt-dash-table-scroll, .xt-table-scroll, .sr-only')) return false;
        const style = getComputedStyle(element);
        const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
        return clips && element.scrollWidth > element.clientWidth + 1;
      })
      .map((element) => `${element.className}: ${element.textContent}`);
    const tiles = [...document.querySelectorAll('[data-testid="overview-tile"]')].map((tile) => {
      const label = tile.querySelector<HTMLElement>(
        '.xt-overview-label > span:not(.xt-metric-icon)',
      )!;
      return {
        text: label.textContent,
        fits: label.scrollWidth <= label.clientWidth,
        label: label.getBoundingClientRect().width,
        tile: tile.getBoundingClientRect(),
      };
    });
    const h1 = document.querySelector('.xt-dashboard h1')!.getBoundingClientRect();
    // A collided lane axis would read as the wrong time, and absolute ticks
    // never trip the clipping check above, so measure them directly.
    const ticks = [...document.querySelectorAll('.xt-lanes-axis > span:not(.sr-only)')]
      .map((tick) => tick.getBoundingClientRect())
      .filter((box) => box.width > 0);
    const collidedTicks = ticks.filter(
      (box, index) => index > 0 && box.left < ticks[index - 1].right + 4,
    ).length;
    return {
      tickCount: ticks.length,
      collidedTicks,
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
          headings: [...document.querySelectorAll('.xt-dashboard h2')].map((h) => h.textContent),
          summary: box(document.querySelector('[data-testid="dashboard-summary"]')),
          gap: parseFloat(getComputedStyle(document.querySelector('.xt-dashboard')!).rowGap),
          tileRow: document.querySelector('.xt-dash-tiles') !== null,
          effort: box(heading('Effort')),
          overview: box(heading('Overview')),
          sessions: box(heading('Sessions')),
          foot: document.querySelector('.xt-dash-foot, [data-testid="dashboard-index"]') !== null,
          details: document.querySelectorAll('.xt-dashboard details').length,
          dialogs: document.querySelectorAll('[role="dialog"]').length,
        };
      });
      expect(layout.periodBeforeRange).toBe(true);
      expect(layout.periodRowMatches).toBe(true);
      // Agent / human hours and Caught by your rules are not drawn, and no row
      // is kept for them, nor for the old row of tiles: Effort and Overview
      // follow the summary at the page's own gap, then Sessions.
      expect(layout.headings).toEqual(['Effort', 'Overview', 'Sessions']);
      expect(layout.tileRow).toBe(false);
      expect(layout.effort.top - layout.summary.bottom).toBeCloseTo(layout.gap, 0);
      expect(layout.overview.top).toBe(layout.effort.top);
      expect(layout.effort.width / layout.overview.width).toBeCloseTo(1.6, 1);
      expect(layout.sessions.top).toBeGreaterThan(layout.effort.bottom);
      // Sessions is the last block: no measurement or index line follows it.
      expect(layout.foot).toBe(false);
      // Nothing expands in place, and nothing is open over the page.
      expect(layout.details).toBe(0);
      expect(layout.dialogs).toBe(0);
      // The whole composition, through Sessions, fits the window at both sizes.
      expect(layout.sessions.bottom).toBeLessThanOrEqual(height);
      const result = await measure(page);
      expect(result.scrollWidth).toBe(width);
      expect(result.outletOverflow).toBeLessThanOrEqual(0);
      expect(result.sidebar).toBe(228);
      expect(result.topbar).toBe(44);
      expect(result.main).toBe(width - 228);
      expect(result.h1).toEqual({ x: 245, y: 52, height: 29 });
      expect(result.clipped).toEqual([]);
      // The lane axis keeps the start and the dated end, and adds the midpoint
      // and quarters only where the column holds a live axis's widest times
      // (dashboard-lanes.spec.ts): at these widths `10:59 PM` ticks overlapped
      // the midpoint at 1120 and the quarters at 1440, so F1's shorter whole
      // hours show the same ticks. No two ticks collide, in WebKit's wider
      // metrics as well as Chromium's.
      expect(result.tickCount).toBe(width === 1440 ? 3 : 2);
      expect(result.collidedTicks).toBe(0);
      expect(result.tiles.map((tile) => tile.text)).toEqual(labels);
      for (const tile of result.tiles) expect(tile.fits, tile.text!).toBe(true);
      // Two by two at both supported widths.
      const rows = new Set(result.tiles.map((tile) => Math.round(tile.tile.y))).size;
      const columns = new Set(result.tiles.map((tile) => Math.round(tile.tile.x))).size;
      expect([rows, columns]).toEqual([2, 2]);
      await info.attach('measurements', { body: JSON.stringify(result, null, 2) });
      await page.screenshot({ path: info.outputPath(`dashboard-${width}-${scheme}.png`) });
      await page.screenshot({
        path: info.outputPath(`dashboard-${width}-${scheme}-full.png`),
        fullPage: true,
      });
      // Nothing scrolls: the lanes are in view as drawn.
      await expect(page.getByRole('table', { name: 'Session lanes' })).toBeInViewport();
      expect(await page.evaluate(() => document.querySelector('.xt-shell-outlet')!.scrollTop)).toBe(
        0,
      );
      // Each supplementary measurement opens over the page from the card it
      // belongs to, with real data readable and nothing clipped, and the page
      // under it keeps its geometry.
      for (const [card, trigger, title, probe] of [
        [
          'Effort',
          /^Effort definition$/,
          METHOD,
          page.getByRole('group', { name: 'Output tokens per day' }),
        ],
        ['Effort', /^Effort definition$/, METHOD, page.getByTestId('cost-total')],
        ['Sessions', /^Coverage/, 'Coverage', page.getByTestId('usage-coverage')],
      ] as const) {
        await page
          .getByRole('region', { name: card, exact: true })
          .getByRole('button', { name: trigger })
          .click();
        const dialog = page.getByRole('dialog', { name: title });
        await probe.scrollIntoViewIfNeeded();
        await expect(probe).toBeVisible();
        await expect(probe).toBeInViewport();
        const clipped = await dialog.evaluate((node) =>
          [...node.querySelectorAll<HTMLElement>('*')]
            .filter((element) => {
              if (element.closest('.xt-dash-table-scroll, .sr-only')) return false;
              const style = getComputedStyle(element);
              const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
              return clips && element.scrollWidth > element.clientWidth + 1;
            })
            .map((element) => `${element.className}: ${element.textContent}`),
        );
        expect(clipped).toEqual([]);
        if (title === 'Coverage')
          await page.screenshot({
            path: info.outputPath(`dashboard-${width}-${scheme}-detail.png`),
          });
        await page.keyboard.press('Escape');
        await expect(dialog).toHaveCount(0);
      }
      const after = await measure(page);
      expect(after.scrollWidth).toBe(width);
      expect(after.outletOverflow).toBeLessThanOrEqual(0);
      expect(after.clipped).toEqual([]);
      expect(errors).toEqual([]);
    });

test('range presets update the Dashboard while account usage stays separate', async ({ page }) => {
  await open(page, 1440, 900, 'light');
  const range = page.getByRole('radiogroup', { name: 'Date range' });
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(page.getByRole('button', { name: 'Custom range' })).toBeDisabled();
  const usage = page.getByRole('region', { name: 'Account usage' });
  await expect(usage).toBeVisible();
  const bars = page.getByRole('group', { name: 'Output tokens per day' }).getByRole('img');
  const period = page.getByTestId('report-period');
  for (const days of [30, 14, 7] as const) {
    await range.getByRole('radio', { name: `${days}d` }).click();
    await expect(range.getByRole('radio', { name: `${days}d` })).toBeChecked();
    await expect(usage).toContainText('Usage source unavailable');
    // The daily chart is behind Effort's ⓘ, one dialog away, and follows the range.
    await page.getByRole('button', { name: 'Effort definition', exact: true }).click();
    await expect(bars).toHaveCount(reports[days].days.length);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    const [first, last] = [reports[days].days[0].date, reports[days].days.at(-1)!.date];
    const short = (date: string) =>
      new Date(`${date}T00:00:00Z`).toLocaleDateString('en-US', {
        month: 'short',
        day: 'numeric',
        timeZone: 'UTC',
      });
    await expect(period).toContainText(short(first));
    await expect(period).toContainText(short(last).split(' ')[1]);
    await expect(page.getByTestId('report-period')).toBeVisible();
  }
  // Lanes keep their fixed recent axis whatever range is selected; its header
  // names it. No caption under the rows and no line under the card's title.
  await expect(page.locator('.xt-lanes-activity-label')).toHaveText('Activity');
  await expect(page.getByTestId('lanes-disclosure')).toHaveCount(0);
  await expect(page.getByText('One row per session')).toHaveCount(0);
  // Sessions measures the same window, so it keeps the control and the period.
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(usage).toContainText('Usage source unavailable');
  await expect(period).toHaveCount(1);
  // The pull requests report measures it too; its cached inventory view,
  // like any route that measures no window, keeps the selection without it.
  await page.getByRole('button', { name: 'Pull requests', exact: true }).click();
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(period).toHaveCount(1);
  await page.getByRole('radio', { name: 'Cached inventory' }).click();
  await expect(page.getByRole('radiogroup', { name: 'Date range' })).toHaveCount(0);
  await expect(usage).toContainText('Usage source unavailable');
  await expect(period).toHaveCount(0);
});

/** Each control Tab reaches inside the Dashboard, in order, by its accessible name or text. */
async function dashboardTabOrder(page: Page) {
  await page
    .getByRole('radiogroup', { name: 'Date range' })
    .getByRole('radio', { checked: true })
    .focus();
  const inside = () =>
    page.evaluate(() => {
      const active = document.activeElement;
      if (!active?.closest('.xt-dashboard')) return null;
      return (active.getAttribute('aria-label') ?? active.textContent ?? '').trim();
    });
  const order: string[] = [];
  for (let presses = 0; presses < 200; presses += 1) {
    await page.keyboard.press('Tab');
    const name = await inside();
    if (name === null) {
      if (order.length > 0) break;
      continue;
    }
    order.push(name);
  }
  return order;
}

for (const scheme of ['light', 'dark'] as const)
  test(`Tab skips the hidden panels and reaches the remaining controls at 1120x720 ${scheme}`, async ({
    page,
  }) => {
    await open(page, 1120, 720, scheme);
    const range = page.getByRole('radiogroup', { name: 'Date range' });
    for (const days of [7, 30] as const) {
      await range.getByRole('radio', { name: `${days}d` }).click();
      await expect(page.getByTestId('report-period')).toBeVisible();
      const order = await dashboardTabOrder(page);
      // The favorite model leads; each Overview tile's definition is a tab
      // stop, in reading order; nothing of the hidden pair is one.
      expect(order[0]).toBe('fixture-model-v1');
      expect(
        order
          .filter((name) => labels.some((label) => name === `${label} definition`))
          .map((name) => name.replace(/ definition$/, '')),
      ).toEqual(labels);
      for (const name of ['Effort definition', 'Overview definition', 'Sessions definition'])
        expect(order).toContain(name);
      expect(order.some((name) => name.startsWith('Coverage'))).toBe(true);
      expect(order.join(' | ')).not.toMatch(
        /Agent \/ human hours|Caught by your rules|Agent hours|Human-in-the-loop|Agent to human ratio|Daily values/,
      );
    }
  });

test('keyboard focus opens tile definitions and reaches the Sessions link', async ({ page }) => {
  await open(page, 1120, 720, 'dark');
  const tile = page.getByRole('button', { name: 'Hands-off median definition' });
  await tile.focus();
  const tip = page.getByRole('tooltip');
  await expect(tip).toContainText('The median time an agent worked on its own');
  await expect(tip).not.toContainText('M-09');
  await page.keyboard.press('Escape');
  const link = page.getByRole('link', { name: 'View sessions' });
  await link.focus();
  await expect(link).toBeFocused();
  await link.press('Enter');
  // Sessions measures over the selected window, so the link carries it.
  await expect(page).toHaveURL(/\/sessions\?range=7d$/);
});
