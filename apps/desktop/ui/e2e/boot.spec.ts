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
  await expect(page.getByRole('heading', { name: 'Welcome to XTrace.' })).toBeVisible();
  const brand = page.getByRole('img', { name: 'XTrace brand mark' });
  await expect(brand).toBeVisible();
  await expect(brand).toHaveJSProperty('naturalWidth', 436);
  await expect(page.getByRole('status')).toHaveText(
    'Browser preview · Native app information is available in the desktop app.',
  );
  await page.evaluate(() => document.fonts.ready);
  expect(externalRequests).toEqual([]);
  expect(errors).toEqual([]);
});
