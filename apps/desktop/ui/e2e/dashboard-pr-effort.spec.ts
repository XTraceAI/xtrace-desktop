import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import {
  MANY_TYPES,
  withPrEffort,
  type SectionSpec,
} from '../src/app/dashboard/pr-effort.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The M-19 merged-PR tile, Effort and the manual refresh control, over
 * the generated F1 export (whose confirmed links were never refreshed) and
 * over test-only synthetic sections (not real history, not a design sample)
 * served in place of F1's section: five assignments and merge markers, and a
 * busy week with several models a day, a day above 24 h, a day no response
 * could price and a partly priced day. Every refresh here is the browser
 * fixture's: nothing reaches GitHub.
 */
const synthetic = structuredClone(fixture as FixtureExport);
synthetic.dashboards = synthetic.dashboards.map((report) => withPrEffort(report, MANY_TYPES));
// MANY_TYPES draws #5's last refresh as failed; the tile's counts say so too,
// so the Effort card shows its red ! and the refresh dialog is one click away.
for (const report of synthetic.dashboards) {
  const freshness = report.pr_effort.current.tile.freshness;
  freshness.refreshed -= 1;
  freshness.failed_after_refresh += 1;
}
const ATTENTION = 'Pull-request checks need your attention';

const HOUR = 3_600_000;
/** Day 2: 26 h over two models; day 3: nothing priced; day 5: partly priced. */
const BUSY: SectionSpec = {
  tile: { known_merged: 1 },
  markers: [{ number: 7, day: 2, workType: 'feat' }],
  groups: [
    {
      assignment: { kind: 'type', work_type: 'feat' },
      sessions: 3,
      days: {
        2: { agentMs: 20 * HOUR, usd: 300, model: 'gpt-6-astra' },
        5: { agentMs: 2 * HOUR, usd: 40, selected: 130, priced: 2, model: 'claude-opus-5-5' },
      },
    },
    {
      assignment: { kind: 'other' },
      sessions: 2,
      days: {
        2: { agentMs: 6 * HOUR, usd: 120, model: 'claude-opus-5-5' },
        3: { agentMs: HOUR, usd: 0, selected: 3, priced: 0 },
      },
    },
  ],
};
const busy = structuredClone(fixture as FixtureExport);
busy.dashboards = busy.dashboards.map((report) => withPrEffort(report, BUSY));

