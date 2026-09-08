import { expect, test } from '@playwright/test';

test('controlled TopBar actions, both themes, minimum width and host glyph geometry', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/e2e/topbar.html');
  await page.evaluate(() => document.fonts.ready);
  const topbar = page.getByTestId('design-topbar').locator('header');
  const minimum = page.getByTestId('minimum-topbar').locator('header');
  for (const theme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await expect(topbar).toHaveCSS('width', '1200px');
    await expect(topbar).toHaveCSS('height', '44px');
    await expect(topbar.getByRole('button', { name: '7d' })).toHaveCSS('height', '24px');
    await topbar.screenshot({ path: info.outputPath(`topbar-${theme}.png`) });
    const row = page.getByTestId('size-16');
    await expect(row.getByRole('img', { name: 'Claude', exact: true })).toHaveCSS(
      'background-color',
      'rgb(217, 119, 87)',
    );
    await expect(row.getByRole('img', { name: 'Codex', exact: true })).toHaveCSS(
      'background-color',
      'rgb(107, 107, 128)',
    );
    await expect(row.getByRole('img', { name: 'Cursor', exact: true })).toHaveCSS(
      'background-color',
      'rgb(13, 148, 136)',
    );
    await expect(row.getByRole('img', { name: 'Unknown host: new-host' })).toHaveCSS(
      'background-color',
      theme === 'dark' ? 'rgb(46, 45, 52)' : 'rgb(244, 244, 247)',
    );
    for (const size of [16, 18, 20, 30]) {
      const glyphs = page.getByTestId(`size-${size}`).getByRole('img');
      for (const glyph of await glyphs.all()) {
        await expect(glyph).toHaveCSS('width', `${size}px`);
        await expect(glyph).toHaveCSS('height', `${size}px`);
        await expect(glyph).toHaveCSS('font-weight', '600');
      }
    }
    await page
      .getByTestId('glyph-gallery')
      .screenshot({ path: info.outputPath(`glyphs-${theme}.png`) });
  }
  await topbar.getByRole('button', { name: '7d' }).focus();
  await page.keyboard.press('Tab');
  await expect(topbar.getByRole('button', { name: '14d' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(topbar.getByRole('button', { name: '14d' })).toHaveAttribute('aria-pressed', 'true');
  await topbar.getByRole('button', { name: '30d' }).click();
  await expect(topbar.getByRole('button', { name: '30d' })).toHaveAttribute('aria-pressed', 'true');
  await topbar.getByRole('button', { name: 'Share' }).click();
  await expect(page.getByRole('status')).toHaveText('share');
  await minimum.getByRole('button', { name: 'Scan' }).click();
  await expect(page.getByRole('status')).toHaveText('scan');
  await page.getByRole('button', { name: 'Copy rules' }).click();
  await expect(page.getByRole('status')).toHaveText('copy');
  await page.setViewportSize({ width: 1120, height: 720 });
  for (const theme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await expect(minimum).toHaveCSS('width', '892px');
    await expect(minimum).toHaveCSS('height', '44px');
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1120);
    const bounds = (await minimum.boundingBox())!;
    for (const button of await minimum.getByRole('button').all()) {
      const buttonBounds = (await button.boundingBox())!;
      expect(buttonBounds.x + buttonBounds.width).toBeLessThanOrEqual(bounds.x + bounds.width);
      expect(buttonBounds.y + buttonBounds.height).toBeLessThanOrEqual(bounds.y + bounds.height);
    }
    expect(
      await minimum
        .locator('.xt-topbar-crumb')
        .evaluate((element) => element.scrollWidth > element.clientWidth),
    ).toBe(true);
    await page
      .getByTestId('minimum-frame')
      .screenshot({ path: info.outputPath(`minimum-${theme}.png`) });
  }
  expect(errors).toEqual([]);
});
