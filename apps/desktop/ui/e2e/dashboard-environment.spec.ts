import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import { syntheticEnvironment } from '../src/app/dashboard/environment.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The Environment panel over the generated F1 export, and over the test-only synthetic report
 * (eleven identities; not real history, not a design sample) served in place of the F1 export.
 */
const synthetic = structuredClone(fixture as FixtureExport);
synthetic.environments = synthetic.environments.map(syntheticEnvironment);

async function open(
  page: Page,
  {
    width,
    height,
    scheme = 'light',
    dense = true,
  }: {
    width: number;
    height: number;
    scheme?: 'light' | 'dark';
    dense?: boolean;
  },
) {
  if (dense)
    await page.route('**/fixtures/F1.json?import', (route) =>
      route.fulfill({
        contentType: 'text/javascript',
        body: `export default ${JSON.stringify(synthetic)};`,
      }),
    );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(panel(page).getByTestId('environment-summary')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const panel = (page: Page) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Environment', exact: true }),
  });
/** The card's Environment + Effort row must leave Sessions in the first 1440x900 view. */
const MAX_CARD_HEIGHT = { 1440: 400, 1120: 440 } as const;
const row = (page: Page, name: string) =>
  panel(page)
    .getByRole('list', { name: /^Most-called identities/ })
    .getByRole('listitem')
    .filter({ has: page.locator('.xt-env-name', { hasText: name }) });

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`environment keeps its place and fits at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      await open(page, { width, height, scheme });
      await expect(
        panel(page).getByRole('list', { name: 'Most-called identities, top 8 of 11' }),
      ).toBeVisible();
      const layout = await page.evaluate(() => {
        const section = (name: string) =>
          [...document.querySelectorAll('.xt-dashboard h2')]
            .find((h) => h.textContent === name)!
            .closest('section')!
            .getBoundingClientRect();
        const card = section('Environment');
        const inside = [...document.querySelectorAll<HTMLElement>('.xt-env *')].every((element) => {
          const box = element.getBoundingClientRect();
          return box.width === 0 || (box.left >= card.left - 0.5 && box.right <= card.right + 0.5);
        });
        // A compact row truncates its name and kind to one 24px line; the full text is in the DOM,
        // the name's title and the observed dialog, so those two are checked separately below.
        const compact =
          '.xt-env-list[data-compact] .xt-env-name, .xt-env-list[data-compact] .xt-kind';
        const clipped = [...document.querySelectorAll<HTMLElement>('.xt-dashboard *')]
          .filter((element) => {
            if (element.closest('.xt-dash-table-scroll, .xt-lanes-scroll, .sr-only')) return false;
            if (element.matches(compact)) return false;
            const style = getComputedStyle(element);
            const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
            return clips && element.scrollWidth > element.clientWidth + 1;
          })
          .map((element) => `${element.className}: ${element.textContent}`);
        const list = document.querySelector<HTMLElement>('.xt-env-list[data-compact]')!;
        const rows = [...list.querySelectorAll<HTMLElement>(':scope > li')];
        return {
          list: {
            clientHeight: list.clientHeight,
            scrollHeight: list.scrollHeight,
            tabIndex: list.tabIndex,
          },
          rowHeights: [
            ...new Set(rows.map((item) => Math.round(item.getBoundingClientRect().height))),
          ],
          untitled: rows
            .map((item) => item.querySelector<HTMLElement>('.xt-env-name')!)
            .filter((name) => !name.title.startsWith(name.firstChild!.textContent!))
            .map((name) => name.textContent),
          scrollWidth: document.documentElement.scrollWidth,
          outletOverflow:
            document.querySelector('.xt-shell-outlet')!.scrollWidth -
            document.querySelector('.xt-shell-outlet')!.clientWidth,
          effort: section('Effort by type'),
          environment: card,
          sessions: section('Sessions'),
          inside,
          clipped,
        };
      });
      // Captured before the assertions so a failure still leaves the card to look at.
      await page.screenshot({ path: info.outputPath(`environment-${width}-${scheme}.png`) });
      // Effort by type on the left at 1.6:1, Environment on the right, then Sessions.
      expect(layout.environment.top).toBe(layout.effort.top);
      expect(layout.environment.left).toBeGreaterThan(layout.effort.right);
      expect(layout.effort.width / layout.environment.width).toBeCloseTo(1.6, 1);
      expect(layout.sessions.top).toBeGreaterThan(layout.environment.bottom);
      expect(layout.scrollWidth).toBe(width);
      expect(layout.outletOverflow).toBeLessThanOrEqual(0);
      expect(layout.inside).toBe(true);
      expect(layout.clipped).toEqual([]);
      expect(layout.untitled).toEqual([]);
      // Dense rows are one 24px line, and the list is bounded: eight rows scroll inside it.
      expect(layout.rowHeights).toEqual([24]);
      expect(layout.list.tabIndex).toBe(0);
      expect(layout.list.clientHeight).toBeLessThanOrEqual(6 * 24 + 2);
      expect(layout.list.scrollHeight).toBeGreaterThan(layout.list.clientHeight);
      // The dense card stays compact, and at 1440x900 Sessions is in the first view.
      expect(layout.environment.height).toBeLessThanOrEqual(MAX_CARD_HEIGHT[width]);
      expect(layout.effort.height).toBe(layout.environment.height);
      if (width === 1440) expect(layout.sessions.bottom).toBeLessThanOrEqual(height);
      await info.attach('environment-layout', {
        body: JSON.stringify(layout, null, 2),
        contentType: 'application/json',
      });
      // Both dialogs fit the window without horizontal overflow.
      for (const [trigger, name] of [
        ['Unresolved attribution details', 'Unresolved attribution'],
        ['Observed identities · 11', 'Observed identities'],
        ['Configured components · 14', 'Configured components'],
      ] as const) {
        await panel(page).getByRole('button', { name: trigger }).click();
        const dialog = page.getByRole('dialog', { name });
        await expect(dialog).toBeVisible();
        const box = (await dialog.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(width);
        expect(box.height).toBeLessThanOrEqual(height);
        expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        await page.screenshot({
          path: info.outputPath(`environment-${width}-${scheme}-${name.split(' ')[0]}.png`),
        });
        await page.keyboard.press('Escape');
        await expect(dialog).toHaveCount(0);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      expect(errors).toEqual([]);
    });

test('the F1 export shows real observed usage, not the unavailable placeholder', async ({
  page,
}) => {
  await open(page, { width: 1440, height: 900, dense: false });
  await expect(panel(page).getByTestId('environment-summary')).toContainText(
    '5 observed calls · last 7d · 1 identity in either window',
  );
  await expect(panel(page)).not.toContainText('Environment data is unavailable');
  await expect(row(page, 'Read').locator('.xt-env-calls')).toHaveText('5 calls');
  await expect(row(page, 'Read').getByRole('img')).toHaveCount(14);
  await expect(panel(page).getByTestId('environment-unresolved')).toHaveCount(0);
});

test('the bounded observed list scrolls with the keyboard and keeps all eight rows', async ({
  page,
}) => {
  await open(page, { width: 1440, height: 900 });
  const list = panel(page).getByRole('list', { name: 'Most-called identities, top 8 of 11' });
  await expect(list.getByRole('listitem')).toHaveCount(8);
  await list.focus();
  await expect(list).toBeFocused();
  expect(await list.evaluate((node) => node.scrollTop)).toBe(0);
  await page.keyboard.press('End');
  await expect
    .poll(() => list.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
    .toBeLessThanOrEqual(1);
  // The eighth row, below the fold of the list, is in view once scrolled.
  const last = list.getByRole('listitem').last();
  await expect(last).toContainText('review');
  const [item, box] = await Promise.all([last.boundingBox(), list.boundingBox()]);
  expect(item!.y + item!.height).toBeLessThanOrEqual(box!.y + box!.height + 1);
  await page.keyboard.press('Home');
  await expect.poll(() => list.evaluate((node) => node.scrollTop)).toBe(0);
});

test('unresolved counts carry no denominator and untimed observations stay apart', async ({
  page,
}) => {
  await open(page, { width: 1120, height: 720, scheme: 'dark' });
  const note = panel(page).getByRole('note', { name: 'Unresolved attribution' });
  await expect(note).toContainText('38 unresolved observations, including untimed');
  await expect(note).not.toContainText(/ of |104/);
  const trigger = note.getByRole('button', { name: 'Unresolved attribution details' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Unresolved attribution' });
  await expect(dialog.getByRole('list', { name: 'Unresolved in the last 7d' })).not.toContainText(
    /timestamp|untimed/,
  );
  await expect(dialog.getByRole('list', { name: 'Untimed observations' })).toHaveText(
    'codex · 7 untimed observations: no timestamp, so no selected range can hold it',
  );
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('keyboard reaches every observed identity and the configured details', async ({ page }) => {
  await open(page, { width: 1120, height: 720, scheme: 'dark' });
  const all = panel(page).getByRole('button', { name: 'Observed identities · 11' });
  await all.focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Observed identities' });
  await expect(dialog).toBeVisible();
  const list = dialog.getByRole('list', { name: 'All 11 observed' });
  await expect(list.getByRole('listitem')).toHaveCount(11);
  await expect(list.getByRole('listitem').last()).toContainText('apply_patch');
  // The list itself takes focus for keyboard scrolling, and focus never leaves for the page.
  await page.keyboard.press('Tab');
  await expect(list).toBeFocused();
  await page.keyboard.press('PageDown');
  // Each Tab settles inside the dialog again (through its focus guards), never on the page.
  for (let step = 0; step < 4; step += 1) {
    await page.keyboard.press('Tab');
    await expect
      .poll(() => dialog.evaluate((node) => node.contains(document.activeElement)))
      .toBe(true);
  }
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(all).toBeFocused();
  // Tab moves on to the configured details, which open with the keyboard too.
  await page.keyboard.press('Tab');
  const configured = panel(page).getByRole('button', { name: 'Configured components · 14' });
  await expect(configured).toBeFocused();
  await page.keyboard.press('Space');
  const details = page.getByRole('dialog', { name: 'Configured components' });
  await expect(details).toContainText('does not show that a component is installed');
  await expect(details.getByRole('heading', { name: 'Cache only' })).toBeVisible();
  await expect(details.getByRole('list', { name: 'Configuration sources' })).toBeVisible();
  await details.getByRole('button', { name: 'Close' }).press('Enter');
  await expect(details).toHaveCount(0);
  await expect(configured).toBeFocused();
});

test('range presets change selected totals while strips keep the fixed 14 dates', async ({
  page,
}) => {
  await open(page, { width: 1440, height: 900 });
  const range = page.getByRole('radiogroup', { name: 'Date range' });
  const bash = row(page, 'Bash');
  const first = bash.getByRole('img').first();
  for (const [days, calls, total] of [
    [14, '84 calls', '208 observed calls'],
    [30, '168 calls', '416 observed calls'],
    [7, '42 calls', '104 observed calls'],
  ] as const) {
    await range.getByRole('radio', { name: `${days}d` }).click();
    await expect(panel(page).getByTestId('environment-summary')).toContainText(
      `${total} · last ${days}d · 11 identities in either window`,
    );
    await expect(bash.locator('.xt-env-calls')).toHaveText(calls);
    await expect(bash.getByRole('img')).toHaveCount(14);
    await expect(first).toHaveAttribute('aria-label', 'Aug 25: 0');
    await expect(page.locator('.xt-token-heading > span')).toHaveText(`Recorded tokens · ${days}d`);
  }
  await expect(
    bash
      .getByRole('img')
      .evaluateAll((cells) => cells.slice(0, 4).map((cell) => cell.getAttribute('data-level'))),
  ).resolves.toEqual(['0', '1', '2', '3']);
});