async function open(
  page: Page,
  {
    width,
    height,
    scheme = 'light',
    dense = true,
    data = synthetic,
  }: {
    width: number;
    height: number;
    scheme?: 'light' | 'dark';
    dense?: boolean;
    data?: FixtureExport;
  },
) {
  if (dense)
    await page.route('**/fixtures/F1.json?import', (route) =>
      route.fulfill({
        contentType: 'text/javascript',
        body: `export default ${JSON.stringify(data)};`,
      }),
    );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByTestId('effort-by-type')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const card = (page: Page) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Effort', exact: true }),
  });

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`effort fits beside Overview at ${width}x${height} ${scheme}`, async ({ page }, info) => {
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      page.on('console', (message) => {
        if (message.type() === 'error') errors.push(message.text());
      });
      await open(page, { width, height, scheme });
      const layout = await page.evaluate(() => {
        const section = (name: string) =>
          [...document.querySelectorAll('.xt-dashboard h2')]
            .find((h) => h.textContent === name)!
            .closest('section')!
            .getBoundingClientRect();
        const effort = section('Effort');
        // Anything drawn outside the card's box, by class and text.
        const outside = [
          ...document.querySelectorAll<HTMLElement>('.xt-effort-card *:not(.sr-only)'),
        ]
          .filter((element) => {
            const box = element.getBoundingClientRect();
            // Base UI's visually hidden 1px radio inputs are not drawn, and a
            // table scrolls inside its own container, which must fit.
            if (box.width <= 1 && box.height <= 1) return false;
            if (element.parentElement?.closest('.xt-dash-table-scroll')) return false;
            return (
              box.width > 0 && (box.left < effort.left - 0.5 || box.right > effort.right + 0.5)
            );
          })
          .map(
            (element) =>
              `${element.tagName}.${element.className}: ${element.textContent?.slice(0, 40)}`,
          );
        const clipped = [...document.querySelectorAll<HTMLElement>('.xt-dashboard *')]
          .filter((element) => {
            if (element.closest('.xt-dash-table-scroll, .xt-table-scroll, .sr-only')) return false;
            if (
              element.matches(
                '.xt-env-list[data-compact] .xt-env-name, .xt-env-list[data-compact] .xt-kind',
              )
            )
              return false;
            const style = getComputedStyle(element);
            const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
            return clips && element.scrollWidth > element.clientWidth + 1;
          })
          .map((element) => `${element.className}: ${element.textContent}`);
        const marker = document.querySelector('.xt-effort-marker')!;
        const markerStyle = getComputedStyle(marker);
        return {
          effort,
          overview: section('Overview'),
          sessions: section('Sessions'),
          outside,
          clipped,
          days: document.querySelectorAll('.xt-effort-card .xt-effort-column').length,
          bars: document.querySelectorAll('.xt-effort-card .xt-effort-bar').length,
          headline: document.querySelector('.xt-effort-card .xt-effort-headline')?.textContent,
          // The scale's top label rises above the plot; it must clear the total.
          headlineClearance:
            document
              .querySelector('.xt-effort-card .xt-effort-scale [data-at="top"]')!
              .getBoundingClientRect().top -
            document.querySelector('.xt-effort-card .xt-effort-headline')!.getBoundingClientRect()
              .bottom,
          markers: [...document.querySelectorAll('.xt-effort-card .xt-effort-marker')].map(
            (badge) => badge.textContent,
          ),
          plot: document.querySelector('.xt-effort-card .xt-effort-plot')!.getBoundingClientRect()
            .height,
          scale: [...document.querySelectorAll('.xt-effort-card .xt-effort-scale span')].map(
            (label) => label.textContent,
          ),
          ticks: document.querySelectorAll('.xt-effort-card .xt-effort-ticks span').length,
          markerColors: [markerStyle.color, markerStyle.backgroundColor],
          scrollWidth: document.documentElement.scrollWidth,
        };
      });
      await page.screenshot({ path: info.outputPath(`effort-${width}-${scheme}.png`) });
      await card(page).screenshot({ path: info.outputPath(`effort-card-${width}-${scheme}.png`) });
      await info.attach('effort-layout', {
        body: JSON.stringify(layout, null, 2),
        contentType: 'application/json',
      });
      // The range's total above one bar per day with agent time (four of
      // seven days); three marker days counting five pull requests; a usable
      // plot with its scale and a seven-day axis; all inside the card.
      expect(layout.headline).toBe('0h50m agentlast 7 days');
      expect(layout.days).toBe(7);
      expect(layout.headlineClearance).toBeGreaterThanOrEqual(0);
      expect(layout.bars).toBe(4);
      expect(layout.markers).toEqual(['1', '2', '2']);
      expect(layout.plot).toBeGreaterThanOrEqual(50);
      expect(layout.scale).toEqual(['0.4 h', '0.2 h', '0']);
      expect(layout.ticks).toBe(7);
      expect(layout.outside).toEqual([]);
      expect(layout.clipped).toEqual([]);
      expect(layout.scrollWidth).toBe(width);
      expect(layout.markerColors[0]).not.toBe(layout.markerColors[1]);
      // The 1.6:1 row keeps its place and one height; Sessions stays in view at both sizes.
      expect(layout.overview.top).toBe(layout.effort.top);
      expect(layout.effort.width / layout.overview.width).toBeCloseTo(1.6, 1);
      expect(layout.effort.height).toBe(layout.overview.height);
      expect(layout.sessions.top).toBeGreaterThan(layout.effort.bottom);
      expect(layout.sessions.bottom).toBeLessThanOrEqual(height);
      // The refresh disclosure fits the window without horizontal overflow.
      await card(page).getByRole('button', { name: ATTENTION }).click();
      const dialog = page.getByRole('dialog', { name: 'Refresh pull-request facts' });
      await expect(dialog.getByRole('list', { name: 'Indexed pull requests' })).toBeVisible();
      const box = (await dialog.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(box.height).toBeLessThanOrEqual(height);
      expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      await page.screenshot({ path: info.outputPath(`effort-${width}-${scheme}-refresh.png`) });
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      expect(errors).toEqual([]);
    });

