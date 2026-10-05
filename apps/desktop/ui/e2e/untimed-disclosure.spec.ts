import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * Synthetic untimed history (not real history): F1 indexes nothing without a
 * timestamp, so the disclosure has nothing to say over the development export.
 * Only `untimed_history` is replaced — every measured number stays F1's, which
 * is the point: this notice explains what the dated numbers cannot hold and
 * changes none of them.
 */
const disclosed = structuredClone(fixture as FixtureExport);
for (const report of disclosed.dashboards)
  report.untimed_history = {
    records: 2318,
    by_surface: [
      { host: 'claude', surface: 'cli', records: 1204 },
      { host: 'claude', surface: 'desktop', records: 806 },
      { host: 'cursor', surface: null, records: 308 },
    ],
  };

const serve = (page: Page, body: FixtureExport) =>
  page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );

const NOTICE =
  'Some indexed history has no timestamps and cannot contribute to date-based measurements.';

test('neither page discloses untimed history when the report counts none', async ({ page }) => {
  await serve(page, fixture as FixtureExport);
  await page.goto('/dashboard');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await expect(page.getByRole('region', { name: 'Untimed indexed history' })).toHaveCount(0);
  // The Sessions header's Coverage names no untimed history either.
  const coverage = page
    .getByRole('region', { name: 'Sessions', exact: true })
    .getByRole('button', { name: /^Coverage/ });
  await expect(coverage).not.toHaveAccessibleName(/untimed/);
  await coverage.click();
  await expect(page.getByTestId('coverage-untimed')).toHaveCount(0);
  await page.keyboard.press('Escape');
  await page.goto('/sessions');
  await expect(page.getByRole('region', { name: 'Range summary' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Untimed indexed history' })).toHaveCount(0);
});

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    for (const path of ['/dashboard', '/sessions'] as const)
      test(`untimed disclosure reads and opens by keyboard on ${path} at ${width}x${height} ${scheme}`, async ({
        page,
      }, info) => {
        await serve(page, disclosed);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto(path);
        const dashboard = path === '/dashboard';
        // On the Dashboard the limit is part of Coverage, in the Sessions card's
        // header: the control reads one word and its name carries the count; on
        // Sessions it is the heading's warning triangle, whose name carries it.
        // Neither is a banner, and neither states a share of anything.
        const notice = dashboard
          ? page.getByRole('region', { name: 'Sessions', exact: true }).locator('header')
          : page.locator('.xt-sessions-heading');
        await expect(notice).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        const summary = dashboard
          ? notice.getByRole('button', { name: /^Coverage/ })
          : notice.getByTestId('sessions-issues');
        if (dashboard) {
          await expect(summary).toHaveText('Coverage');
          await expect(summary).toHaveAccessibleName(/^Coverage: (.+, )?2,318 untimed records$/);
        } else {
          await expect(summary).toHaveAccessibleName('History limits: 2,318 untimed records');
          await expect(page.getByRole('region', { name: 'Untimed indexed history' })).toHaveCount(
            0,
          );
        }
        await expect(notice).not.toContainText('%');
        const line = await summary.evaluate((element) => element.getBoundingClientRect().height);
        expect(line).toBeLessThanOrEqual(40);

        // The explanation and breakdown are closed by default and open from the
        // keyboard alone.
        const body = page.getByRole('dialog', { name: dashboard ? 'Coverage' : 'Untimed history' });
        const rows = body.getByRole('list', {
          name: 'Untimed indexed history by host and surface',
        });
        await expect(rows).toBeHidden();
        await expect(body.getByText(NOTICE)).toBeHidden();
        await summary.focus();
        await expect(summary).toBeFocused();
        await page.keyboard.press('Enter');
        await expect(body.getByText(NOTICE)).toBeVisible();
        if (dashboard)
          await expect(body).toContainText('2,318 records · outside dated measurements');
        const count = body.getByTestId('untimed-count');
        await expect(count).toHaveText(
          '2,318 records in all indexed history, independent of the selected dates and of any filter on the table.',
        );
        await expect(rows).toBeVisible();
        await expect(rows.getByRole('listitem')).toHaveText([
          'claude · cli1,204 records',
          'claude · desktop806 records',
          'cursor · unknown surface308 records',
        ]);

        // Readable where it is drawn: inside its own box, in the viewport, on
        // this theme's own foreground and surface colors, and without pushing
        // the page sideways at either native width.
        const measured = await body.evaluate((element) => {
          const box = element.getBoundingClientRect();
          const text = [...element.querySelectorAll<HTMLElement>('p, summary, li')];
          return {
            inside: text
              .map((node) => node.getBoundingClientRect())
              .every((rect) => rect.left >= box.left - 0.5 && rect.right <= box.right + 0.5),
            clipped: text.some((node) => node.scrollHeight > node.clientHeight + 1),
            within: box.left >= -0.5 && box.right <= window.innerWidth + 0.5,
            colors: text.map((node) => getComputedStyle(node).color),
            background: getComputedStyle(element).backgroundColor,
          };
        });
        await info.attach('untimed', { body: JSON.stringify(measured, null, 2) });
        await body.screenshot({
          path: info.outputPath(`untimed-${path.slice(1)}-${width}-${scheme}.png`),
        });
        expect(measured.inside).toBe(true);
        expect(measured.clipped).toBe(false);
        expect(measured.within).toBe(true);
        // Themed from tokens: no transparent text, and a surface of its own.
        for (const color of measured.colors) expect(color).not.toMatch(/rgba\(0, 0, 0, 0\)/);
        expect(measured.background).not.toMatch(/rgba\(0, 0, 0, 0\)/);
        expect(new Set(measured.colors).size).toBeGreaterThan(1);
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        await page.keyboard.press('Escape');
        await expect(body).toHaveCount(0);
        await expect(summary).toBeFocused();
      });
