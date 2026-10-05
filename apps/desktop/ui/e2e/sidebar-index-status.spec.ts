import { expect, test, type Locator, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { NativeHostState } from '../src/data/generated/NativeHostState';
import type { NativeIndexStatus } from '../src/data/generated/NativeIndexStatus';

/**
 * The sidebar's local-index row in the real Shell, over test-only synthetic
 * statuses served in place of the F1 export's (not real history, and not a
 * design sample): the compact row keeps its geometry in both schemes at both
 * native widths, its panel opens and closes from the keyboard inside the
 * window, and nothing about a plugin is inferred from the index or the other
 * way round.
 */
const scan = (host: string, state: NativeHostState, detail: string | null = null) => ({
  host,
  state,
  detail,
  sessions_imported: 1,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 3,
  records_enriched: 0,
  diagnostics: 0,
});
const ready: NativeIndexStatus = {
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/synthetic/python3' },
  readers: { state: 'verified', commit: '0'.repeat(40), plugin_version: '0.0.0' },
  hosts: [scan('claude', 'complete'), scan('cursor', 'missing_source')],
  reconciles: 2,
  files_scanned: 3,
};
const partial: NativeIndexStatus = {
  ...ready,
  hosts: [
    scan('claude', 'incomplete'),
    // One unbroken token, far wider than the panel: it has to wrap inside it.
    scan('codex', 'reader_failed', `synthetic-reader-detail-${'x'.repeat(120)}`),
    scan('cursor', 'complete'),
  ],
};
const scanning: NativeIndexStatus = {
  ...ready,
  phase: { phase: 'scanning' },
  freshness: { freshness: 'unknown' },
  hosts: [scan('claude', 'pending'), scan('codex', 'pending'), scan('cursor', 'pending')],
  reconciles: 0,
};

async function open(page: Page, index: NativeIndexStatus | null, width: number, height: number) {
  if (index)
    await page.route('**/fixtures/F1.json?import', (route) =>
      route.fulfill({
        contentType: 'text/javascript',
        body: `export default ${JSON.stringify({ ...(fixture as FixtureExport), native_index: index })};`,
      }),
    );
  await page.setViewportSize({ width, height });
  await page.goto('/dashboard');
  await page.evaluate(() => document.fonts.ready);
}
const trigger = (page: Page) => page.getByRole('button', { name: /^Local index:/ });
const panel = (page: Page) => page.getByRole('dialog', { name: 'Local index and plugin status' });
/** A theme token's colour as the sidebar resolves it. */
const token = (page: Page, name: string) =>
  page.evaluate((variable) => {
    const probe = document.createElement('span');
    probe.style.color = `var(${variable})`;
    document.querySelector('.xt-sidebar')!.append(probe);
    const { color } = getComputedStyle(probe);
    probe.remove();
    return color;
  }, name);
const box = async (locator: Locator) => (await locator.boundingBox())!;
/** The row's words are drawn whole: nothing is cut to an ellipsis. */
const unclipped = (locator: Locator) =>
  locator.evaluate((element) => element.scrollWidth <= element.clientWidth);

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`index row and panel keep their geometry and keyboard at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      page.on('console', (message) => {
        if (message.type() === 'error') errors.push(message.text());
      });
      await page.emulateMedia({ colorScheme: scheme });
      await open(page, partial, width, height);
      const row = trigger(page);
      await expect(row).toHaveText('index · partial');
      await expect(row).toHaveAccessibleName('Local index: Updating · partial');
      // A live watcher does not get the green dot while a host scan is partial.
      await expect(row.locator('i')).toHaveCSS('background-color', await token(page, '--warning'));

      // The compact row: one line, whole words, beside the two footer controls.
      const sidebar = page.getByRole('complementary', { name: 'Workspace' });
      const label = row.locator('span');
      const footer = await box(sidebar.locator('.xt-sidebar-status'));
      const rowBox = await box(row);
      const actions = await box(sidebar.locator('.xt-sidebar-actions'));
      expect((await box(sidebar)).width).toBe(228);
      expect(rowBox.height).toBeLessThanOrEqual(18);
      expect(rowBox.x).toBeGreaterThanOrEqual(footer.x);
      expect(rowBox.x + rowBox.width).toBeLessThanOrEqual(actions.x);
      expect(actions.x + actions.width).toBeLessThanOrEqual(footer.x + footer.width + 0.5);
      expect(await unclipped(label)).toBe(true);
      expect(await sidebar.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
      // Layout probe: the widest state the presenter can word (17 characters,
      // a last-known state) is drawn whole in the same row.
      await label.evaluate((element) => (element.textContent = 'index · updating?'));
      expect(await unclipped(label)).toBe(true);
      expect((await box(row)).x + (await box(row)).width).toBeLessThanOrEqual(actions.x);
      await label.evaluate((element) => (element.textContent = 'index · partial'));

      // Keyboard: a visible focus ring, Enter opens, Escape closes and returns focus.
      await row.focus();
      await expect(row).toBeFocused();
      expect(await row.evaluate((element) => getComputedStyle(element).outlineStyle)).not.toBe(
        'none',
      );
      await page.keyboard.press('Enter');
      const dialog = panel(page);
      await expect(dialog).toBeVisible();
      await expect(row).toHaveAttribute('aria-expanded', 'true');

      // The panel sits beside the sidebar, whole and inside the window.
      const dialogBox = await box(dialog);
      expect(dialogBox.width).toBe(264);
      expect(dialogBox.x).toBeGreaterThanOrEqual(228);
      expect(dialogBox.y).toBeGreaterThanOrEqual(0);
      expect(dialogBox.x + dialogBox.width).toBeLessThanOrEqual(width);
      expect(dialogBox.y + dialogBox.height).toBeLessThanOrEqual(height);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      // The long reported reason wraps inside it instead of widening it.
      expect(
        await dialog.evaluate((element) => {
          const frame = element.getBoundingClientRect();
          return [...element.querySelectorAll<HTMLElement>('*')]
            .map((node) => node.getBoundingClientRect())
            .filter((rect) => rect.width > 1)
            .every((rect) => rect.left >= frame.left - 0.5 && rect.right <= frame.right + 0.5);
        }),
      ).toBe(true);

      // What it says: the index, each host's last scan, and the plugin apart.
      await expect(dialog.getByText('Updating · partial')).toHaveCSS(
        'color',
        await token(page, '--warning'),
      );
      // A scan that did not complete is said to leave the index possibly short,
      // never that stored history is missing.
      await expect(
        dialog.getByText(
          /Last scan not complete for claude \(incomplete\), codex \(reader failed\)\. The index may be incomplete or out of date for those hosts\./,
        ),
      ).toBeVisible();
      await expect(dialog.getByText('claude', { exact: true }).locator('..')).toContainText(
        'Incomplete',
      );
      await expect(dialog.getByText('codex', { exact: true }).locator('..')).toContainText(
        'Reader failed',
      );
      await expect(dialog.getByText('Plugin receiver').locator('..')).toHaveText(
        'Plugin receiverOff',
      );
      await expect(dialog.getByText('Plugin delivery').locator('..')).toHaveText(
        'Plugin deliveryUnknown',
      );
      // Nothing is inferred from the receiver being off, and nothing is invented.
      const said = (await dialog.innerText()).toLowerCase();
      for (const absent of ['capturing', 'install', '47421', 'connected'])
        expect(said).not.toContain(absent);
      expect(said).not.toMatch(/:\d/);
      await page.screenshot({ path: info.outputPath(`index-panel-${width}-${scheme}.png`) });

      await page.keyboard.press('Escape');
      await expect(dialog).toBeHidden();
      await expect(row).toBeFocused();
      await expect(row).toHaveAttribute('aria-expanded', 'false');
      expect(errors).toEqual([]);
    });

for (const scheme of ['light', 'dark'] as const)
  test(`the dot is green only for a live watcher with nothing to qualify (${scheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await open(page, ready, 1120, 720);
    await expect(trigger(page)).toHaveText('index · updating');
    await expect(trigger(page).locator('i')).toHaveCSS(
      'background-color',
      await token(page, '--success'),
    );
    await trigger(page).click();
    // An absent source is its own neutral fact, not a failure and not “complete”.
    await expect(panel(page).getByText('cursor', { exact: true }).locator('..')).toHaveText(
      'cursorNo local history found',
    );
    // Neutral states are plain values: neither the warning nor the success colour.
    for (const words of ['No local history found', 'Complete'])
      await expect(panel(page).getByText(words, { exact: true })).toHaveCSS(
        'color',
        await token(page, '--ink'),
      );
    expect(await token(page, '--ink')).not.toBe(await token(page, '--warning'));
  });

test('a scanning index is neither green nor a warning, and its waiting hosts are not gaps', async ({
  page,
}) => {
  await open(page, scanning, 1120, 720);
  await expect(trigger(page)).toHaveText('index · scanning');
  await expect(trigger(page).locator('i')).toHaveCSS(
    'background-color',
    await token(page, '--meta'),
  );
  await trigger(page).click();
  await expect(panel(page).getByText('Not scanned yet')).toHaveCount(3);
  await expect(panel(page).getByText(/Last scan not complete/)).toHaveCount(0);
});

test('the fixture’s own disabled index is shown as disabled, and Settings opens from the panel by keyboard', async ({
  page,
}) => {
  await open(page, null, 1120, 720);
  const row = trigger(page);
  await expect(row).toHaveText('index · disabled');
  await expect(row.locator('i')).toHaveCSS('background-color', await token(page, '--warning'));
  await row.focus();
  await page.keyboard.press('Enter');
  const dialog = panel(page);
  await expect(
    dialog.getByText(`Local history is not being indexed: ${fixture.native_index.phase.reason}.`),
  ).toBeVisible();
  await expect(dialog.getByText('No host scan reported')).toBeVisible();
  const settings = dialog.getByRole('button', { name: 'Index details in Settings' });
  await settings.focus();
  await expect(settings).toBeFocused();
  expect(await settings.evaluate((element) => getComputedStyle(element).outlineStyle)).not.toBe(
    'none',
  );
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Settings');
  await expect(dialog).toBeHidden();
  // The full diagnostics stay where they were.
  await expect(page.getByTestId('native-index')).toContainText(
    `Disabled: ${fixture.native_index.phase.reason}`,
  );
});

test('the index panel and the Hub panel never stay open together', async ({ page }) => {
  await open(page, ready, 1120, 720);
  const hub = page.getByRole('button', { name: 'XTrace Hub', exact: true });
  await trigger(page).click();
  await expect(panel(page)).toBeVisible();
  await hub.click();
  await expect(panel(page)).toBeHidden();
  await expect(page.getByRole('dialog', { name: /Connect this desktop/ })).toBeVisible();
  await trigger(page).click();
  await expect(page.getByRole('dialog', { name: /Connect this desktop/ })).toBeHidden();
  await expect(panel(page)).toBeVisible();
});
