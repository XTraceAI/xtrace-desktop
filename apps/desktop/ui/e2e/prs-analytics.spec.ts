import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import synthetic from '../src/app/prs-analytics.synthetic.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { FixturePrSessions } from '../src/data/generated/FixturePrSessions';
import type { PrAnalyticsPage } from '../src/data/generated/PrAnalyticsPage';

/**
 * The merged-PR report over the Rust-generated synthetic export (test-only,
 * not real history, not a design sample), served in place of F1's empty
 * report. The export holds every page; the browser fixture serves first pages
 * only, so paging is covered by the unit tests.
 */
type Scenario = {
  pr_analytics: PrAnalyticsPage[];
  pr_sessions: (FixturePrSessions & { after: string | null })[];
};
const scenarios = synthetic as unknown as Record<'measured' | 'sparse', Scenario>;

const withReport = (scenario: Scenario) => {
  const copy = structuredClone(fixture as FixtureExport);
  copy.pr_analytics = scenario.pr_analytics;
  // The browser fixture serves first pages only; the cursor is not its field.
  copy.pr_sessions = scenario.pr_sessions
    .filter((page) => page.after === null)
    .map((page) => ({
      repository: page.repository,
      number: page.number,
      confirmed_only: page.confirmed_only,
      window_days: page.window_days,
      window_end_ms: page.window_end_ms,
      page: page.page,
    }));
  return copy;
};

