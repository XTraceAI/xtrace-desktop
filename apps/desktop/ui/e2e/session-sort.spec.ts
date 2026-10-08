import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['dark', 'light'] as const)
    test(`Recently active sorting and Back fit ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      const data = structuredClone(fixture as FixtureExport);
      for (const sessionPage of data.sessions)
        sessionPage.rows = Array.from({ length: 60 }, (_, i) => ({
          ...sessionPage.rows[0],
          id: `synthetic-${String(i).padStart(3, '0')}`,
          title: `Conversation ${i}`,
          last_activity_at_ms: i,
        }));
      await page.route('**/fixtures/F1.json?import', (route) =>
        route.fulfill({
          contentType: 'text/javascript',
          body: `export default ${JSON.stringify(data)};`,
        }),
      );
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/sessions?range=14d&host=claude');
      await expect(page.getByRole('region', { name: 'Recent indexed activity' })).toHaveCount(0);
      const sort = page.getByRole('combobox', { name: 'Sort sessions' });
      await expect(sort).toHaveValue('started');
      await sort.focus();
      await expect(sort).toBeFocused();
      await sort.selectOption('recently_active');
      await expect(page).toHaveURL(/sort=recently_active/);
      const links = page
        .getByRole('table', { name: 'Indexed sessions' })
        .getByRole('link', { name: /^Open session/ });
      await expect(links.first()).toHaveText('Conversation 59');
      await expect(links.first()).toHaveAttribute('href', /sort=recently_active/);
      await links.first().click();
      await expect(page.getByRole('link', { name: '← All sessions' })).toHaveAttribute(
        'href',
        /sort=recently_active/,
      );
      await page.goBack();
      await expect(sort).toHaveValue('recently_active');
      await page.goForward();
      await page.getByRole('link', { name: '← All sessions' }).click();
      await expect(sort).toHaveValue('recently_active');
      await expect(page.getByRole('searchbox', { name: 'Search sessions' })).toHaveValue('');
      await expect(page.getByRole('radio', { name: '14d' })).toHaveAttribute(
        'aria-checked',
        'true',
      );
      const layout = await page.evaluate(() => {
        const scroll = document.querySelector<HTMLElement>('.xt-table-scroll')!;
        const select = document
          .querySelector('select[aria-label="Sort sessions"]')!
          .getBoundingClientRect();
        return {
          pageOverflow: document.documentElement.scrollWidth > innerWidth,
          verticalOverflow: document.documentElement.scrollHeight > innerHeight,
          rowsScroll: scroll.scrollHeight > scroll.clientHeight,
          selectFits:
            select.left >= 0 &&
            select.right <= innerWidth &&
            select.top >= 0 &&
            select.bottom <= innerHeight,
        };
      });
      expect(layout).toEqual({
        pageOverflow: false,
        verticalOverflow: false,
        rowsScroll: true,
        selectFits: true,
      });
      await page.screenshot({ path: info.outputPath(`sessions-sort-${scheme}-${width}.png`) });
      await sort.selectOption('started');
      await expect(links.first()).toHaveText('Conversation 0');
      await expect(page).not.toHaveURL(/sort=/);
    });
