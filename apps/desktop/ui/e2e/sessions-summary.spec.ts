import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The four range measurements above the session list. They describe the whole
 * selected range, so they must stay readable beside a list that filters and
 * pages independently of them: at both supported widths, in both schemes, no
 * label is clipped, no value or aside leaves its tile, and the list itself is
 * still on the page underneath.
 */
const LABELS = ['Human messages', 'Output tokens', 'Agent time', 'Sessions / day'];
const SCOPE =
  'The summary counts all indexed activity in the selected range; filters narrow only the table.';
const SCOPE_LINE = 'Range: all indexed activity · filters: table only';

for (const [width, height] of [
  [1440, 900],
  [1250, 768],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`Sessions summary tiles stay readable at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/sessions');
      const summary = page.getByRole('region', { name: 'Range summary' });
      await expect(summary.locator('.xt-stat-tile')).toHaveCount(4);
      // What the tiles cover reads in view as the heading's one short line;
      // the whole sentence is the summary's description.
      const line = page.locator('.xt-sessions-heading p');
      await expect(line).toHaveText(SCOPE_LINE);
      await expect(summary).toHaveAccessibleDescription(SCOPE);
      await page.evaluate(() => document.fonts.ready);
      const subtitle = await line.evaluate((element) => ({
        height: element.getBoundingClientRect().height,
        lineHeight: parseFloat(getComputedStyle(element).lineHeight),
        clipped: element.scrollWidth > element.clientWidth,
      }));
      const overflow = await page.evaluate(() => {
        const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
        return {
          outlet: outlet.scrollHeight - outlet.clientHeight,
          page: document.documentElement.scrollHeight - window.innerHeight,
        };
      });
      await info.attach('subtitle', { body: JSON.stringify({ subtitle, overflow }, null, 2) });
      // One whole line, neither wrapped nor cut short, and the page itself does not scroll.
      expect(subtitle.height).toBeLessThanOrEqual(subtitle.lineHeight + 0.5);
      expect(subtitle.clipped).toBe(false);
      expect(overflow.outlet).toBeLessThanOrEqual(0);
      expect(overflow.page).toBeLessThanOrEqual(0);
      // The filters and the warning slot stay usable beside or under it.
      await expect(page.getByRole('searchbox', { name: 'Search sessions' })).toBeVisible();
      await expect(page.getByRole('switch', { name: 'With PRs only' })).toBeVisible();
      const tiles = await page.evaluate(() =>
        [...document.querySelectorAll<HTMLElement>('.xt-sessions-tiles .xt-stat-tile')].map(
          (tile) => {
            const fits = (selector: string) => {
              const element = tile.querySelector<HTMLElement>(selector);
              return element ? element.scrollWidth <= element.clientWidth : true;
            };
            const box = tile.getBoundingClientRect();
            return {
              label: tile.querySelector<HTMLElement>('.xt-stat-label')!.textContent,
              text: tile.textContent,
              labelFits: fits('.xt-stat-label'),
              asideFits: fits('.xt-stat-aside'),
              valueFits: fits('.xt-stat-value .xt-metric-cell'),
              inside: [...tile.querySelectorAll<HTMLElement>('.xt-stat-label, .xt-stat-row')]
                .map((element) => element.getBoundingClientRect())
                .every((rect) => rect.left >= box.left && rect.right <= box.right + 0.5),
            };
          },
        ),
      );
      await info.attach('tiles', { body: JSON.stringify(tiles, null, 2) });
      await page
        .locator('.xt-sessions-tiles')
        .screenshot({ path: info.outputPath(`sessions-tiles-${width}-${scheme}.png`) });
      expect(tiles.map((tile) => tile.label)).toEqual(LABELS);
      // F1's range: five human messages, 150 independently measured output
      // tokens, a 23-minute active span and one session in one day bucket.
      expect(tiles[0].text).toContain('5');
      expect(tiles[1].text).toContain('150');
      expect(tiles[2].text).toContain('23');
      expect(tiles[3].text).toContain('max 1');
      for (const tile of tiles) {
        expect(tile.labelFits, tile.label ?? '').toBe(true);
        expect(tile.asideFits, tile.label ?? '').toBe(true);
        expect(tile.valueFits, tile.label ?? '').toBe(true);
        expect(tile.inside, tile.label ?? '').toBe(true);
      }
      // The summary sits above the list; it never stands in for it.
      await expect(page.getByRole('table', { name: 'Indexed sessions' })).toBeVisible();
      await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    });

/**
 * The values the one-decimal scale cannot show, in a real engine: a range that
 * did a little must not render as "0", and the longer `<0.1` must still fit.
 * Served in place of the development F1 export; these are not real history.
 */
test('a positive below the shown scale reads as such and still fits', async ({ page }, info) => {
  const small = structuredClone(fixture as FixtureExport);
  for (const report of small.dashboards) {
    // One session across the range's day buckets, and two seconds of span.
    report.tiles.sessions_per_day.value = 1 / 30;
    report.tiles.agent_hours.value = 2 / 3600;
    report.tokens.counters.output_tokens = null;
  }
  // The row under the tiles measures the same two seconds, on the same scale.
  for (const page of small.sessions)
    for (const row of page.rows) if (row.metrics.state === 'indexed') row.metrics.agent_ms = 2000;
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(small)};`,
    }),
  );
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto('/sessions');
  const summary = page.getByRole('region', { name: 'Range summary' });
  await expect(summary.locator('.xt-stat-tile')).toHaveCount(4);
  await page.evaluate(() => document.fonts.ready);
  const tiles = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>('.xt-sessions-tiles .xt-stat-tile')].map((tile) => {
      const value = tile.querySelector<HTMLElement>('.xt-stat-value .xt-metric-cell')!;
      return {
        label: tile.querySelector<HTMLElement>('.xt-stat-label')!.textContent,
        value: value.textContent,
        valueFits: value.scrollWidth <= value.clientWidth,
      };
    }),
  );
  await info.attach('tiles', { body: JSON.stringify(tiles, null, 2) });
  await page
    .locator('.xt-sessions-tiles')
    .screenshot({ path: info.outputPath('sessions-tiles-small.png') });
  // Agent time is written as all agent time is; sessions per day on the one-decimal scale.
  expect(tiles[2].value).toBe('<0.1m');
  expect(tiles[3].value).toBe('<0.1');
  // Absent counters are not an absence of output.
  expect(tiles[1].value).toContain('Output token counts are missing or incomplete');
  for (const tile of tiles) expect(tile.valueFits, tile.label ?? '').toBe(true);
  // A row reads the same way, and the longer value fits its narrow column.
  const cell = page
    .getByRole('row')
    .filter({ hasText: 'Session 00000000' })
    .getByText('<0.1m', { exact: true });
  await expect(cell).toBeVisible();
  expect(await cell.evaluate((element) => element.scrollWidth <= element.clientWidth + 0.5)).toBe(
    true,
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1120);
});
