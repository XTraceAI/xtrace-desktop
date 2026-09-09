import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 1440, height: 900 } });

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
  await expect(sidebar.locator('.xt-brand-mark img')).toHaveCSS('width', '22px');
  await expect(sidebar.locator('.xt-nav-group h2').first()).toHaveCSS('line-height', '13.775px');
  await expect(sidebar.locator('.xt-host-glyph img')).toHaveCount(3);
  const cursorLogo = sidebar.locator('.xt-host-cursor .xt-host-glyph img');
  await expect(cursorLogo).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await expect(sidebar.getByLabel('Claude Code tokens: 5500000')).toHaveText('5.5M');
  await expect(sidebar.getByLabel('Codex tokens: 400000')).toHaveText('0.4M');
  for (const logo of await sidebar.locator('img').all())
    await expect(logo).toHaveJSProperty('complete', true);
  expect(
    await sidebar
      .locator('img')
      .evaluateAll((es) => es.every((e) => (e as HTMLImageElement).naturalWidth > 0)),
  ).toBe(true);
  const footer = sidebar.locator('.xt-sidebar-status');
  const settings = sidebar.getByRole('button', { name: 'Settings' });
  const footerBox = (await footer.boundingBox())!;
  const settingsBox = (await settings.boundingBox())!;
  expect(settingsBox.y).toBeGreaterThanOrEqual(footerBox.y);
  expect(settingsBox.y + settingsBox.height).toBeLessThanOrEqual(footerBox.y + footerBox.height);
  expect(settingsBox.x + settingsBox.width).toBeLessThanOrEqual(228 - 12);
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
  await expect(cursorLogo).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await active.click();
  await page.mouse.move(600, 500);
  await page
    .getByTestId('sidebar-preview')
    .screenshot({ path: info.outputPath('sidebar-light.png') });
  const trigger = page.getByRole('button', { name: 'XTrace Hub', exact: true });
  await expect(trigger).toHaveText('Cloud and Team');
  await expect(trigger).toHaveCSS('width', '204px');
  const hub = page.getByRole('dialog', { name: 'Connect this desktop to your team.' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(hub).toBeVisible();
  await expect(hub).toHaveCSS('width', '264px');
  const sidebarBounds = (await sidebar.boundingBox())!;
  const hubBounds = (await hub.boundingBox())!;
  const footerBounds = (await footer.boundingBox())!;
  expect(hubBounds.x).toBe(sidebarBounds.x + sidebarBounds.width - 12 + 14);
  expect(hubBounds.y + hubBounds.height).toBeCloseTo(footerBounds.y + footerBounds.height + 6, 0);
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
  await trigger.click();
  await page.getByRole('button', { name: 'Sessions' }).click();
  await expect(hub).toBeHidden();
  await trigger.click();
  await page.getByRole('button', { name: 'Switch to dark appearance' }).click();
  await expect(hub).toBeHidden();
  await page.getByRole('button', { name: 'Switch to light appearance' }).click();
  await active.click();
  const capture = page.getByRole('button', { name: 'Capture status', exact: true });
  const coverage = page.getByRole('dialog', { name: 'Capture by surface' });
  await expect(coverage).toBeHidden();
  await capture.focus();
  await page.keyboard.press('Enter');
  await expect(coverage).toBeVisible();
  await expect(coverage.getByText('Codex · cli').locator('..')).toContainText('Capturing');
  await expect(coverage.getByText('Codex · desktop').locator('..')).toContainText('Not capturing');
  await expect(coverage.getByText('Other host · new-surface').locator('..')).toContainText(
    'Unknown',
  );
  await page.keyboard.press('Escape');
  await expect(coverage).toBeHidden();
  await expect(capture).toBeFocused();
  await capture.click();
  await trigger.click();
  await expect(coverage).toBeHidden();
  await expect(hub).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Reserve native controls' }).click();
  await expect(sidebar).toHaveCSS('padding-top', '74px');
  await page.screenshot({ path: info.outputPath('brand-sizes-native-inset.png') });
  expect(errors).toEqual([]);
});
