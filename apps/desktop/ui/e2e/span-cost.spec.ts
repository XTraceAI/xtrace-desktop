import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

for (const [name, unknown, longTool] of [
  ['normal', false, false],
  ['long tool', false, true],
  ['unknown usage', true, true],
] as const) {
  test(`span tooltip fits ${name} beside tokens`, async ({ page }, testInfo) => {
    const out = structuredClone(fixture as FixtureExport);
    for (const span of out.span_details) {
      if (span.detail.state !== 'indexed') continue;
      span.detail.tool = {
        state: 'called',
        name: longTool ? 'a_long_tool_name_that_needs_to_shrink' : 'exec',
        calls: 1,
      };
      span.detail.output_tokens = unknown ? null : 440_000;
      span.detail.cost = {
        total_usd: unknown ? null : 1.25,
        priced_subtotal_usd: unknown ? 0 : 1.25,
        selected_observations: 2,
        priced_observations: unknown ? 0 : 2,
        unpriced_observations: unknown ? 2 : 0,
        assumed_tier_observations: 0,
        unpriced: [],
      };
    }
    await page.route('**/fixtures/F1.json?import', (route) =>
      route.fulfill({
        contentType: 'text/javascript',
        body: `export default ${JSON.stringify(out)};`,
      }),
    );
    await page.setViewportSize({ width: 1000, height: 750 });
    await page.goto('/');
    await page.evaluate(() => document.fonts.ready);
    await page
      .getByRole('img', { name: /^Active span / })
      .first()
      .hover();
    const bubble = page.getByRole('tooltip');
    await expect(bubble.locator('.xt-span-bubble-cost')).toHaveText(
      unknown ? 'cost unknown' : '$1.25',
    );
    await expect(bubble.locator('.xt-span-bubble-cost')).toHaveAttribute(
      'title',
      /recorded response usage in this span at public API prices/,
    );
    const fit = await bubble.evaluate((element) => {
      const box = element.getBoundingClientRect();
      return {
        width: box.width,
        scrolls: element.scrollWidth > element.clientWidth + 1,
        fieldsFit: [...element.querySelectorAll('.xt-span-bubble-meta > *')].every((field) => {
          const rect = field.getBoundingClientRect();
          return rect.left >= box.left && rect.right <= box.right;
        }),
      };
    });
    expect(fit.width).toBeLessThanOrEqual(360);
    expect(fit.scrolls).toBe(false);
    expect(fit.fieldsFit).toBe(true);
    await bubble.screenshot({
      path: testInfo.outputPath(`span-${name.replaceAll(' ', '-')}.png`),
    });
  });
}
