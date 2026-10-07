import { expect, test, type Locator, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import { withPrEffort, type SectionSpec } from '../src/app/dashboard/pr-effort.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * Every limitation at once, over test-only synthetic data served in place of
 * the F1 export (not real history, not a design sample): an incomplete index,
 * untimed history, partial pricing, an unresolved session and an unknown
 * merged count. Each keeps a short visible state while its
 * explanation is one native disclosure away, reachable by keyboard, inside its
 * card and the viewport, in both schemes at both native widths.
 */
const COMBINED: SectionSpec = {
  tile: { known_merged: 1, unknown_facts: 1 },
  markers: [{ number: 1, day: 4, workType: 'feat' }],
  groups: [
    {
      assignment: { kind: 'type', work_type: 'feat' },
      sessions: 1,
      days: { 2: { agentMs: 1_800_000, usd: 1.25, selected: 2, priced: 1 } },
    },
    {
      assignment: { kind: 'unresolved' },
      sessions: 1,
      days: { 3: { agentMs: 1_800_000, usd: 0.8 } },
    },
  ],
};
const combined = structuredClone(fixture as FixtureExport);
combined.dashboards = combined.dashboards.map((report) => ({
  ...withPrEffort(report, COMBINED),
  untimed_history: {
    records: 2318,
    by_surface: [
      { host: 'claude', surface: 'cli', records: 2010 },
      { host: 'cursor', surface: null, records: 308 },
    ],
  },
}));
combined.native_index = {
  ...combined.native_index,
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  hosts: [
    {
      host: 'claude',
      state: 'incomplete',
      needs_attention: true,
      detail: null,
      sessions_imported: 1,
      sessions_partial: 1,
      sessions_skipped: 0,
      skipped_conversations: [],
      skipped_conversations_omitted: 0,
      records_new: 25,
      records_enriched: 0,
      diagnostics: 1,
    },
  ],
  needs_attention: true,
};

async function open(page: Page, path: string, width: number, height: number, scheme: string) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(combined)};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme as 'light' | 'dark' });
  await page.goto(path);
}
const card = (page: Page, name: string) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name, exact: true }),
  });
const height = (locator: Locator) =>
  locator.evaluate((element) => element.getBoundingClientRect().height);
/** Opens a native disclosure from the keyboard alone: focus its summary, then Enter. */
async function openByKeyboard(page: Page, summary: Locator) {
  await summary.focus();
  await expect(summary).toBeFocused();
  // A visible focus indicator on the summary itself.
  expect(await summary.evaluate((element) => getComputedStyle(element).outlineStyle)).not.toBe(
    'none',
  );
  await page.keyboard.press('Enter');
}
/** Every drawn part of `inner` sits inside `outer`, horizontally. */
async function inside(inner: Locator, outer: Locator) {
  const frame = await outer.evaluate((element) => {
    const box = element.getBoundingClientRect();
    return { left: box.left, right: box.right };
  });
  return inner.evaluate(
    (target, box) =>
      [target, ...target.querySelectorAll<HTMLElement>('*:not(.sr-only)')]
        .map((node) => node.getBoundingClientRect())
        .filter((rect) => rect.width > 1)
        .every((rect) => rect.left >= box.left - 0.5 && rect.right <= box.right + 0.5),
    frame,
  );
}
/** One text line of the 10.5px mono scale, with its padding and border. */
const ONE_LINE = 26;

