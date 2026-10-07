import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import { syntheticMergedPage, syntheticOverview } from '../src/app/dashboard/overview.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The Overview card and the Effort card's "human h" timeline over the
 * test-only synthetic report (not real history) served in place of the F1
 * export. `OVERVIEW_SHOTS=DIR` also saves a screenshot of each case there.
 */
const synthetic = structuredClone(fixture as FixtureExport);
synthetic.dashboards = synthetic.dashboards.map((report) => syntheticOverview(report));
synthetic.pr_analytics = synthetic.pr_analytics.map(syntheticMergedPage);
const shots = process.env.OVERVIEW_SHOTS;

async function open(page: Page, scheme: 'light' | 'dark', width = 1440, height = 900) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(synthetic)};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByTestId('overview-grid')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const range = (page: Page, value: '7d' | '14d' | '30d') =>
  page.getByRole('radio', { name: value, exact: true }).click();
const measure = (page: Page, name: 'agent h' | 'human h' | 'cost') =>
  page.getByRole('radio', { name, exact: true }).click();
const effortCard = (page: Page) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Effort', exact: true }),
  });

/** The timeline's geometry: rows, their height, the scroll position and the bars. */
async function timeline(page: Page) {
  return page.getByTestId('human-timeline-body').evaluate((body) => {
    const rows = [...body.querySelectorAll<HTMLElement>('[data-testid="human-day"]')];
    const tracks = rows.map((row) => row.querySelector('.xt-human-track')!.getBoundingClientRect());
    const axis = document.querySelector('.xt-human-axis')!.getBoundingClientRect();
    return {
      rows: rows.length,
      rowHeight: rows[0]!.getBoundingClientRect().height,
      trackHeight: Math.max(...tracks.map((track) => track.height)),
      clientHeight: body.clientHeight,
      scrollHeight: body.scrollHeight,
      scrollTop: body.scrollTop,
      axisAboveBody: axis.bottom <= body.getBoundingClientRect().top + 1,
      lastVisible:
        rows.at(-1)!.getBoundingClientRect().bottom <= body.getBoundingClientRect().bottom + 1,
      lastDate: rows.at(-1)!.dataset.date,
    };
  });
}

for (const scheme of ['light', 'dark'] as const)
  test(`the Overview card and the human h timeline at 7d and 30d, ${scheme}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, scheme);
    // The tile row and the Environment card are gone; Overview holds the four.
    await expect(page.locator('.xt-dash-tiles')).toHaveCount(0);
    await expect(page.getByRole('heading', { level: 2, name: 'Environment' })).toHaveCount(0);
    await expect(page.getByTestId('overview-tile')).toHaveCount(4);
    await expect(
      page.getByTestId('overview-tile').evaluateAll((tiles) => tiles.map((t) => t.dataset.label)),
    ).resolves.toEqual(['Leverage', 'Concurrency', 'Merged PRs', 'Hands-off median']);
    await expect(page.getByTestId('overview-prs').locator('li').first()).toBeVisible();
    await expect(page.getByTestId('day-line')).toHaveCount(3);
    const heights: number[] = [];
    for (const preset of ['7d', '30d'] as const) {
      await range(page, preset);
      await measure(page, 'agent h');
      await expect(page.getByTestId('effort-chart')).toBeVisible();
      heights.push((await effortCard(page).boundingBox())!.height);
      if (shots)
        await page.screenshot({ path: `${shots}/dashboard-${preset}-agent-${scheme}.png` });
      await measure(page, 'human h');
      await expect(page.getByTestId('human-timeline')).toBeVisible();
      heights.push((await effortCard(page).boundingBox())!.height);
      const geometry = await timeline(page);
      const days = synthetic.dashboards.find((r) => r.window.days === Number(preset.slice(0, -1)))!
        .human_hours.current.by_day;
      expect(geometry.rows).toBe(days.length);
      // The newest day is the last row, in view; the hour axis stays above the rows.
      expect(geometry.lastDate).toBe(days.at(-1)!.date);
      expect(geometry.lastVisible).toBe(true);
      expect(geometry.axisAboveBody).toBe(true);
      // Eight rows fill the plot; the bars keep space between rows.
      expect(geometry.rowHeight).toBeCloseTo(
        Math.max(28, Math.floor(geometry.clientHeight / 8)),
        0,
      );
      expect(geometry.trackHeight).toBeLessThanOrEqual(30);
      expect(geometry.trackHeight).toBeLessThan(geometry.rowHeight);
      if (preset === '30d') {
        // Longer ranges scroll, opened on today.
        expect(geometry.scrollHeight).toBeGreaterThan(geometry.clientHeight);
        expect(geometry.scrollTop + geometry.clientHeight).toBeGreaterThanOrEqual(
          geometry.scrollHeight - 1,
        );
      } else expect(geometry.scrollHeight).toBeLessThanOrEqual(geometry.clientHeight + 1);
      if (shots)
        await page.screenshot({ path: `${shots}/dashboard-${preset}-human-${scheme}.png` });
    }
    // The Effort card keeps one height in every range and view.
    for (const height of heights) expect(height).toBeCloseTo(heights[0]!, 0);
    expect(errors).toEqual([]);
  });

test('the Effort switch and a day of the timeline work from the keyboard', async ({ page }) => {
  await open(page, 'light');
  const agent = page.getByRole('radio', { name: 'agent h', exact: true });
  await agent.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('radio', { name: 'human h', exact: true })).toBeChecked();
  await expect(page.getByTestId('human-timeline')).toBeVisible();
  // One tab stop: today's row. Tab reaches it, arrows move between days,
  // and the focused day's card opens.
  const timeline = page.getByTestId('human-timeline');
  await expect(timeline.locator('[tabindex="0"]')).toHaveCount(1);
  const days = page.getByTestId('human-day');
  const last = days.last();
  await expect(last).toHaveAttribute('tabindex', '0');
  await expect(last).toHaveAttribute('aria-label', /^Sep 7: /);
  await last.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(last).toBeFocused();
  // These visual-only day popups are aria-hidden and their triggers carry
  // no described-by/controls link. Select the active card for the fixed day,
  // so another day's closing animation cannot make the query ambiguous.
  const activeDayCard = (day: string) =>
    page.locator('.xt-effort-tip[data-open]').filter({
      has: page.locator('.xt-effort-tip-head > span', { hasText: new RegExp(`^${day}$`) }),
    });
  await expect(page.locator('.xt-effort-tip[data-open]')).toHaveCount(1);
  await expect(activeDayCard('Sep 7')).toHaveCount(1);
  await expect(activeDayCard('Sep 7')).toBeVisible();
  await expect(activeDayCard('Sep 7').locator('.xt-effort-tip-head')).toContainText('Sep 7');
  await page.keyboard.press('ArrowUp');
  const before = days.nth(-2);
  await expect(before).toBeFocused();
  await expect(before).toHaveAttribute('aria-label', /^Sep 6: /);
  await expect(page.locator('.xt-effort-tip[data-open]')).toHaveCount(1);
  await expect(activeDayCard('Sep 6')).toHaveCount(1);
  await expect(activeDayCard('Sep 6').locator('.xt-effort-tip-head')).toContainText('Sep 6');
  await expect(timeline.locator('[tabindex="0"]')).toHaveCount(1);
  await page.keyboard.press('Home');
  await expect(days.first()).toBeFocused();
});
