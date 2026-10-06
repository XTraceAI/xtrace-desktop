import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardMetrics } from '../src/data/generated/DashboardMetrics';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { MetricTile } from '../src/data/generated/MetricTile';
import { CONCURRENCY_DEFINITION } from '../src/app/dashboard/overview';

/**
 * Synthetic tile values (not real history), served in place of the development F1 export, in
 * the Overview card's four tiles: `dense` has multi-digit values with percentage changes and
 * secondary values; `reference` has a three-digit change, and 32 pull requests not checked
 * yet, said in words; `unknown` has every tile unmeasured with its reason. Labels must stay
 * at full width, and every value, change and line under it stays inside its tile.
 */
const measured = (tile: MetricTile, value: number, pct: number | null): MetricTile => ({
  ...tile,
  value,
  reason: null,
  current_n: 12,
  previous_n: 10,
  delta: { previous: value, pct, suppressed: pct === null },
});
const unknown = (tile: MetricTile, reason: string): MetricTile => ({
  ...tile,
  value: null,
  reason,
});
type Shape = 'dense' | 'reference' | 'unknown';
const edits: Record<Shape, (report: DashboardMetrics) => void> = {
  dense: (report) => {
    const { tiles } = report;
    tiles.leverage = measured(tiles.leverage, 12.4, 8.5);
    tiles.concurrency_mean = measured(tiles.concurrency_mean, 1.8, -12.3);
    tiles.concurrency_max = measured(tiles.concurrency_max, 8, null);
    tiles.merged_prs = measured(tiles.merged_prs, 23, 15.2);
    // A measured count is a complete one: every linked pull request is checked.
    report.pr_effort.current.tile = {
      ...report.pr_effort.current.tile,
      complete: true,
      known_merged: 23,
      unknown_facts: 0,
      merged: 23,
    };
    tiles.hands_off_median = measured(tiles.hands_off_median, 3.2, -4.1);
    tiles.hands_off_p90 = measured(tiles.hands_off_p90, 14.8, null);
  },
  reference: (report) => {
    const { tiles } = report;
    tiles.leverage = measured(tiles.leverage, 30, 166.2);
    tiles.concurrency_mean = measured(tiles.concurrency_mean, 2.6, 112.4);
    tiles.concurrency_max = measured(tiles.concurrency_max, 8, null);
    tiles.merged_prs = unknown(tiles.merged_prs, 'Merged pull requests with unknown facts');
    report.pr_effort.current.tile = {
      ...report.pr_effort.current.tile,
      complete: false,
      known_merged: 0,
      unknown_facts: 32,
      freshness: { ...report.pr_effort.current.tile.freshness, never_attempted: 32 },
    };
    tiles.hands_off_median = measured(tiles.hands_off_median, 5.6, 298.6);
    tiles.hands_off_p90 = measured(tiles.hands_off_p90, 32.1, null);
  },
  unknown: ({ tiles }) => {
    tiles.leverage = unknown(tiles.leverage, 'No active spans');
    tiles.concurrency_mean = unknown(tiles.concurrency_mean, 'No overlapping spans');
    tiles.concurrency_max = unknown(tiles.concurrency_max, 'No overlapping spans');
    tiles.merged_prs = unknown(tiles.merged_prs, 'No cached pull request facts');
    tiles.hands_off_median = unknown(tiles.hands_off_median, 'No hands-off stretches');
    tiles.hands_off_p90 = unknown(tiles.hands_off_p90, 'No hands-off stretches');
  },
};

