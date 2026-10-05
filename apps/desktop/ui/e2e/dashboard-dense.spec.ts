import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardMetrics } from '../src/data/generated/DashboardMetrics';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { MetricTile } from '../src/data/generated/MetricTile';

/**
 * Synthetic tile values (not real history), served in place of the development F1 export:
 * `dense` has multi-digit means with percentage changes and asides; `reference` has the
 * shape of the user's 2026-09-23 screenshot, where short values sat beside their labels and
 * dense ones wrapped under theirs (a three-digit change, an unknown merged count with its
 * known and unknown subtotal); `unknown` has every tile unmeasured with its reason. Labels
 * must stay at full width, and every tile draws its label and its value on the same two rows.
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
  dense: ({ tiles }) => {
    tiles.agent_hours_per_day = measured(tiles.agent_hours_per_day, 12.4, 8.5);
    tiles.concurrency_mean = measured(tiles.concurrency_mean, 1.8, -12.3);
    tiles.concurrency_max = measured(tiles.concurrency_max, 8, null);
    tiles.merged_prs = measured(tiles.merged_prs, 23, 15.2);
    tiles.hands_off_median = measured(tiles.hands_off_median, 3.2, -4.1);
    tiles.hands_off_p90 = measured(tiles.hands_off_p90, 14.8, null);
  },
  reference: (report) => {
    const { tiles } = report;
    tiles.agent_hours_per_day = measured(tiles.agent_hours_per_day, 30, 166.2);
    tiles.concurrency_mean = measured(tiles.concurrency_mean, 2.6, 112.4);
    tiles.concurrency_max = measured(tiles.concurrency_max, 8, null);
    tiles.merged_prs = unknown(tiles.merged_prs, 'Merged pull requests with unknown facts');
    report.pr_effort.current.tile = {
      ...report.pr_effort.current.tile,
      complete: false,
      known_merged: 0,
      unknown_facts: 32,
    };
    tiles.hands_off_median = measured(tiles.hands_off_median, 5.6, 298.6);
    tiles.hands_off_p90 = measured(tiles.hands_off_p90, 32.1, null);
  },
  unknown: ({ tiles }) => {
    tiles.agent_hours_per_day = unknown(tiles.agent_hours_per_day, 'No active spans');
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

const texts: Record<Shape, [string, string, string, string]> = {
  dense: [
    'Agent h/day12.4h▲8.5%',
    'Concurrency1.8▼12.3%max 8',
    // F1's own merged-PR tile is incomplete, so its known + unknown subtotal
    // stays beside the value.
    'Merged PRs23▲15.2%0 + 2 unknown',
    'Hands-off median3.2min▼4.1%p90 14.8',
  ],
  reference: [
    'Agent h/day30h▲166.2%',
    'Concurrency2.6▲112.4%max 8',
    'Merged PRs—Unmeasured: Merged pull requests with unknown facts0 + 32 unknown',
    'Hands-off median5.6min▲298.6%p90 32.1',
  ],
  unknown: [
    'Agent h/day—Unmeasured: No active spansh',
    'Concurrency—Unmeasured: No overlapping spans',
    'Merged PRs—Unmeasured: No cached pull request facts0 + 2 unknown',
    'Hands-off median—Unmeasured: No hands-off stretchesmin',
  ],
};

for (const shape of ['dense', 'reference', 'unknown'] as const)
  for (const [width, height] of [
    [1440, 900],
    [1120, 720],
  ] as const)
    for (const scheme of ['light', 'dark'] as const)
      test(`${shape} tile values keep full labels on one shared value row at ${width}x${height} ${scheme}`, async ({
        page,
      }, info) => {
        await serve(page, shape);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto('/dashboard');
        await expect(page.locator('.xt-stat-tile')).toHaveCount(4);
        await page.evaluate(() => document.fonts.ready);
        const tiles = await page.evaluate(() =>
          [...document.querySelectorAll<HTMLElement>('.xt-dash-tiles .xt-stat-tile')].map(
            (tile) => {
              const fits = (selector: string) => {
                const element = tile.querySelector<HTMLElement>(selector);
                return element ? element.scrollWidth <= element.clientWidth : true;
              };
              const label = tile.querySelector<HTMLElement>('.xt-stat-label')!;
              const row = tile.querySelector<HTMLElement>('.xt-stat-row')!;
              const cell = tile.querySelector<HTMLElement>('.xt-stat-value .xt-metric-cell')!;
              const box = tile.getBoundingClientRect();
              const inside = [
                ...tile.querySelectorAll<HTMLElement>(
                  '.xt-stat-label, .xt-stat-row, .xt-stat-row *',
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
              // The value row's baseline: an empty item has none of its own, so
              // the row aligns its bottom edge to the row's shared baseline.
              // The value's own text sits on the same line: an empty
              // inline-block after its text sits on the text's baseline.
              const baselineIn = (parent: HTMLElement) => {
                const probe = document.createElement('span');
                probe.style.display = 'inline-block';
                parent.append(probe);
                const bottom = probe.getBoundingClientRect().bottom - box.top;
                probe.remove();
                return bottom;
              };
              const baseline = baselineIn(row);
              const textBaseline = baselineIn(cell);
              return {
                text: tile.textContent,
                label: label.textContent,
                labelTop: label.getBoundingClientRect().top - box.top,
                clientWidth: label.clientWidth,
                scrollWidth: label.scrollWidth,
                labelFits: label.scrollWidth <= label.clientWidth,
                asideFits: fits('.xt-stat-aside'),
                valueFits: fits('.xt-stat-value .xt-metric-cell'),
                rowFits: row.scrollWidth <= row.clientWidth,
                valueBelowLabel:
                  row.getBoundingClientRect().top >= label.getBoundingClientRect().bottom,
                rowRight: box.right - row.getBoundingClientRect().right,
                baseline,
                textBaseline,
                height: box.height,
                inside,
              };
            },
          ),
        );
        await info.attach('tiles', { body: JSON.stringify(tiles, null, 2) });
        await page.locator('.xt-dash-tiles').screenshot({
          path: info.outputPath(`${shape}-tiles-${width}-${scheme}.png`),
        });
        expect(tiles.map((tile) => tile.label)).toEqual([
          'Agent h/day',
          'Concurrency',
          'Merged PRs',
          'Hands-off median',
        ]);
        // Every number, change, aside and reason stays; only Concurrency's
        // visible "mean" is gone.
        expect(tiles.map((tile) => tile.text)).toEqual(texts[shape]);
        for (const tile of tiles) {
          expect(tile.labelFits, `${tile.label}: ${tile.scrollWidth}/${tile.clientWidth}`).toBe(
            true,
          );
          expect(tile.asideFits, tile.label).toBe(true);
          expect(tile.valueFits, tile.label).toBe(true);
          expect(tile.rowFits, tile.label).toBe(true);
          expect(tile.inside, tile.label).toBe(true);
          expect(tile.valueBelowLabel, tile.label).toBe(true);
        }
        // One label row and one value row, shared by all four: the same
        // height, the same label top, the same value baseline, every value
        // ending at the same inset from its tile's right edge.
        const [first] = tiles;
        for (const tile of tiles) {
          expect(tile.height, tile.label).toBeCloseTo(first.height, 1);
          expect(tile.labelTop, tile.label).toBeCloseTo(first.labelTop, 1);
          expect(tile.baseline, tile.label).toBeCloseTo(first.baseline, 1);
          expect(tile.textBaseline, tile.label).toBeCloseTo(tile.baseline, 1);
          expect(tile.rowRight, tile.label).toBeCloseTo(first.rowRight, 1);
        }
        expect(first.height).toBeLessThanOrEqual(64);
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
  const tile = page.getByRole('button', { name: /^Concurrency/ });
  await expect(tile).toContainText('2.6');
  await expect(tile).not.toContainText('mean');
  // Tab reaches the tile from the tile before it, and focus alone opens the
  // definition, which the tile names as its description.
  await page.getByRole('button', { name: /^Agent h\/day/ }).focus();
  await page.keyboard.press('Tab');
  await expect(tile).toBeFocused();
  // The tile before it may still be closing its own definition.
  const tip = page.getByRole('tooltip', { name: /^M-06 · Concurrency\./ });
  await expect(tip).toBeVisible();
  await expect(tip).toContainText('M-06 · Concurrency.');
  await expect(tip).toContainText('Displayed value is mean concurrency; max is the peak overlap.');
  await expect(tile).toHaveAccessibleDescription(/Displayed value is mean concurrency/);
  const box = (await tip.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(1120);
  expect(box.y + box.height).toBeLessThanOrEqual(720);
  await page.keyboard.press('Escape');
  await expect(tip).toBeHidden();
  await expect(tile).toBeFocused();
  // Hover shows the same definition.
  await page.mouse.move(0, 0);
  await tile.hover();
  await expect(tip).toBeVisible();
  await expect(tip).toContainText('Displayed value is mean concurrency');
});
