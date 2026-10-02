import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { MetricTile } from '../src/data/generated/MetricTile';

/**
 * Synthetic dense tile values (not real history): multi-digit means with percentage changes and
 * asides, served in place of the development F1 export. Labels must stay at full width.
 */
const measured = (tile: MetricTile, value: number, pct: number | null): MetricTile => ({
  ...tile,
  value,
  reason: null,
  current_n: 12,
  previous_n: 10,
  delta: { previous: value, pct, suppressed: pct === null },
});
// JSON imports widen literal unions; the export is the generated shape.
const dense = structuredClone(fixture as FixtureExport);
for (const report of dense.dashboards) {
  const { tiles } = report;
  tiles.agent_hours_per_day = measured(tiles.agent_hours_per_day, 12.4, 8.5);
  tiles.concurrency_mean = measured(tiles.concurrency_mean, 1.8, -12.3);
  tiles.concurrency_max = measured(tiles.concurrency_max, 8, null);
  tiles.merged_prs = measured(tiles.merged_prs, 23, 15.2);
  tiles.hands_off_median = measured(tiles.hands_off_median, 3.2, -4.1);
  tiles.hands_off_p90 = measured(tiles.hands_off_p90, 14.8, null);
}

async function serveDense(page: Page) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(dense)};`,
    }),
  );
}

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`dense tile values keep full labels at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      await serveDense(page);
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/dashboard');
      await expect(page.locator('.xt-stat-tile')).toHaveCount(4);
      await page.evaluate(() => document.fonts.ready);
      const tiles = await page.evaluate(() =>
        [...document.querySelectorAll<HTMLElement>('.xt-dash-tiles .xt-stat-tile')].map((tile) => {
          const fits = (selector: string) => {
            const element = tile.querySelector<HTMLElement>(selector);
            return element ? element.scrollWidth <= element.clientWidth : true;
          };
          const label = tile.querySelector<HTMLElement>('.xt-stat-label')!;
          const box = tile.getBoundingClientRect();
          const inside = [...tile.querySelectorAll<HTMLElement>('.xt-stat-label, .xt-stat-row')]
            .map((element) => element.getBoundingClientRect())
            .every((rect) => rect.left >= box.left && rect.right <= box.right + 0.5);
          return {
            text: tile.textContent,
            label: label.textContent,
            clientWidth: label.clientWidth,
            scrollWidth: label.scrollWidth,
            labelFits: label.scrollWidth <= label.clientWidth,
            asideFits: fits('.xt-stat-aside'),
            valueFits: fits('.xt-stat-value .xt-metric-cell'),
            inside,
          };
        }),
      );
      await info.attach('tiles', { body: JSON.stringify(tiles, null, 2) });
      await page.locator('.xt-dash-tiles').screenshot({
        path: info.outputPath(`dense-tiles-${width}-${scheme}.png`),
      });
      expect(tiles.map((tile) => tile.label)).toEqual([
        'Agent h/day',
        'Concurrency',
        'Merged PRs',
        'Hands-off median',
      ]);
      // Meaningful values stay visible: mean, change and max for Concurrency.
      expect(tiles[1].text).toContain('1.8mean▼12.3%max 8');
      expect(tiles[3].text).toContain('p90 14.8');
      for (const tile of tiles) {
        expect(tile.labelFits, `${tile.label}: ${tile.scrollWidth}/${tile.clientWidth}`).toBe(true);
        expect(tile.asideFits, tile.label).toBe(true);
        expect(tile.valueFits, tile.label).toBe(true);
        expect(tile.inside, tile.label).toBe(true);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    });
