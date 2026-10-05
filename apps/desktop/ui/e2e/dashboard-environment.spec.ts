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
  await expect(panel(page).getByTestId(/^environment-(columns|empty)$/)).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const panel = (page: Page) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Environment', exact: true }),
  });
type Columns = {
  head: {
    top: number;
    bottom: number;
    height: number;
    text: (string | null)[];
    cells: number[][];
    insideList: boolean;
  };
  listTop: number;
  rowRight: number[];
  rowCells: number[][][];
  rowOrder: string[][];
  gutter: number[];
};
/** The header and every row share the three column tracks; see the first case for the rule. */
function expectColumns(columns: Columns, range: string) {
  expect(columns.head.text).toEqual(['Tool · kind · host', '14 days', `Calls · last ${range}`]);
  expect(columns.head.insideList).toBe(false);
  expect(columns.head.height).toBeLessThanOrEqual(16);
  expect(columns.head.bottom).toBeLessThanOrEqual(columns.listTop + 0.5);
  expect(columns.gutter[0]).toBe(columns.gutter[1]);
  const [tool, legend, calls] = columns.head.cells;
  expect(columns.rowCells.length).toBeGreaterThan(0);
  columns.rowCells.forEach((cells, index) => {
    expect(columns.rowOrder[index]).toEqual(['xt-env-tool', 'xt-day-strip', 'xt-env-calls']);
    const [rowTool, rowStrip, rowCalls] = cells;
    expect(rowTool[0]).toBeCloseTo(tool[0], 1);
    expect(rowTool[1]).toBeCloseTo(tool[1], 1);
    expect(rowStrip[0]).toBeCloseTo(legend[0] + 4, 1);
    expect(rowCalls[1]).toBeCloseTo(calls[1], 1);
    // A row's box already ends where the list's gutter begins; the count ends 2px inside it.
    expect(rowCalls[1]).toBeCloseTo(columns.rowRight[index] - 2, 1);
    expect(rowStrip[1]).toBeLessThan(rowCalls[0]);
    expect(rowTool[1]).toBeLessThan(rowStrip[0]);
  });
}
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
        // A compact row truncates its name, kind and host to one 24px line; the full text is in
        // the DOM, the tool's title and the observed dialog, so those are checked separately below.
        const compact =
          '.xt-env-list[data-compact] .xt-env-name, .xt-env-list[data-compact] .xt-kind, .xt-env-list[data-compact] .xt-env-host';
        const clipped = [...document.querySelectorAll<HTMLElement>('.xt-dashboard *')]
          .filter((element) => {
            if (element.closest('.xt-dash-table-scroll, .xt-table-scroll, .sr-only')) return false;
            if (element.matches(compact)) return false;
            const style = getComputedStyle(element);
            const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
            return clips && element.scrollWidth > element.clientWidth + 1;
          })
          .map((element) => `${element.className}: ${element.textContent}`);
        const list = document.querySelector<HTMLElement>('.xt-env-list[data-compact]')!;
        const rows = [...list.querySelectorAll<HTMLElement>(':scope > li')];
        const head = document.querySelector<HTMLElement>('.xt-env-head')!;
        const edges = (cells: Element[]) =>
          cells.map((cell) => {
            const box = cell.getBoundingClientRect();
            return [Math.round(box.left * 100) / 100, Math.round(box.right * 100) / 100];
          });
        return {
          columns: {
            head: {
              top: head.getBoundingClientRect().top,
              bottom: head.getBoundingClientRect().bottom,
              height: head.getBoundingClientRect().height,
              text: [...head.children].map((cell) => cell.textContent),
              cells: edges([...head.children]),
              insideList: list.contains(head),
            },
            listTop: list.getBoundingClientRect().top,
            rowRight: rows.map(
              (item) => Math.round(item.getBoundingClientRect().right * 100) / 100,
            ),
            rowCells: rows.map((item) => edges([...item.children])),
            rowOrder: rows.map((item) =>
              [...item.children].map((cell) => cell.className.split(' ')[0]),
            ),
            gutter: [list.offsetWidth - list.clientWidth, head.offsetWidth - head.clientWidth],
          },
          list: {
            clientHeight: list.clientHeight,
            scrollHeight: list.scrollHeight,
            tabIndex: list.tabIndex,
          },
          rowHeights: [
            ...new Set(rows.map((item) => Math.round(item.getBoundingClientRect().height))),
          ],
          untitled: rows
            .map((item) => item.querySelector<HTMLElement>('.xt-env-tool')!)
            .filter(
              (tool) => !tool.title.startsWith(tool.querySelector('.xt-env-name')!.textContent!),
            )
            .map((tool) => tool.textContent),
          scrollWidth: document.documentElement.scrollWidth,
          outletOverflow:
            document.querySelector('.xt-shell-outlet')!.scrollWidth -
            document.querySelector('.xt-shell-outlet')!.clientWidth,
          effort: section('Effort'),
          environment: card,
          sessions: section('Sessions'),
          inside,
          clipped,
        };
      });
      // Captured before the assertions so a failure still leaves the card to look at.
      await page.screenshot({ path: info.outputPath(`environment-${width}-${scheme}.png`) });
      // Effort on the left at 1.6:1, Environment on the right, then Sessions.
      expect(layout.environment.top).toBe(layout.effort.top);
      expect(layout.environment.left).toBeGreaterThan(layout.effort.right);
      expect(layout.effort.width / layout.environment.width).toBeCloseTo(1.6, 1);
      expect(layout.sessions.top).toBeGreaterThan(layout.environment.bottom);
      expect(layout.scrollWidth).toBe(width);
      expect(layout.outletOverflow).toBeLessThanOrEqual(0);
      expect(layout.inside).toBe(true);
      expect(layout.clipped).toEqual([]);
      expect(layout.untitled).toEqual([]);
      // Dense rows are one 24px line, and the list is bounded: it shows at least three
      // of its eight rows, more in a taller window, and the rest scroll inside it.
      expect(layout.rowHeights).toEqual([24]);
      expect(layout.list.tabIndex).toBe(0);
      expect(layout.list.clientHeight).toBeGreaterThanOrEqual(3 * 24);
      expect(layout.list.clientHeight).toBeLessThanOrEqual(8 * 24 + 2);
      expect(layout.list.scrollHeight).toBeGreaterThanOrEqual(8 * 24 - 1);
      // One column header, outside the rows that scroll and not one of them, on the rows' own
      // tracks: TOOL · KIND starts where every tool starts, 14 DAYS (the legend control, whose
      // box begins 4px before its track so its focus ring surrounds the label) where every strip
      // starts, and CALLS ends where every right-aligned count ends, 2px inside the row's edge.
      // Header and list reserve the same scrollbar gutter, none with an overlay scrollbar.
      expectColumns(layout.columns, '7d');
      // The dense card shares Effort's height, and Sessions is in view at both sizes.
      expect(layout.effort.height).toBe(layout.environment.height);
      expect(layout.sessions.bottom).toBeLessThanOrEqual(height);
      await info.attach('environment-layout', {
        body: JSON.stringify(layout, null, 2),
        contentType: 'application/json',
      });
      // Both dialogs fit the window without horizontal overflow.
      for (const [trigger, name] of [
        [/^Unresolved attribution: /, 'Unresolved attribution'],
        ['Observed identities · 11', 'Observed identities · last 7d'],
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

test('the F1 export omits its built-in calls but keeps the configured facts', async ({ page }) => {
  await open(page, { width: 1440, height: 900, dense: false });
  // The card shows no summary line of call and identity totals.
  await expect(panel(page).getByTestId('environment-summary')).toHaveCount(0);
  await expect(panel(page)).not.toContainText('Environment data is unavailable');
  await expect(panel(page).getByTestId('environment-empty')).toContainText(
    'No non-built-in tool calls',
  );
  await expect(
    panel(page).getByRole('button', { name: 'Configured components · 14' }),
  ).toBeVisible();
  await expect(panel(page).locator('.xt-env-row')).toHaveCount(0);
  await expect(panel(page).getByTestId('environment-unresolved')).toHaveCount(0);
});

test('the strip header opens the legend from the keyboard and returns focus to it', async ({
  page,
}) => {
  await open(page, { width: 1120, height: 720, scheme: 'dark' });
  const columns = panel(page).getByTestId('environment-columns');
  const legend = panel(page).getByRole('button', {
    name: '14 days · activity strips, fixed 14 local days',
  });
  await expect(legend).toHaveText('14 days');
  await expect(columns.locator(':scope > *')).toHaveText([
    'Tool · kind · host',
    '14 days',
    /^Calls/,
  ]);
  const list = panel(page).getByRole('list', { name: 'Most-called identities, top 8 of 11' });
  await expect(list).toHaveAttribute('aria-describedby', (await columns.getAttribute('id'))!);
  // The footer keeps only the dialog controls; the legend's line is gone.
  await expect(panel(page).locator('.xt-env-actions')).toHaveText('Observed · 11Configured · 14');
  const before = (await panel(page).boundingBox())!;
  await legend.focus();
  await expect(legend).toBeFocused();
  // The ring is drawn inside the header's 16px box, so the header's gutter clips none of it.
  expect(
    await legend.evaluate((node) => {
      const style = getComputedStyle(node);
      return { outline: style.outlineStyle, offset: style.outlineOffset };
    }),
  ).toEqual({ outline: 'solid', offset: '-2px' });
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Activity strips' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId('environment-strip-note')).toContainText('fixed 14 local days');
  await expect(dialog).toContainText('installed components are unknown');
  await expect(dialog.locator('.xt-env-legend-swatch')).toHaveCount(3);
  // Base UI moves focus to the dialog's first control on a later frame than it
  // becomes visible, so wait for focus to land exactly on Close.
  await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
  // Resizing while it is open keeps the dialog in the window.
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(dialog).toBeVisible();
  const box = (await dialog.boundingBox())!;
  expect(box.x + box.width).toBeLessThanOrEqual(1440);
  expect(box.y + box.height).toBeLessThanOrEqual(900);
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(legend).toBeFocused();
  expect(await panel(page).boundingBox()).toEqual(before);
  // Tab moves on to the list, which scrolls from the keyboard, then to the dialog controls.
  await page.keyboard.press('Tab');
  await expect(list).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(panel(page).getByRole('button', { name: 'Observed identities · 11' })).toBeFocused();
});

test("the column header keeps the rows' tracks beside a classic scrollbar", async ({
  page,
  browserName,
}) => {
  await open(page, { width: 1120, height: 720 });
  // A styled scrollbar is a classic one that takes room; the header reserves the same gutter.
  // Headless WebKit keeps its overlay scrollbar, so there the gutter is 0 on both.
  await page.addStyleTag({
    content: '.xt-env-list::-webkit-scrollbar, .xt-env-head::-webkit-scrollbar { width: 15px; }',
  });
  const columns = await page.evaluate(() => {
    const list = document.querySelector<HTMLElement>('.xt-env-list[data-compact]')!;
    const head = document.querySelector<HTMLElement>('.xt-env-head')!;
    const rows = [...list.querySelectorAll<HTMLElement>(':scope > li')];
    const edges = (cells: Element[]) =>
      cells.map((cell) => {
        const box = cell.getBoundingClientRect();
        return [Math.round(box.left * 100) / 100, Math.round(box.right * 100) / 100];
      });
    return {
      head: {
        top: head.getBoundingClientRect().top,
        bottom: head.getBoundingClientRect().bottom,
        height: head.getBoundingClientRect().height,
        text: [...head.children].map((cell) => cell.textContent),
        cells: edges([...head.children]),
        insideList: list.contains(head),
      },
      listTop: list.getBoundingClientRect().top,
      rowRight: rows.map((item) => Math.round(item.getBoundingClientRect().right * 100) / 100),
      rowCells: rows.map((item) => edges([...item.children])),
      rowOrder: rows.map((item) => [...item.children].map((cell) => cell.className.split(' ')[0])),
      gutter: [list.offsetWidth - list.clientWidth, head.offsetWidth - head.clientWidth],
    };
  });
  if (browserName === 'chromium') expect(columns.gutter).toEqual([15, 15]);
  expectColumns(columns, '7d');
});

test('the bounded observed list scrolls with the keyboard and keeps all eight rows', async ({
  page,
}) => {
  // At the minimum window the list shows its three-row minimum, so the rest scroll.
  await open(page, { width: 1120, height: 720 });
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
  // A triangle at the header's top right, named with the count; no line in the body.
  const trigger = panel(page).getByRole('button', {
    name: 'Unresolved attribution: 38 unresolved observations, including untimed',
  });
  await expect(trigger).not.toHaveAccessibleName(/ of |104/);
  await expect(panel(page).getByRole('note')).toHaveCount(0);
  await expect(panel(page).locator('.xt-section-panel')).not.toContainText('unresolved');
  const header = (await panel(page).locator('.xt-dash-card-header').boundingBox())!;
  const flag = (await trigger.boundingBox())!;
  expect(flag.y + flag.height).toBeLessThanOrEqual(header.y + header.height);
  expect(header.x + header.width - (flag.x + flag.width)).toBeLessThanOrEqual(20);
  // Hover reads the counts, the untimed ones apart, and leaves the page as it was.
  await trigger.hover();
  const gist = page.getByRole('tooltip');
  await expect(gist).toContainText('In the last 7d: claude');
  await expect(gist).toContainText('Untimed, in no selected range: codex · 7 untimed observations');
  const tip = (await gist.boundingBox())!;
  expect(tip.x).toBeGreaterThanOrEqual(0);
  expect(tip.x + tip.width).toBeLessThanOrEqual(1120);
  await page.mouse.move(0, 0);
  await expect(gist).toHaveCount(0);
  await trigger.focus();
  await expect(page.getByRole('tooltip')).toContainText('38 unresolved observations');
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
  const dialog = page.getByRole('dialog', { name: 'Observed identities · last 7d' });
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
  for (const [days, calls] of [
    [14, '84 calls'],
    [30, '168 calls'],
    [7, '42 calls'],
  ] as const) {
    await range.getByRole('radio', { name: `${days}d` }).click();
    await expect(bash.locator('.xt-env-calls')).toHaveText(calls);
    await expect(bash.getByRole('img')).toHaveCount(14);
    await expect(first).toHaveAttribute('aria-label', 'Aug 25: 0');
    // The calls header follows the range; the strip header keeps its fixed scope.
    await expect(panel(page).getByTestId('environment-columns')).toHaveText(
      `Tool · kind · host14 daysCalls · last ${days}d`,
    );
    await expect(page.getByRole('region', { name: 'Account usage' })).toContainText(
      'Usage source unavailable',
    );
  }
  await expect(
    bash
      .getByRole('img')
      .evaluateAll((cells) => cells.slice(0, 4).map((cell) => cell.getAttribute('data-level'))),
  ).resolves.toEqual(['0', '1', '2', '3']);
});
