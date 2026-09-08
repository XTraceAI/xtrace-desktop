import { expect, test } from '@playwright/test';

test('sidebar geometry, controlled state, theme and native Hub dismissal', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/e2e/sidebar.html');
  await page.evaluate(() => document.fonts.ready);
  const sidebar = page.getByRole('complementary', { name: 'Workspace' });
  await expect(sidebar).toHaveCSS('width', '228px');
  await expect(sidebar).toHaveCSS('height', '900px');
  await expect(sidebar).toHaveCSS('padding-top', '16px');
  const active = page.getByRole('button', { name: 'Dashboard' });
  await expect(active).toHaveCSS('height', '32px');
  await expect(active).toHaveCSS('color', 'rgb(207, 201, 255)');
  await page
    .getByTestId('sidebar-preview')
    .screenshot({ path: info.outputPath('sidebar-dark.png') });
  await page.getByRole('button', { name: 'Sessions' }).click();
  await expect(page.getByRole('button', { name: 'Sessions' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.getByRole('button', { name: 'Switch to light appearance' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page
    .getByTestId('sidebar-preview')
    .screenshot({ path: info.outputPath('sidebar-light.png') });
  const trigger = page.getByRole('button', { name: 'XTrace Hub', exact: true });
  const hub = page.getByRole('dialog', { name: 'Connect this desktop to your team.' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(hub).toBeVisible();
  await expect(hub).toHaveCSS('width', '264px');
  const sidebarBounds = (await sidebar.boundingBox())!;
  const hubBounds = (await hub.boundingBox())!;
  const triggerBounds = (await trigger.boundingBox())!;
  expect(hubBounds.x).toBe(sidebarBounds.x + sidebarBounds.width + 14);
  expect(hubBounds.y + hubBounds.height).toBeCloseTo(triggerBounds.y + triggerBounds.height + 6, 0);
  await expect(hub).toHaveCSS('background-color', 'rgb(241, 241, 245)');
  await expect(page.getByRole('button', { name: 'Connect XTrace Hub' })).toBeDisabled();
  await page.screenshot({ path: info.outputPath('sidebar-hub.png') });
  await page.keyboard.press('Escape');
  await expect(hub).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.getByRole('button', { name: 'Close XTrace Hub' }).click();
  await expect(hub).toBeHidden();
  await trigger.click();
  await page.getByTestId('outside').click();
  await expect(hub).toBeHidden();
  await page.getByRole('button', { name: 'Reserve native controls' }).click();
  await expect(sidebar).toHaveCSS('padding-top', '74px');
  await page.screenshot({ path: info.outputPath('brand-sizes-native-inset.png') });
  expect(errors).toEqual([]);
});