async function serve(page: Page, shape: Shape) {
  // JSON imports widen literal unions; the export is the generated shape.
  const out = structuredClone(fixture as FixtureExport);
  out.dashboards.forEach(edits[shape]);
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(out)};`,
    }),
  );
}

/** Each tile's number with its unit and change, then the line under it. */
const texts: Record<Shape, [string, string][]> = {
  dense: [
    ['12.4×▲8.5%', '0.4 agent h ÷ 0.3 your h'],
    ['1.8▼12.3%', 'max 8'],
    ['23▲15.2%', ''],
    ['3.2min▼4.1%', 'p90 14.8 min'],
  ],
  reference: [
    ['30×▲166.2%', '0.4 agent h ÷ 0.3 your h'],
    ['2.6▲112.4%', 'max 8'],
    // Nothing checked yet: words instead of a number, never a zero.
    ['32 PRs not checked yet', ''],
    ['5.6min▲298.6%', 'p90 32.1 min'],
  ],
  unknown: [
    ['—Unmeasured: No active spans.', '0.4 agent h ÷ 0.3 your h'],
    ['—Unmeasured: No overlapping spans.', 'No overlapping spans.'],
    ['2 PRs not checked yet', ''],
    ['—Unmeasured: No hands-off stretches.', 'No hands-off stretches.'],
  ],
};

for (const shape of ['dense', 'reference', 'unknown'] as const)
  for (const [width, height] of [
    [1440, 900],
    [1120, 720],
  ] as const)
    for (const scheme of ['light', 'dark'] as const)
      test(`${shape} Overview values keep full labels inside their tiles at ${width}x${height} ${scheme}`, async ({
        page,
      }, info) => {
        await serve(page, shape);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto('/dashboard');
        await expect(page.getByTestId('overview-tile')).toHaveCount(4);
        await page.evaluate(() => document.fonts.ready);
        const tiles = await page.evaluate(() =>
          [...document.querySelectorAll<HTMLElement>('[data-testid="overview-tile"]')].map(
            (tile) => {
              const label = tile.querySelector<HTMLElement>(
                '.xt-overview-label > span:not(.xt-metric-icon)',
              )!;
              const value = tile.querySelector<HTMLElement>('[data-testid="overview-value"]')!;
              const sub = tile.querySelector<HTMLElement>('.xt-overview-sub')!;
              const box = tile.getBoundingClientRect();
              const inside = [
                ...tile.querySelectorAll<HTMLElement>(
                  '.xt-overview-label, .xt-overview-value, .xt-overview-value *, .xt-overview-sub',
                ),
              ]
                .map((element) => element.getBoundingClientRect())
                .filter((rect) => rect.width > 0)
                .every(
                  (rect) =>
                    rect.left >= box.left &&
                    rect.right <= box.right + 0.5 &&
                    rect.top >= box.top &&
                    rect.bottom <= box.bottom + 0.5,
                );
              return {
                label: label.textContent,
                value: value.textContent,
                sub: sub.textContent,
                labelFits: label.scrollWidth <= label.clientWidth,
                valueFits: value.scrollWidth <= value.clientWidth,
                valueBelowLabel:
                  value.getBoundingClientRect().top >= label.getBoundingClientRect().bottom,
                top: box.top,
                height: box.height,
                inside,
              };
            },
          ),
        );
        await info.attach('tiles', { body: JSON.stringify(tiles, null, 2) });
        await page.getByTestId('overview-grid').screenshot({
          path: info.outputPath(`${shape}-overview-${width}-${scheme}.png`),
        });
        expect(tiles.map((tile) => tile.label)).toEqual([
          'Leverage',
          'Concurrency',
          'Merged PRs',
          'Hands-off median',
        ]);
        // Every number, change, secondary value and reason stays.
        expect(tiles.map((tile) => [tile.value, tile.sub])).toEqual(texts[shape]);
        for (const tile of tiles) {
          expect(tile.labelFits, tile.label!).toBe(true);
          expect(tile.valueFits, tile.label!).toBe(true);
          expect(tile.inside, tile.label!).toBe(true);
          expect(tile.valueBelowLabel, tile.label!).toBe(true);
        }
        // Two rows of two, each pair the same height.
        expect(tiles[0]!.top).toBeCloseTo(tiles[1]!.top, 1);
        expect(tiles[2]!.top).toBeCloseTo(tiles[3]!.top, 1);
        expect(tiles[0]!.height).toBeCloseTo(tiles[2]!.height, 0);
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        expect(
          await page.evaluate(() => {
            const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
            return [
              document.documentElement.scrollHeight <= window.innerHeight,
              outlet.scrollHeight <= outlet.clientHeight + 1,
            ];
          }),
        ).toEqual([true, true]);
      });

test('the Concurrency definition says the shown value is the mean, from the keyboard', async ({
  page,
}) => {
  await serve(page, 'reference');
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto('/dashboard');
  const tile = page.getByTestId('overview-tile').filter({ hasText: 'Concurrency' });
  await expect(tile.getByTestId('overview-value')).toContainText('2.6');
  await expect(tile).not.toContainText('mean');
  // Tab reaches the definition from the tile before it, and focus alone opens
  // it, which the control names as its description.
  const info = page.getByRole('button', { name: 'Concurrency definition' });
  await page.getByRole('button', { name: 'Leverage definition' }).focus();
  await page.keyboard.press('Tab');
  await expect(info).toBeFocused();
  // The control before it may still be closing its own definition.
  const tip = page.getByRole('tooltip', { name: /^How many agent sessions ran at the same time/ });
  await expect(tip).toBeVisible();
  await expect(tip).toContainText(CONCURRENCY_DEFINITION);
  await expect(info).toHaveAccessibleDescription(/average over the time any ran/);
  const box = (await tip.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(1120);
  expect(box.y + box.height).toBeLessThanOrEqual(720);
  await page.keyboard.press('Escape');
  await expect(tip).toBeHidden();
  await expect(info).toBeFocused();
  // Hover shows the same definition.
  await page.mouse.move(0, 0);
  await info.hover();
  await expect(tip).toBeVisible();
  await expect(tip).toContainText('Max is the most at once');
});
