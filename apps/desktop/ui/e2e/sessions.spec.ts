import { test, expect } from '@playwright/test';
for (const colorScheme of ['light', 'dark'] as const) {
  test(`indexed Sessions metadata, filters and coverage in ${colorScheme}`, async ({
    page,
  }, info) => {
    await page.emulateMedia({ colorScheme });
    await page.goto('/sessions');
    await expect(page.getByRole('heading', { name: 'Sessions', exact: true })).toBeVisible();
    await expect(page.getByRole('table', { name: 'Indexed sessions' })).toBeVisible();
    await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
    await expect(page.getByText('fixture-model-v1', { exact: true })).toBeVisible();
    await page.screenshot({ path: info.outputPath(`sessions-${colorScheme}.png`) });
    await page.getByLabel('Search sessions').fill('missing');
    await expect(page.getByText('No sessions match these filters.')).toBeVisible();
    await page.getByLabel('Search sessions').fill('');
    await page.getByLabel('Filter by host').selectOption('codex');
    await expect(page.getByText('No sessions match these filters.')).toBeVisible();
    await page.getByLabel('Filter by host').selectOption('claude');
    await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
  });
}
