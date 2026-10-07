import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import { manyPrRows, PR_CACHE_STATES } from '../src/app/prs.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { PrRow } from '../src/data/generated/PrRow';

/**
 * The cached pull-request inventory in both engines and themes, over the
 * generated F1 export and over test-only synthetic stored rows (not real
 * history, not a design sample) served in place of F1's list. Nothing here
 * reaches GitHub: the one refresh below is the browser fixture's own.
 */
const withRows = (rows: PrRow[]) => {
  // JSON imports widen literal unions; the export is the generated shape.
  const copy = structuredClone(fixture as FixtureExport);
  copy.pull_requests = { rows };
  return copy;
};

async function open(
  page: Page,
  {
    width,
    height,
    scheme = 'light',
    rows,
  }: { width: number; height: number; scheme?: 'light' | 'dark'; rows?: PrRow[] },
) {
  if (rows)
    await page.route('**/fixtures/F1.json?import', (route) =>
      route.fulfill({
        contentType: 'text/javascript',
        body: `export default ${JSON.stringify(withRows(rows))};`,
      }),
    );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/prs?view=inventory');
  await expect(page.getByRole('table', { name: 'Linked pull requests' })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const region = (page: Page) =>
  page.getByRole('region', { name: 'Linked pull requests scroll area' });

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`every cache state fits its row at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      const errors: string[] = [];
      const external: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      page.on('console', (message) => {
        if (message.type() === 'error') errors.push(message.text());
      });
      page.on('request', (request) => {
        const { hostname } = new URL(request.url());
        if (!['127.0.0.1', 'localhost'].includes(hostname)) external.push(request.url());
      });
      await open(page, { width, height, scheme, rows: PR_CACHE_STATES });
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('Pull requests');
      // The Shell offers no range here, so nothing appears to filter the list.
      await expect(page.getByRole('radiogroup', { name: 'Date range' })).toHaveCount(0);
      await expect(page.getByTestId('report-period')).toHaveCount(0);
      // No figure stands in for the missing effort report, and nothing refreshes.
      await expect(page.locator('main .xt-stat-tile')).toHaveCount(0);
      await expect(page.locator('main').getByRole('button', { name: /refresh/i })).toHaveCount(0);
      await expect(page.locator('main a[href^="http"]')).toHaveCount(0);
      const headers = await page.getByRole('columnheader').allTextContents();
      expect(headers.map((header) => header.trim())).toEqual([
        'pull request · repo · branch',
        'state',
        'size',
        'sessions',
        'cache status',
      ]);

      const layout = await page.evaluate(() => {
        const box = (element: Element) => element.getBoundingClientRect();
        const clipped = (element: HTMLElement) => element.scrollWidth > element.clientWidth + 0.5;
        const scroll = document.querySelector<HTMLElement>('.xt-prs .xt-table-scroll')!;
        const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
        const card = document.querySelector<HTMLElement>('.xt-prs > .xt-section-card')!;
        const note = document.querySelector<HTMLElement>('[data-testid="prs-scope"]')!;
        const rows = [...document.querySelectorAll<HTMLElement>('.xt-prs .xt-data-row')];
        const probe = (token: string) => {
          const swatch = document.createElement('span');
          swatch.style.color = `var(${token})`;
          document.body.append(swatch);
          const color = getComputedStyle(swatch).color;
          swatch.remove();
          return color;
        };
        return {
          pageWidth: document.documentElement.scrollWidth,
          outletScrolls: outlet.scrollHeight > outlet.clientHeight + 1,
          tableScrollsSideways: scroll.scrollWidth > scroll.clientWidth + 1,
          cardBottom: box(card).bottom,
          viewport: innerHeight,
          noteLines: Math.round(box(note).height / parseFloat(getComputedStyle(note).lineHeight)),
          rowHeights: rows.map((row) => box(row).height),
          // Whatever must be read whole is whole; only free text may ellipsize,
          // and then its full text is one hover away.
          clippedFacts: [
            ...document.querySelectorAll<HTMLElement>(
              '.xt-prs .xt-pr-state-word, .xt-prs .xt-pr-cache-word, .xt-prs .xt-pr-size, .xt-prs .xt-pr-size > span, .xt-prs .xt-metric-cell, .xt-prs .xt-pr-state .xt-pr-meta',
            ),
          ]
            .filter(clipped)
            .map((element) => element.textContent),
          // Exact counts, each line whole and inside its column, with the
          // whole value on the cell whatever a line could ever clip.
          sizes: [...document.querySelectorAll<HTMLElement>('.xt-prs .xt-pr-size')].map((size) => {
            const cell = size.closest<HTMLElement>('[role="cell"]')!;
            const lines = [...size.querySelectorAll<HTMLElement>(':scope > span')];
            return {
              lines: lines.map((line) => line.textContent),
              whole: lines.every((line) => !clipped(line) && !clipped(size)),
              inside: lines.every(
                (line) =>
                  box(line).left >= box(cell).left - 0.5 &&
                  box(line).right <= box(cell).right + 0.5,
              ),
              stacked: lines.length < 2 || box(lines[0]!).bottom <= box(lines[1]!).top + 0.5,
              title: size.title,
            };
          }),
          clippedWithoutTitle: [
            ...document.querySelectorAll<HTMLElement>(
              '.xt-prs .xt-pr-title, .xt-prs .xt-pr-name .xt-pr-meta, .xt-prs .xt-pr-cache .xt-pr-meta',
            ),
          ]
            .filter(clipped)
            .filter((element) => !(element.title || element.parentElement?.title))
            .map((element) => element.textContent),
          outsideRow: rows.flatMap((row) =>
            [...row.querySelectorAll<HTMLElement>('[role="cell"]')]
              .filter(
                (cell) =>
                  box(cell).left < box(row).left - 0.5 ||
                  box(cell).right > box(row).right + 0.5 ||
                  box(cell).top < box(row).top - 0.5 ||
                  box(cell).bottom > box(row).bottom + 0.5,
              )
              .map((cell) => cell.textContent),
          ),
          tones: [...document.querySelectorAll<HTMLElement>('.xt-prs .xt-pr-cache-word')].map(
            (word) => [word.dataset.tone, getComputedStyle(word).color],
          ),
          tokens: { meta: probe('--meta'), info: probe('--info'), warning: probe('--warning') },
        };
      });
      await info.attach('layout', { body: JSON.stringify(layout, null, 2) });
      await page.screenshot({ path: info.outputPath(`prs-states-${width}-${scheme}.png`) });
      expect(layout.pageWidth).toBe(width);
      expect(layout.outletScrolls).toBe(false);
      expect(layout.tableScrollsSideways).toBe(false);
      expect(layout.cardBottom).toBeLessThanOrEqual(layout.viewport);
      expect(layout.noteLines).toBe(1);
      expect(layout.rowHeights).toEqual(PR_CACHE_STATES.map(() => 40));
      expect(layout.clippedFacts).toEqual([]);
      expect(layout.clippedWithoutTitle).toEqual([]);
      expect(layout.sizes.map((size) => size.lines)).toEqual([
        ['+0 additions', '−0 deletions'],
        ['+1,234,567 additions', '−987,654 deletions'],
        ['+5 additions', '−— deletions not cached'],
        ['+12,345 additions', '−67,890 deletions'],
      ]);
      expect(layout.sizes.map((size) => size.title)).toEqual([
        '+0 additions · −0 deletions',
        '+1,234,567 additions · −987,654 deletions',
        '+5 additions · deletions not cached',
        '+12,345 additions · −67,890 deletions',
      ]);
      for (const size of layout.sizes) {
        expect(size.whole, size.lines.join(' ')).toBe(true);
        expect(size.inside, size.lines.join(' ')).toBe(true);
        expect(size.stacked, size.lines.join(' ')).toBe(true);
      }
      expect(layout.outsideRow).toEqual([]);
      // Each status takes its tone from this theme's tokens; refreshed is never green.
      expect(layout.tones).toEqual(
        (['meta', 'info', 'warning', 'warning', 'info', 'info'] as const).map((tone) => [
          tone,
          layout.tokens[tone],
        ]),
      );

      // The explanation opens from the keyboard and stays inside the page.
      const summary = page.getByText('What is listed and counted', { exact: true });
      await summary.focus();
      await expect(summary).toBeFocused();
      expect(await summary.evaluate((el) => getComputedStyle(el).outlineStyle)).not.toBe('none');
      await page.keyboard.press('Enter');
      const counted = page.getByTestId('prs-counted');
      await expect(counted).toHaveAttribute('open', '');
      await expect(counted).toContainText('the selected range does not apply');
      const opened = await page.evaluate(() => {
        const details = document.querySelector<HTMLElement>('[data-testid="prs-counted"]')!;
        const frame = document.querySelector<HTMLElement>('.xt-prs')!.getBoundingClientRect();
        const rect = details.getBoundingClientRect();
        return {
          inside: rect.left >= frame.left - 0.5 && rect.right <= frame.right + 0.5,
          pageWidth: document.documentElement.scrollWidth,
          tableHeight: document.querySelector<HTMLElement>('.xt-prs .xt-table-scroll')!
            .clientHeight,
        };
      });
      expect(opened.inside).toBe(true);
      expect(opened.pageWidth).toBe(width);
      // The list keeps a usable height under the opened explanation.
      expect(opened.tableHeight).toBeGreaterThanOrEqual(150);
      await page.screenshot({ path: info.outputPath(`prs-explained-${width}-${scheme}.png`) });
      await page.keyboard.press('Enter');
      await expect(counted).not.toHaveAttribute('open', '');
      expect(errors).toEqual([]);
      expect(external).toEqual([]);
    });

for (const [width, height, scheme] of [
  [1120, 720, 'light'],
  [1440, 900, 'dark'],
] as const)
  test(`a long cached list scrolls in its own panel from the keyboard at ${width}x${height} ${scheme}`, async ({
    page,
    browserName,
  }, info) => {
    const total = 600;
    await open(page, { width, height, scheme, rows: manyPrRows(total) });
    const scroll = region(page);
    const rows = page.locator('.xt-prs .xt-data-row');
    await expect(rows).toHaveCount(total);
    await expect(page.locator('.xt-section-meta')).toContainText('600 pull requests indexed');
    const geometry = () =>
      page.evaluate(() => {
        const scroll = document.querySelector<HTMLElement>('.xt-prs .xt-table-scroll')!;
        const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
        const head = scroll.querySelector<HTMLElement>('.xt-table-head')!;
        const last = [...scroll.querySelectorAll<HTMLElement>('.xt-data-row')].at(-1)!;
        const frame = scroll.getBoundingClientRect();
        return {
          scrollTop: scroll.scrollTop,
          scrolls: scroll.scrollHeight > scroll.clientHeight,
          atEnd: Math.abs(scroll.scrollHeight - scroll.clientHeight - scroll.scrollTop) <= 1,
          outletTop: outlet.scrollTop,
          outletScrolls: outlet.scrollHeight > outlet.clientHeight + 1,
          pageTop: document.scrollingElement!.scrollTop,
          headPinned: Math.abs(head.getBoundingClientRect().top - frame.top) <= 1,
          lastVisible:
            last.getBoundingClientRect().bottom <= frame.bottom + 1 &&
            last.getBoundingClientRect().top >= frame.top,
        };
      });
    const start = await geometry();
    expect(start.scrolls).toBe(true);
    expect(start.outletScrolls).toBe(false);
    expect(start.headPinned).toBe(true);

    // The filter, then the explanation, then the rows' own region: three tab
    // stops, however long the list is.
    const tab = browserName === 'webkit' ? 'Alt+Tab' : 'Tab';
    await page.keyboard.press('Meta+k');
    await expect(page.getByRole('searchbox', { name: 'Filter pull requests' })).toBeFocused();
    await page.keyboard.press(tab);
    await expect(page.getByText('What is listed and counted', { exact: true })).toBeFocused();
    await page.keyboard.press(tab);
    await expect(scroll).toBeFocused();
    expect(await scroll.evaluate((el) => getComputedStyle(el).outlineStyle)).not.toBe('none');

    await page.keyboard.press('PageDown');
    await expect.poll(async () => (await geometry()).scrollTop).toBeGreaterThan(0);
    await page.keyboard.press('End');
    await expect.poll(async () => (await geometry()).atEnd).toBe(true);
    const end = await geometry();
    // One scrollbar moved the rows: the header stayed, the page did not move.
    expect(end.headPinned).toBe(true);
    expect(end.lastVisible).toBe(true);
    expect(end.outletTop).toBe(0);
    expect(end.pageTop).toBe(0);
    await expect(rows.last()).toContainText('xtrace/app#1599');
    await page.screenshot({ path: info.outputPath(`prs-long-end-${width}-${scheme}.png`) });
    await page.keyboard.press('Home');
    await expect.poll(async () => (await geometry()).scrollTop).toBe(0);

    // Narrowing is local and immediate, and the list is back when it is cleared.
    const filter = page.getByRole('searchbox', { name: 'Filter pull requests' });
    await filter.fill('milestone 5');
    await expect(rows).toHaveCount(3);
    await expect(page.locator('.xt-section-meta')).toContainText('3 of 600 shown');
    await filter.fill('no such pull request');
    await expect(page.getByText('No pull request matches this filter.')).toBeVisible();
    await filter.fill('');
    await expect(rows).toHaveCount(total);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
  });

test('shows what a refresh made on the Dashboard stored, and starts none itself', async ({
  page,
}) => {
  await open(page, { width: 1120, height: 720 });
  const table = page.getByRole('table', { name: 'Linked pull requests' });
  const rows = table.locator('.xt-data-row');
  await expect(rows).toHaveCount(3);
  // A fixture database starts with links only: nothing is cached, and nothing is guessed.
  for (const row of await rows.all()) {
    await expect(row).toContainText('Title not cached');
    await expect(row).toContainText('not checked yet');
    await expect(row.locator('.xt-pr-state-word')).toHaveCount(0);
  }
  // The manual refresh stays where it lives. The way there is in the
  // explanation, and it is the app's own route change, not a network link.
  await page.getByText('What is listed and counted', { exact: true }).click();
  await page.getByRole('link', { name: 'Dashboard' }).click();
  await expect(page).toHaveURL(/\/dashboard$/);
  // F1's links were never checked and this source never checks on its own,
  // so the Effort card's red ! is there to open the refresh.
  await page.getByRole('button', { name: 'Pull-request checks need your attention' }).click();
  const dialog = page.getByRole('dialog', { name: 'Refresh pull-request facts' });
  await dialog.getByRole('checkbox', { name: /#11/ }).check();
  await dialog.getByRole('checkbox', { name: /#12/ }).check();
  await dialog.getByRole('button', { name: 'Refresh 2 pull requests' }).click();
  await expect(dialog.getByTestId('pr-refresh-report')).toContainText(
    '1 checked, 1 could not be checked',
  );
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Pull requests', exact: true }).click();
  // The page opens on its report; the inventory is its other view.
  await page.getByRole('radio', { name: 'Cached inventory' }).click();
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText('Synthetic fixture pull request 11');
  await expect(rows.nth(0)).toContainText('merged');
  await expect(rows.nth(0)).toContainText('+33');
  await expect(rows.nth(0).locator('.xt-pr-cache-word')).toHaveText('checked');
  await expect(rows.nth(1)).toContainText('Title not cached');
  await expect(rows.nth(1).locator('.xt-pr-cache-word')).toHaveText('could not be checked');
  await expect(rows.nth(1)).toContainText('rate limited');
  await expect(rows.nth(2).locator('.xt-pr-cache-word')).toHaveText('not checked yet');
});