for (const [width, viewportHeight] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const) {
    test(`dashboard limits stay short and open by keyboard at ${width}x${viewportHeight} ${scheme}`, async ({
      page,
    }, info) => {
      await open(page, '/dashboard', width, viewportHeight, scheme);
      const effort = card(page, 'Effort');
      const overview = card(page, 'Overview');
      await expect(effort.getByTestId('effort-by-type')).toBeVisible();
      await expect(overview.getByTestId('overview-tile')).toHaveCount(4);
      await page.evaluate(() => document.fonts.ready);

      await page
        .locator('.xt-dashboard')
        .screenshot({ path: info.outputPath(`compact-dashboard-closed-${width}-${scheme}.png`) });
      // Effort: the card draws no work type, so an unresolved session needs no
      // triangle in its header; the method is closed.
      // The card shows no line of session totals above the chart.
      await expect(effort.getByTestId('effort-cohort')).toHaveCount(0);
      await expect(effort.getByRole('button', { name: /^Unresolved type/ })).toHaveCount(0);
      const notes = page.getByTestId('effort-notes');
      await expect(notes).toHaveCount(0);
      const method = effort.getByRole('button', { name: 'Effort definition', exact: true });
      expect(await height(method)).toBeLessThanOrEqual(ONE_LINE);
      // Partial pricing is a priced subtotal marked +, on the total and on its
      // day, with the unpriced responses named.
      await effort.getByRole('radio', { name: 'cost' }).click();
      await expect(effort.getByTestId('effort-headline')).toHaveText(
        '$2.05+last 7 days1 response has no price',
      );
      const partial = effort.locator('[data-testid="effort-day"][data-partial]');
      await expect(partial).toHaveCount(1);
      await expect(partial).toHaveAccessibleName(
        /: \$1\.25\+\. synthetic-model \$1\.25 \(100%\)\. No price: 1 codex-auto-review response$/,
      );
      // From the measure control, Tab alone reaches the method control, which
      // opens the method and the daily values over the page.
      await page.keyboard.press('Tab');
      await openByKeyboard(page, method);
      const methodDialog = page.getByRole('dialog', {
        name: 'How effort is counted · daily values',
      });
      await expect(notes).toBeVisible();
      await expect(notes).toContainText('1 of 3 selected responses could not be priced');
      await expect(methodDialog.getByRole('table')).toBeAttached();
      expect(await inside(notes, methodDialog)).toBe(true);
      await page.keyboard.press('Escape');
      await expect(methodDialog).toHaveCount(0);
      await expect(method).toBeFocused();
      await page.mouse.move(0, 0);

      // Overview: the unknown merged count is said in words, one line, and its
      // explanation opens from the tile's definition on focus, without moving
      // anything.
      const closed = { effort: await height(effort), overview: await height(overview) };
      const merged = overview.getByTestId('overview-tile').filter({ hasText: 'Merged PRs' });
      await expect(merged.getByTestId('overview-value')).toHaveText('1so far');
      await expect(merged.locator('.xt-overview-sub')).toHaveText(
        /^1 (not checked yet|could not be checked)$/,
      );
      expect(await height(merged.locator('.xt-overview-sub'))).toBeLessThanOrEqual(ONE_LINE);
      const mergedInfo = overview.getByRole('button', { name: 'Merged PRs definition' });
      await mergedInfo.focus();
      const tooltipId = await mergedInfo.getAttribute('aria-describedby');
      expect(tooltipId).toBeTruthy();
      const ownedPopup = page.locator(`[role="tooltip"][id="${tooltipId}"]`);
      const gist = page.locator(`[role="tooltip"][id="${tooltipId}"][data-open]`);
      await expect(gist).toContainText('1 PR is not checked yet');
      expect(await inside(gist, page.locator('body'))).toBe(true);
      await page.keyboard.press('Escape');
      await expect(ownedPopup).toHaveCount(0);
      await expect(mergedInfo).toBeFocused();

      const measured = {
        closed,
        open: { effort: await height(effort), overview: await height(overview) },
        scrollWidth: await page.evaluate(() => document.documentElement.scrollWidth),
      };
      await info.attach('dashboard-geometry', { body: JSON.stringify(measured, null, 2) });
      await page
        .locator('.xt-dashboard')
        .screenshot({ path: info.outputPath(`compact-dashboard-${width}-${scheme}.png`) });
      expect(measured.scrollWidth).toBe(width);
      // Opening and closing the explanations left the cards exactly as they were.
      expect(measured.open).toEqual(measured.closed);
    });

    test(`sessions limits are one heading triangle that opens by keyboard at ${width}x${viewportHeight} ${scheme}`, async ({
      page,
      browserName,
    }, info) => {
      await open(page, '/sessions', width, viewportHeight, scheme);
      await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
      const heading = page.locator('.xt-sessions-heading');
      const flag = heading.getByTestId('sessions-issues');
      // Its name states the index state and the global untimed count, which is
      // never a share or a filtered count.
      await expect(flag).toHaveAccessibleName(
        'History limits: History incomplete, 2,318 untimed records',
      );
      await page.evaluate(() => document.fonts.ready);
      // No row of limits stands between the heading and the tiles; what the
      // tiles cover is the heading's own subtitle.
      await expect(page.locator('.xt-sessions-heading + .xt-sessions-summary')).toHaveCount(1);
      await expect(page.getByRole('region', { name: 'Untimed indexed history' })).toHaveCount(0);
      await expect(page.locator('.xt-sessions [role="status"]')).toHaveCount(0);
      await expect(heading.locator('p')).toHaveText(
        'Range: all indexed activity · filters: table only',
      );
      await expect(page.getByRole('region', { name: 'Range summary' })).toHaveAccessibleDescription(
        'The summary counts all indexed activity in the selected range; filters narrow only the table.',
      );
      // A small triangle, the heading's last control, at its right edge.
      const [box, head] = await Promise.all([flag.boundingBox(), heading.boundingBox()]);
      expect(box!.width).toBeLessThanOrEqual(20);
      expect(box!.y).toBeGreaterThanOrEqual(head!.y);
      expect(box!.y + box!.height).toBeLessThanOrEqual(head!.y + head!.height + 0.5);
      expect(head!.x + head!.width - (box!.x + box!.width)).toBeLessThanOrEqual(1);
      await page
        .locator('.xt-sessions')
        .screenshot({ path: info.outputPath(`compact-sessions-${width}-${scheme}.png`) });

      // Hover reads the gist.
      await flag.hover();
      const gist = page.getByRole('tooltip');
      await expect(gist).toContainText(
        'History incomplete: Some history could not be fully indexed.',
      );
      await expect(gist).toContainText('2,318 untimed records: outside dated measurements');
      await expect(gist).not.toContainText('%');
      await page.mouse.move(0, 0);
      await expect(gist).toHaveCount(0);
      // Focus reads it too, and Enter opens both explanations over the page.
      await flag.focus();
      await expect(gist).toContainText('History incomplete');
      await page.keyboard.press('Enter');
      const dialog = page.getByRole('dialog', { name: 'Index status and untimed history' });
      await expect(dialog).toBeVisible();
      await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
      await expect(dialog).toContainText('Some history could not be fully indexed.');
      await expect(dialog.getByTestId('untimed-count')).toHaveText(
        '2,318 records in all indexed history, independent of the selected dates and of any filter on the table.',
      );
      await expect(
        dialog.getByRole('list', { name: 'Untimed indexed history by host and surface' }),
      ).toBeVisible();
      // The Settings link is the next tab stop after Close.
      // (WebKit, like Safari, reaches links with Option-Tab.)
      await page.keyboard.press(browserName === 'webkit' ? 'Alt+Tab' : 'Tab');
      await expect(dialog.getByRole('link', { name: 'View indexing status' })).toBeFocused();
      const within = await dialog.evaluate((element) => {
        const box = element.getBoundingClientRect();
        return box.left >= -0.5 && box.right <= window.innerWidth + 0.5 && box.top >= -0.5;
      });
      expect(within).toBe(true);
      await page
        .locator('.xt-modal')
        .screenshot({ path: info.outputPath(`compact-sessions-dialog-${width}-${scheme}.png`) });
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(flag).toBeFocused();

      // The rows keep the page's remaining height and scroll on their own; the
      // page itself does not scroll, and the tiles and rows stay readable.
      const table = page.locator('.xt-sessions .xt-table-scroll');
      const measured = await page.evaluate(() => {
        const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
        const scroller = document.querySelector<HTMLElement>('.xt-sessions .xt-table-scroll')!;
        return {
          heading: document.querySelector('.xt-sessions-heading')!.getBoundingClientRect().height,
          outletOverflow: outlet.scrollHeight - outlet.clientHeight,
          pageOverflow: document.documentElement.scrollHeight - window.innerHeight,
          rowsViewport: scroller.getBoundingClientRect().height,
          scrolls: getComputedStyle(scroller).overflowY,
          scrollWidth: document.documentElement.scrollWidth,
        };
      });
      await info.attach('sessions-geometry', { body: JSON.stringify(measured, null, 2) });
      expect(measured.scrolls).toMatch(/auto|scroll/);
      expect(measured.outletOverflow).toBeLessThanOrEqual(0);
      expect(measured.pageOverflow).toBeLessThanOrEqual(0);
      expect(measured.scrollWidth).toBe(width);
      await expect(page.getByRole('region', { name: 'Range summary' })).toBeVisible();
      const tiles = page.locator('.xt-sessions-tiles');
      expect(await inside(tiles, page.locator('.xt-sessions'))).toBe(true);
      await expect(page.getByRole('table', { name: 'Indexed sessions' })).toBeVisible();
      await expect(table).toBeVisible();
    });
  }

test('healthy sessions draw no warning triangle', async ({ page }) => {
  // F1 as generated: its index status is fixture-disabled, which is not native, and no record is untimed.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/sessions');
  await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Range summary' })).toBeVisible();
  await expect(page.locator('.xt-sessions-tiles .xt-stat-tile').first()).toContainText('5');
  await expect(page.getByTestId('sessions-issues')).toHaveCount(0);
  await expect(page.locator('.xt-sessions details')).toHaveCount(0);
  await expect(page.locator('.xt-sessions [role="status"]')).toHaveCount(0);
});
