import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * Browser evidence for the popover's layout only: this is the fixture preview
 * in a browser page, not the native menu-bar window, its click, position or
 * scale. F1 is pinned at exact UTC midnight, so its own today is the empty day;
 * the measured variant replaces only `today` with synthetic figures (not real
 * history) that exercise the widest strings.
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
    state: 'partial',
    total_usd: null,
    priced_subtotal_usd: 1234.56,
    selected_observations: 412,
    priced_observations: 398,
  },
  agent: { active_ms: 5.7 * 3_600_000, sessions: 7 },
};

const serve = (page: Page, body: FixtureExport) =>
  page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );

for (const [name, body] of [
  ['empty midnight', fixture as FixtureExport],
  ['measured', measured],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`tray popover lays out the ${name} day in ${scheme}`, async ({ page }, info) => {
      await serve(page, body);
      await page.setViewportSize({ width: 360, height: 540 });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/tray');
      const panel = page.getByRole('main', { name: 'XTrace today' });
      await expect(panel.getByRole('region', { name: 'Today’s output tokens' })).toBeVisible();
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
      if (name === 'measured') await expect(panel.getByText(/not a total/)).toBeVisible();
      await expect(panel.getByRole('region', { name: 'Active now' })).toContainText(
        'cannot show what is running now',
      );
      await page.screenshot({
        path: info.outputPath(`tray-${name.replace(' ', '-')}-${scheme}.png`),
      });
    });
