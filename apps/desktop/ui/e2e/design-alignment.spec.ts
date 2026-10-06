import { expect, test, type Locator, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { OVERVIEW_DEFINITION } from '../src/app/dashboard/overview';
import { ruleSummary } from '../src/kit/rules';

/**
 * Synthetic rows (not real history): F1 indexes one session, which never fills
 * the list, so its row is repeated under distinct IDs until the list must
 * scroll. Every measured value stays F1's own.
 */
const ROWS = 60;
const many = structuredClone(fixture as FixtureExport);
for (const page of many.sessions)
  page.rows = Array.from({ length: ROWS }, (_, index) => ({
    ...page.rows[0],
    id: `${String(index).padStart(8, '0')}-synthetic`,
  }));

const serve = (page: Page, body: FixtureExport) =>
  page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );

/** Each card and the definition it opens: a rule's summary, or the card's own plain words. */
const CARDS = [
  ['Effort', ruleSummary('M-19')],
  ['Overview', OVERVIEW_DEFINITION],
  ['Sessions', ruleSummary('M-05')],
] as const;
/**
 * Measurements opened from the card they belong to — tokens and cost as sections of Effort's
 * Details, coverage from the Sessions header — each with its definition beside its title.
 */
const METHOD = 'How effort is counted · daily values';
const DETAILS = [
  ['Tokens per day', 'M-04', 'Effort', /^Details$/, METHOD],
  ['API-equivalent cost', 'M-04', 'Effort', /^Details$/, METHOD],
  ['Coverage', 'M-18', 'Sessions', /^Coverage/, 'Coverage'],
] as const;

