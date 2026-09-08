import { expect, test } from '@playwright/test';

test('app shell boots with local branding and an honest browser state', async ({ page }) => {
  const errors: string[] = [];
  const externalRequests: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (!['127.0.0.1', 'localhost'].includes(url.hostname)) externalRequests.push(url.origin);
  });
  await page.goto('/');
  await expect(page).toHaveURL(/\/dashboard$/);
  await expect(page.getByRole('heading', { name: 'Dashboard', exact: true })).toBeVisible();
  const brand = page
    .getByRole('complementary', { name: 'Workspace' })
    .locator('.xt-brand-mark img');
  await expect(brand).toBeVisible();
  await expect(brand).toHaveAttribute('src', '/mark.png');
  await expect(brand).toHaveJSProperty('naturalWidth', 436);
  await expect(
    page.getByText('Browser preview · Open the desktop app to read local data.', { exact: true }),
  ).toBeVisible();
  await expect(page.locator('.xt-fixture-badge')).toHaveCount(0);
  await page.evaluate(() => document.fonts.ready);
  expect(externalRequests).toEqual([]);
  expect(errors).toEqual([]);
});

test('gallery route is unavailable without its development flag', async ({ page }) => {
  await page.goto('/gallery');
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
  await expect(page.locator('iframe')).toHaveCount(0);
});
