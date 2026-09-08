import { expect, test } from '@playwright/test';

test('keyboard reaches native controls, skips disabled states and preserves visible numeric contracts', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/e2e/controls.html');
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
    await page.goto(`/e2e/controls.html?theme=${theme}`);
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
    if (process.env.UPDATE_EVIDENCE === '1')
      await card.screenshot({ path: `../../../docs/acceptance/FND-07b/controls-${theme}.png` });
  }
  expect(errors).toEqual([]);
});
