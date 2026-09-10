import { expect, test } from '@playwright/test';
test.use({ viewport: { width: 1120, height: 900 } });
for (const theme of ['dark', 'light'] as const) {
  test(`table containment, keyboard, host filters and charts in ${theme}`, async ({
    page,
  }, info) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/e2e/table-overflow.html');
    await page.evaluate(() => document.fonts.ready);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    const table = page.getByRole('table', { name: 'Pull requests', exact: true });
    const scroll = page.getByRole('region', { name: 'Pull requests scroll area' });
    expect(await scroll.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);
    const head = table.getByRole('columnheader', { name: 'Tokens' });
    const cell = table.getByRole('row').nth(1).getByRole('cell').nth(2);
    expect((await head.boundingBox())!.x).toBeCloseTo((await cell.boundingBox())!.x, 1);
    await scroll.evaluate((el) => {
      el.scrollLeft = 200;
    });
    expect(await scroll.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
    expect((await head.boundingBox())!.x).toBeCloseTo((await cell.boundingBox())!.x, 1);
    await scroll.evaluate((el) => {
      el.scrollLeft = 0;
    });
    await expect(table.locator('.xt-data-row').first()).toContainText('Improve activity layout');
    await head.getByRole('button').click();
    await expect(head).toHaveAttribute('aria-sort', 'ascending');
    await expect(table.locator('.xt-data-row').first()).toContainText('Add session filters');
    await expect(table.locator('.xt-data-row').last()).toContainText('Handle unknown measurements');
    await table.getByText('Select row', { exact: true }).first().click();
    await expect(table.getByRole('checkbox').first()).toBeChecked();
    await expect(page.getByRole('status', { name: 'Clicked row' })).toHaveText('none');
    const flexible = page.getByTestId('flexible-table');
    expect(await flexible.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    const longTitle = flexible.locator('.xt-title-cell > span');
    expect(await longTitle.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);
    const first = table.getByRole('row').nth(1);
    await first.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('status', { name: 'Clicked row' })).toHaveText('sample-1');
    const toggle = table.getByRole('button', { name: 'Collapse sample-1' });
    await toggle.click();
    await expect(table.getByText('Synthetic details for sample-1')).toBeHidden();
    await table.getByRole('button', { name: 'Expand sample-1' }).click();
    await expect(table.getByText('Synthetic details for sample-1')).toBeVisible();
    for (const height of [22, 24, 32])
      await expect(
        page
          .getByRole('table', { name: `Rows ${height}` })
          .getByRole('row')
          .nth(1),
      ).toHaveCSS('height', `${height}px`);
    await expect(
      page.getByRole('table', { name: 'Rules', exact: true }).getByRole('row').nth(1),
    ).toHaveCSS('height', '44px');
    await expect(first).toHaveCSS('height', '40px');
    await expect(page.getByRole('img', { name: 'Day 1, agent: 0' })).toHaveCSS('opacity', '0.25');
    await expect(page.getByRole('img', { name: 'Day 4, agent: unmeasured' })).toHaveCSS(
      'border-style',
      'dashed',
    );
    await expect(page.locator('.xt-legend [data-tone="accent"]')).toHaveCSS(
      'background-color',
      await page
        .getByRole('img', { name: 'Day 2, agent: 1' })
        .evaluate((el) => getComputedStyle(el).backgroundColor),
    );
    const filter = page.getByRole('button', { name: 'Filter hosts' });
    await filter.focus();
    await page.keyboard.press('Enter');
    const claude = page.getByRole('checkbox', { name: 'Claude 12' });
    await expect(claude).toBeFocused();
    await page.keyboard.press('Space');
    await expect(claude).not.toBeChecked();
    await expect(page.getByRole('status', { name: 'Selected hosts' })).toHaveText('codex');
    await page.keyboard.press('Tab');
    await expect(page.getByRole('checkbox', { name: 'Codex 0' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(claude).toBeHidden();
    await expect(filter).toBeFocused();
    await filter.click();
    await expect(claude).toBeVisible();
    await page.screenshot({ path: info.outputPath(`filter-${theme}.png`), fullPage: true });
    await page.getByRole('button', { name: 'Outside control' }).click();
    await expect(claude).toBeHidden();
    await scroll.evaluate((el) => {
      el.scrollLeft = 0;
      el.scrollTop = 0;
    });
    await page.screenshot({ path: info.outputPath(`table-${theme}.png`), fullPage: true });
    const headerY = (await head.boundingBox())!.y;
    await scroll.evaluate((el) => {
      el.scrollTop = 100;
    });
    expect(await scroll.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
    expect((await head.boundingBox())!.y).toBeCloseTo(headerY, 1);
    await page.getByRole('button', { name: 'Detect again' }).click();
    await expect(page.getByRole('status', { name: 'Clicked row' })).toHaveText('detect');
    expect(errors).toEqual([]);
  });
}

test('narrow preview keeps wide tables contained and filter uses the live theme', async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 780 });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/e2e/table-overflow.html');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('button', { name: 'Switch to dark theme' }).click();
  await page.getByRole('button', { name: 'Filter hosts' }).click();
  const popup = page.getByRole('dialog', { name: 'Filter hosts' });
  await expect(popup.locator('xpath=ancestor::*[@data-theme][1]')).toHaveAttribute(
    'data-theme',
    'dark',
  );
  const box = (await popup.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(360);
  const cursor = popup.getByRole('checkbox', { name: 'Cursor —' });
  await cursor.check();
  await expect(page.getByRole('status', { name: 'Selected hosts' })).toHaveText(
    'claude, codex, cursor',
  );
  await page.keyboard.press('Escape');
  await expect(popup).toBeHidden();
  await page.getByRole('button', { name: 'Switch to light theme' }).click();
  await page.getByRole('button', { name: 'Filter hosts' }).click();
  await expect(
    page.getByRole('dialog').locator('xpath=ancestor::*[@data-theme][1]'),
  ).toHaveAttribute('data-theme', 'light');
});
