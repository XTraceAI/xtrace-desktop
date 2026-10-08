import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

// Test-only measurement. Keep the Rust-exported summary and row answer in sync.
const ACTIVE_MS = 916_020_000; // 254 hours, 27 minutes.
for (const [width, height] of [
  [1120, 720],
  [1250, 768],
  [1440, 900],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`254 h 27 m stays exact and fits the summary at ${width} ${scheme}`, async ({
      page,
    }, info) => {
      const served = structuredClone(fixture as FixtureExport);
      for (const entry of served.sessions_summaries ?? [])
        if (entry.session_ids.length) entry.summary.agent_ms = ACTIVE_MS;
      for (const sessionPage of served.sessions) {
        if (sessionPage.summary) sessionPage.summary.agent_ms = ACTIVE_MS;
        for (const row of sessionPage.rows)
          if (row.metrics.state === 'indexed') row.metrics.agent_ms = ACTIVE_MS;
      }
      await page.route('**/fixtures/F1.json*', (route) =>
        route.fulfill({
          contentType: 'text/javascript',
          body: `export default ${JSON.stringify(served)};`,
        }),
      );
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/sessions');
      const tile = page
        .getByRole('region', { name: 'Range summary' })
        .locator('.xt-stat-tile')
        .filter({ hasText: 'Agent working time' });
      await expect(tile.locator('.xt-metric-cell')).toHaveText('254 h 27 m');
      await expect(tile.locator('.xt-stat-aside')).toHaveText('Parallel sessions add together');
      await expect(page.locator('.xt-session-agent .xt-metric-cell')).toHaveText('254 h 27 m');
      await expect(page.locator('.xt-session-agent')).toHaveAttribute(
        'title',
        'Exactly 916,020,000 ms active in this range',
      );
      await page.evaluate(() => document.fonts.ready);
      const fit = await tile.evaluate((element) => {
        const outer = element.getBoundingClientRect();
        return [
          ...element.querySelectorAll<HTMLElement>(
            '.xt-stat-label, .xt-stat-row, .xt-stat-value, .xt-metric-cell, .xt-stat-aside',
          ),
        ].map((part) => ({
          text: part.textContent,
          fits: part.scrollWidth <= part.clientWidth + 0.5,
          inside:
            part.getBoundingClientRect().left >= outer.left - 0.5 &&
            part.getBoundingClientRect().right <= outer.right + 0.5,
        }));
      });
      await info.attach('large-duration-layout', { body: JSON.stringify(fit, null, 2) });
      for (const part of fit) {
        expect(part.fits, part.text ?? '').toBe(true);
        expect(part.inside, part.text ?? '').toBe(true);
      }
      await page.locator('.xt-sessions').screenshot({
        path: info.outputPath(`sessions-large-duration-${width}-${scheme}.png`),
      });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await tile.focus();
      await expect(page.getByRole('tooltip')).toContainText(
        'Parallel sessions add together; this is not time saved.',
      );
      await page.keyboard.press('Escape');
    });