for (const scheme of ['light', 'dark'] as const)
  test(`Dashboard cards explain themselves from an info control, not an ID chip, in ${scheme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/dashboard');
    await expect(page.getByTestId('dashboard-summary')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    // No rule ID is visible chrome anywhere on the page.
    await expect(page.locator('.xt-rule-chip')).toHaveCount(0);
    const visible = await page.locator('.xt-dashboard').innerText();
    expect(visible).not.toMatch(/\b[MRC]-\d{2}\b/);
    await page.screenshot({ path: info.outputPath(`dashboard-${scheme}.png`) });

    const explain = async (control: Locator, text: string, shot?: string) => {
      await control.focus();
      await expect(control).toBeFocused();
      // Small, and drawn in the theme's own ink rather than as a tag.
      const box = await control.boundingBox();
      expect(box!.width).toBeLessThanOrEqual(16);
      const definition = page.getByRole('tooltip');
      await expect(definition).toContainText(text);
      if (shot) await page.screenshot({ path: info.outputPath(shot) });
      await page.keyboard.press('Escape');
      await expect(definition).toHaveCount(0);
    };
    // Agent / human hours and Caught by your rules are hidden, definitions and all.
    for (const title of ['Agent / human hours', 'Caught by your rules'])
      await expect(page.getByRole('button', { name: `${title} definition` })).toHaveCount(0);
    for (const [title, text] of CARDS)
      await explain(
        page.getByRole('button', { name: `${title} definition` }),
        text,
        title === 'Effort' ? `dashboard-definition-${scheme}.png` : undefined,
      );
    for (const [title, id, card, trigger, dialogName] of DETAILS) {
      // Opened from the keyboard, so the definition inside opens on focus as it does on the page.
      const item = page
        .getByRole('region', { name: card, exact: true })
        .getByRole('button', { name: trigger });
      await item.focus();
      await page.keyboard.press('Enter');
      const dialog = page.getByRole('dialog', { name: dialogName });
      await expect(dialog).toBeVisible();
      await explain(dialog.getByRole('button', { name: `${title} definition` }), ruleSummary(id));
      // Escape closed the definition and left its dialog; a second Escape closes that.
      await expect(dialog).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(item).toBeFocused();
    }
  });

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`Sessions rows scroll inside the list, not the page, at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      await serve(page, many);
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/sessions');
      const scroll = page.getByRole('region', { name: 'Indexed sessions scroll area' });
      await expect(page.getByText('Session 00000059', { exact: true })).toBeAttached();
      await page.evaluate(() => document.fonts.ready);

      const layout = () =>
        page.evaluate(() => {
          const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
          const region = document.querySelector<HTMLElement>('.xt-sessions .xt-table-scroll')!;
          const head = region.querySelector<HTMLElement>('.xt-table-head')!;
          const rows = [...region.querySelectorAll<HTMLElement>('.xt-data-row')];
          const card = region.closest<HTMLElement>('.xt-section-card')!;
          return {
            outlet: { scroll: outlet.scrollHeight, client: outlet.clientHeight },
            region: {
              scroll: region.scrollHeight,
              client: region.clientHeight,
              top: region.getBoundingClientRect().top,
              bottom: region.getBoundingClientRect().bottom,
            },
            headTop: head.getBoundingClientRect().top,
            lastBottom: rows.at(-1)!.getBoundingClientRect().bottom,
            rows: rows.length,
            cardBottom: card.getBoundingClientRect().bottom,
            pageWidth: document.documentElement.scrollWidth,
          };
        });

      const before = await layout();
      await info.attach('layout', { body: JSON.stringify(before, null, 2) });
      await page.screenshot({ path: info.outputPath(`sessions-${width}-${scheme}.png`) });
      // One vertical scroll: the page fits the window, the rows overflow their
      // own region, and the list reaches the bottom of the window.
      expect(before.outlet.scroll).toBeLessThanOrEqual(before.outlet.client);
      expect(before.region.scroll).toBeGreaterThan(before.region.client);
      expect(before.rows).toBe(ROWS);
      expect(before.cardBottom).toBeLessThanOrEqual(height);
      expect(before.cardBottom).toBeGreaterThan(height - 40);
      expect(before.pageWidth).toBe(width);

      // The keyboard reaches the last row; the header stays above the rows.
      await scroll.focus();
      await expect(scroll).toBeFocused();
      await page.keyboard.press('End');
      await expect
        .poll(() =>
          scroll.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop),
        )
        .toBeLessThanOrEqual(1);
      const after = await layout();
      expect(after.headTop).toBeCloseTo(before.headTop, 0);
      expect(after.lastBottom).toBeLessThanOrEqual(after.region.bottom + 0.5);
      expect(after.outlet.scroll).toBeLessThanOrEqual(after.outlet.client);
      await page.screenshot({ path: info.outputPath(`sessions-scrolled-${width}-${scheme}.png`) });
    });

test('a window too short for the list keeps its rows reachable by scrolling the page', async ({
  page,
}) => {
  await serve(page, many);
  await page.setViewportSize({ width: 1120, height: 400 });
  await page.goto('/sessions');
  await expect(page.getByText('Session 00000059', { exact: true })).toBeAttached();
  const measured = await page.evaluate(() => {
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    const region = document.querySelector<HTMLElement>('.xt-sessions .xt-table-scroll')!;
    return {
      outletScrolls: outlet.scrollHeight > outlet.clientHeight,
      regionHeight: region.clientHeight,
      regionScrolls: region.scrollHeight > region.clientHeight,
    };
  });
  // The list keeps a usable height instead of collapsing to nothing, and the
  // page scrolls to reach it; the rows still scroll inside it.
  expect(measured.outletScrolls).toBe(true);
  expect(measured.regionHeight).toBeGreaterThan(150);
  expect(measured.regionScrolls).toBe(true);
  const scroll = page.getByRole('region', { name: 'Indexed sessions scroll area' });
  await scroll.scrollIntoViewIfNeeded();
  await scroll.focus();
  await page.keyboard.press('End');
  await expect(page.getByText('Session 00000059', { exact: true })).toBeInViewport();
});