test('switches the effort measure from the keyboard', async ({ page }) => {
  await open(page, { width: 1440, height: 900 });
  const agent = card(page).getByRole('radio', { name: 'agent h' });
  await agent.focus();
  await expect(agent).toHaveAttribute('aria-checked', 'true');
  await page.keyboard.press('ArrowRight');
  // agent h, then human h, then cost.
  await expect(card(page).getByRole('radio', { name: 'human h' })).toBeFocused();
  await expect(page.getByTestId('human-timeline')).toBeVisible();
  await page.keyboard.press('ArrowRight');
  const dollars = card(page).getByRole('radio', { name: 'cost' });
  await expect(dollars).toHaveAttribute('aria-checked', 'true');
  await expect(dollars).toBeFocused();
  // The chart and the total above it now state dollars.
  await expect(page.getByTestId('effort-chart')).toHaveAttribute(
    'aria-label',
    /^Effort chart: daily dollars, one bar per day/,
  );
  await expect(page.getByTestId('effort-total')).toHaveText('$1.00');
  await expect(page.getByTestId('effort-cohort')).toHaveCount(0);
});

for (const scheme of ['light', 'dark'] as const)
  test(`a day's card lists every model, by hover and by keyboard, ${scheme}`, async ({
    page,
  }, info) => {
    await open(page, { width: 1440, height: 900, scheme, data: busy });
    const effort = card(page);
    // The card's definition says what it shows, not per-type series.
    await page.getByRole('button', { name: 'Effort definition' }).focus();
    await expect(page.getByRole('tooltip')).toContainText(
      'This card covers every session in the range, one bar per day, split by model, whether or not it is linked to a PR.',
    );
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Effort definition' }).blur();
    // Agent hours: the range's total, the 24 h line and a note for the day above it.
    await expect(effort.getByTestId('effort-headline')).toHaveText(
      /^29h00m agentlast 7 days1 day above 24 h$/,
    );
    await expect(effort.getByTestId('effort-reference')).toHaveText('24 h');
    // The scale's top label, "30 h", stays clear of the total above it.
    const [total, topLabel] = await Promise.all([
      effort.getByTestId('effort-headline').boundingBox(),
      effort.locator('.xt-effort-scale [data-at="top"]').boundingBox(),
    ]);
    expect(topLabel!.y).toBeGreaterThanOrEqual(total!.y + total!.height);
    const day = (date: string) => effort.locator(`[data-testid="effort-day"][data-date="${date}"]`);
    const dates = await effort
      .getByTestId('effort-day')
      .evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-date')!));
    const [, , second, third, , fifth] = dates;
    await expect(day(second!)).toHaveAccessibleName(
      `${second}: 26h00m. gpt-6-astra 20h00m (77%), claude-opus-5-5 6h00m (23%). Above 24 h: agents ran at the same time. merged #7`,
    );
    await day(second!).hover();
    // The open card only: a closing one can still be leaving as the next opens.
    const tip = page.locator('[data-testid="effort-day-card"][data-open]');
    await expect(tip).toBeVisible();
    await expect(tip.getByTestId('effort-day-model')).toHaveText([
      'gpt-6-astra20h00m77%',
      'claude-opus-5-56h00m23%',
    ]);
    await expect(tip).toContainText('Above 24 h: agents ran at the same time');
    await card(page).screenshot({ path: info.outputPath(`effort-card-hours-${scheme}.png`) });
    await page.mouse.move(0, 0);
    await expect(tip).toHaveCount(0);

    // Cost: the total is a priced subtotal; the partly priced day carries a +
    // and names its unpriced responses; the day nothing could price has no bar.
    await effort.getByRole('radio', { name: 'cost' }).click();
    await expect(effort.getByTestId('effort-headline')).toHaveText(
      /^\$460\+last 7 days131 responses have no price$/,
    );
    await expect(effort.getByTestId('effort-reference')).toHaveCount(0);
    await expect(day(fifth!).getByTestId('effort-partial')).toHaveText('+');
    await expect(day(fifth!)).toHaveAccessibleName(
      `${fifth}: $40.00+. claude-opus-5-5 $40.00 (100%). No price: 128 codex-auto-review responses`,
    );
    await expect(day(third!).getByTestId('effort-bar')).toHaveCount(0);
    await day(third!).hover();
    await expect(tip.locator('.xt-effort-tip-head')).toHaveText(/cost unknown$/);
    await expect(tip).toContainText('No price: 3 codex-auto-review responses');
    await page.mouse.move(0, 0);
    await expect(day(third!)).toHaveAccessibleName(
      `${third}: cost unknown. No price: 3 codex-auto-review responses`,
    );
    // The keyboard reaches each day in turn and opens the same card.
    await day(dates[4]!).focus();
    await page.keyboard.press('Tab');
    await expect(day(fifth!)).toBeFocused();
    await expect(tip).toBeVisible();
    await expect(tip).toContainText('No price: 128 codex-auto-review responses');
    await expect(tip.getByTestId('effort-day-model')).toHaveText(['claude-opus-5-5$40.00100%']);
    await page.keyboard.press('Shift+Tab');
    await expect(day(dates[4]!)).toBeFocused();
    await expect(tip).not.toContainText('No price: 128');
    await day(dates[4]!).blur();
    await day(second!).hover();
    await expect(tip).toContainText('$420');
    await card(page).screenshot({ path: info.outputPath(`effort-card-cost-${scheme}.png`) });
  });

