import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 1120, height: 800 } });

test('keyboard reaches native controls, skips disabled states and preserves visible numeric contracts', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/e2e/controls.html');
  expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(
    true,
  );
  await page.getByRole('searchbox').focus();
  await page.keyboard.type('synthetic');
  await expect(page.getByRole('searchbox')).toHaveValue('synthetic');
  await page.keyboard.press('Tab');
  const toggle = page.getByRole('switch', { name: 'Capture' });
  await expect(toggle).toBeFocused();
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status', { name: 'Action count' })).toHaveText('Saved 1');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('radio', { name: 'Advise' })).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('radio', { name: 'Gate' })).toBeFocused();
  await expect(page.getByRole('radio', { name: 'Gate' })).toHaveAttribute('aria-checked', 'true');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'primary', exact: true })).toBeFocused();
  await expect(page.getByRole('progressbar', { name: 'Backtest' })).toHaveAttribute(
    'aria-valuetext',
    'Followed: 80%; Ignored: 20%',
  );
  for (const theme of ['dark', 'light']) {
    await page.emulateMedia({ colorScheme: theme as 'dark' | 'light' });
    await page.goto('/e2e/controls.html');
    await page.evaluate(() => document.fonts.ready);
    const card = page.getByRole('region', { name: 'Control samples' });
    await expect(page.getByRole('button', { name: 'primary', exact: true })).toHaveCSS(
      'height',
      '28px',
    );
    await expect(page.getByRole('button', { name: 'ghost', exact: true })).toHaveCSS(
      'height',
      '38px',
    );
    await expect(
      page.getByRole('switch', { name: 'Capture' }).locator('.xt-toggle-track > span'),
    ).toHaveCSS('background-color', 'rgb(255, 255, 255)');
    await expect(page.locator('[data-live="true"] .xt-state-dot')).toHaveCSS(
      'animation-name',
      'none',
    );
    expect(await card.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    await card.screenshot({ path: info.outputPath(`controls-${theme}.png`) });
  }
  expect(errors).toEqual([]);
});

test('held and unavailable radio selections remain keyboard usable', async ({ page }) => {
  await page.goto('/e2e/controls.html');
  await page.getByText('Keyboard edge cases', { exact: true }).click();
  const group = page.getByRole('radiogroup', { name: 'Held selection' });
  const first = group.getByRole('radio', { name: 'First', exact: true });
  const second = group.getByRole('radio', { name: 'Second' });
  await first.focus();
  await page.keyboard.press('ArrowRight');
  await expect(second).toBeFocused();
  await expect(first).toHaveAttribute('aria-checked', 'true');
  await expect(second).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('status', { name: 'Action count' })).toHaveText('Saved 1');
  await page.keyboard.press('ArrowRight');
  await expect(first).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(second).toBeFocused();
  await expect(page.getByRole('status', { name: 'Action count' })).toHaveText('Saved 2');
  await page.getByRole('button', { name: 'Disable First' }).click();
  await page.getByRole('button', { name: 'Remove First' }).focus();
  await page.keyboard.press('Tab');
  await expect(second).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'After groups' })).toBeFocused();
  await page.getByRole('button', { name: 'Remove First' }).click();
  await page.getByRole('button', { name: 'Restore First' }).focus();
  await page.keyboard.press('Tab');
  await expect(second).toBeFocused();
  await expect(group.getByRole('radio')).toHaveCount(2);
  await page.getByRole('button', { name: 'Restore First' }).click();
  await page.getByRole('button', { name: 'Enable First' }).click();
  await expect(first).toHaveAttribute('aria-checked', 'true');
  await page.getByRole('button', { name: 'Remove First' }).focus();
  await page.keyboard.press('Tab');
  await expect(first).toBeFocused();
});

test('preview adapts to a narrow pane and its theme switch works', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 850 });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/e2e/controls.html');
  await page.getByRole('button', { name: 'Switch to dark theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByRole('button', { name: 'Switch to light theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