async function open(
  page: Page,
  {
    width,
    height,
    scheme = 'light',
    scenario = 'measured',
  }: {
    width: number;
    height: number;
    scheme?: 'light' | 'dark';
    scenario?: 'measured' | 'sparse' | 'missing';
  },
) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(
        // No exported report at all: the browser fixture's read fails.
        withReport(
          scenario === 'missing' ? { pr_analytics: [], pr_sessions: [] } : scenarios[scenario],
        ),
      )};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/prs');
  await expect(
    scenario === 'missing'
      ? page.getByRole('alert').filter({ hasText: 'report could not be read' })
      : page.getByRole('table', { name: 'Merged pull requests' }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

/** Geometry of the report view, measured in the page. */
const measure = (page: Page) =>
  page.evaluate(() => {
    const box = (element: Element) => element.getBoundingClientRect();
    const clipped = (element: HTMLElement) => element.scrollWidth > element.clientWidth + 0.5;
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    const scroll = document.querySelector<HTMLElement>('.xt-prs-main .xt-table-scroll')!;
    const rows = [...document.querySelectorAll<HTMLElement>('.xt-prs-main .xt-data-row')];
    return {
      pageWidth: document.documentElement.scrollWidth,
      outletScrolls: outlet.scrollHeight > outlet.clientHeight + 1,
      tableScrollsSideways: scroll.scrollWidth > scroll.clientWidth + 1,
      mainBottom: box(document.querySelector('.xt-prs-main')!).bottom,
      factsBottom: box(document.querySelector('[data-testid="prs-eligibility"]')!).bottom,
      viewport: innerHeight,
      visibleRows: rows.filter((row) => box(row).bottom <= box(scroll).bottom + 0.5).length,
      rowHeights: [...new Set(rows.map((row) => Math.round(box(row).height)))],
      // Values, samples, labels and headers are read whole; only free text
      // (titles, identities) may ellipsize, and then it carries a title.
      clippedValues: [
        ...document.querySelectorAll<HTMLElement>(
          '.xt-prs-tiles .xt-stat-label, .xt-prs-tiles .xt-stat-aside, .xt-prs-tiles .xt-metric-cell, .xt-prs-main .xt-metric-cell, .xt-prs-main .xt-pr-open, .xt-prs-main .xt-pr-evidence, .xt-prs-main .xt-pr-mono, .xt-prs-main [role="columnheader"], .xt-prs-main [role="columnheader"] > *, .xt-prs-type-values, .xt-prs-type-head',
        ),
      ]
        .filter(clipped)
        .map((element) => element.textContent),
      clippedWithoutTitle: [
        ...document.querySelectorAll<HTMLElement>(
          '.xt-prs-main .xt-pr-title, .xt-prs-main .xt-pr-meta, .xt-prs-main .xt-pr-type',
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
    };
  });

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`the report fits the window with every value whole at ${width}x${height} ${scheme}`, async ({
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
      await open(page, { width, height, scheme });
      await expect(page.getByRole('radiogroup', { name: 'Date range' })).toBeVisible();
      await expect(page.getByTestId('report-period')).toHaveText(/^Sep 1\s–\s7, 2026/);
      await expect(page.locator('.xt-prs-tiles .xt-stat-tile')).toHaveCount(4);
      const headers = await page
        .getByRole('table', { name: 'Merged pull requests' })
        .getByRole('columnheader')
        .allTextContents();
      expect(headers.map((header) => header.trim())).toEqual([
        'pull request',
        'type',
        'merged',
        'sessions',
        'msgs',
        'tokens',
        'agent',
        'hands-off',
        'evidence',
      ]);
      const layout = await measure(page);
      await info.attach('layout', { body: JSON.stringify(layout, null, 2) });
      await page.screenshot({ path: info.outputPath(`prs-report-${width}-${scheme}.png`) });
      expect(layout.pageWidth).toBe(width);
      expect(layout.outletScrolls).toBe(false);
      expect(layout.tableScrollsSideways).toBe(false);
      expect(layout.factsBottom).toBeLessThanOrEqual(layout.viewport);
      expect(layout.rowHeights).toEqual([40]);
      expect(layout.visibleRows).toBeGreaterThanOrEqual(width === 1120 ? 6 : 8);
      expect(layout.clippedValues).toEqual([]);
      expect(layout.clippedWithoutTitle).toEqual([]);
      expect(layout.outsideRow).toEqual([]);
      expect(errors).toEqual([]);
      expect(external).toEqual([]);
    });

test('opens and closes the drilldown by keyboard, bounded, with its rows in a focusable region', async ({
  page,
}) => {
  await open(page, { width: 1120, height: 720 });
  const trigger = page.getByRole('button', {
    name: /^3 linked sessions, 3 active in range: open the linked sessions of example\/atlas#101$/,
  });
  await trigger.focus();
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Linked sessions' });
  await expect(dialog).toBeVisible();
  // Close is first, so the definition tooltips do not open on arrival.
  await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
  await expect(dialog.getByText('Session s-alpha')).toBeVisible();
  await expect(
    dialog.getByText('3 of 3 linked sessions shown · Sessions list order'),
  ).toBeVisible();
  const bounds = await dialog.evaluate((element) => {
    const box = element.getBoundingClientRect();
    return {
      inside: box.top >= 0 && box.left >= 0 && box.bottom <= innerHeight && box.right <= innerWidth,
      scrolls: element.scrollHeight > element.clientHeight + 1,
    };
  });
  expect(bounds).toEqual({ inside: true, scrolls: false });
  const region = dialog.getByRole('region', {
    name: 'Linked sessions of example/atlas#101 scroll area',
  });
  await region.focus();
  await expect(region).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('the unknown token total opens the same drilldown, and the idle member is listed', async ({
  page,
}) => {
  await open(page, { width: 1440, height: 900, scheme: 'dark' });
  await page
    .getByRole('button', {
      name: /no selected usage in this range: open the linked sessions of example\/atlas#102$/,
    })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Linked sessions' });
  const idle = dialog.getByRole('row').filter({ hasText: 'Session s-idle' });
  await expect(idle).toContainText('idle');
  await expect(idle).toContainText('inferred');
  await dialog.getByRole('button', { name: 'Close' }).click();
  await expect(dialog).toHaveCount(0);
});

for (const scheme of ['light', 'dark'] as const)
  test(`no known merged PR with unknown facts is said, and the inventory is one step away — ${scheme}`, async ({
    page,
  }) => {
    await open(page, { width: 1120, height: 720, scheme, scenario: 'sparse' });
    await expect(
      page.getByText(
        /^No merged pull request is known in this range\. 2 linked pull requests have unknown cached facts/,
      ),
    ).toBeVisible();
    await expect(page.locator('.xt-prs-tiles .xt-unmeasured')).toHaveCount(4);
    await expect(page.getByTestId('prs-types')).toHaveText(
      'No known merged PR to group; 2 linked PRs have unknown cached facts.',
    );
    const layout = await measure(page);
    expect(layout.outletScrolls).toBe(false);
    expect(layout.clippedValues).toEqual([]);
    const inventory = page.getByRole('button', { name: 'Cached inventory' });
    await inventory.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('table', { name: 'Linked pull requests' })).toBeVisible();
    await expect(page).toHaveURL(/\/prs\?view=inventory$/);
    await expect(page.getByRole('radiogroup', { name: 'Date range' })).toHaveCount(0);
    await page.goBack();
    await expect(page.getByRole('table', { name: 'Merged pull requests' })).toBeVisible();
  });

for (const scheme of ['light', 'dark'] as const)
  test(`a failed report read is never shown as no data or as loading — ${scheme}`, async ({
    page,
  }) => {
    await open(page, { width: 1120, height: 720, scheme, scenario: 'missing' });
    const failed = 'Not available: the report could not be read';
    const reasons = await page
      .locator('.xt-prs-tiles .xt-unmeasured')
      .evaluateAll((items) => items.map((item) => item.getAttribute('title')));
    expect(reasons).toEqual([failed, failed, failed, failed]);
    await expect(page.getByTestId('prs-types')).toHaveText(`${failed}.`);
    await expect(page.getByText(/No merged pull request|Reading/)).toHaveCount(0);
    await expect(page.getByTestId('prs-eligibility')).toHaveCount(0);
    await expect(page.getByRole('alert').getByRole('button', { name: 'Retry' })).toBeVisible();
    const fits = await page.evaluate(() => {
      const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
      return {
        outletScrolls: outlet.scrollHeight > outlet.clientHeight + 1,
        pageWidth: document.documentElement.scrollWidth,
      };
    });
    expect(fits).toEqual({ outletScrolls: false, pageWidth: 1120 });
  });