test('refreshes the F1 fixture from the keyboard and updates the tile after the whole selection', async ({
  page,
}) => {
  await open(page, { width: 1440, height: 900, dense: false });
  const tile = page.getByTestId('overview-tile').filter({ hasText: 'Merged PRs' });
  await expect(tile).toContainText('2 PRs not checked yet');
  // F1's links were never checked and this source never checks on its own,
  // so the red ! is there.
  const trigger = card(page).getByRole('button', { name: ATTENTION });
  await trigger.focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Refresh pull-request facts' });
  await expect(dialog).toContainText('It only reads: nothing is written to GitHub');
  const boxes = dialog.getByRole('checkbox');
  await expect(boxes).toHaveCount(3);
  for (const index of [0, 1, 2]) {
    await boxes.nth(index).focus();
    await page.keyboard.press('Space');
    await expect(boxes.nth(index)).toBeChecked();
  }
  await dialog.getByRole('button', { name: 'Refresh 3 pull requests' }).focus();
  await page.keyboard.press('Enter');
  await expect(dialog.getByTestId('pr-refresh-report')).toHaveText(
    'Requested 3: 2 checked, 1 could not be checked, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
  await expect(dialog).toContainText('Could not be checked: rate limited; earlier facts are kept');
  // While the dialog is open the mark stays, so its result stays reachable.
  // (The page behind the dialog is hidden from the accessibility tree, so
  // the mark is found by its test id rather than through the card's heading.)
  await expect(page.getByTestId('pr-attention')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  // #12's check failed: it was checked, but could not be.
  await expect(tile).toContainText('0so far1 could not be checked');
  // A rate limit is about the whole run and temporary, so the manual refresh
  // does not mark #12 as tried: the red ! still asks, and focus is back on it.
  // Nothing is left under the chart.
  const mark = card(page).getByTestId('pr-attention');
  await expect(mark).toHaveAttribute('data-state', 'attention');
  await expect(mark).toBeFocused();
  await expect(card(page).getByTestId('pr-refresh')).toHaveCount(0);
  await expect(trigger).toHaveAttribute('data-state', 'attention');
  await page.keyboard.press('Enter');
  await expect(dialog.getByTestId('pr-refresh')).toContainText(
    'Confirmed-linked pull requests: 1 checked, 1 could not be checked',
  );
  await page.keyboard.press('Escape');
});
