import { expect, test } from '@playwright/test';
import { readdir, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

test('production build ignores the fixture flag and contains no fixture export or adapter chunk', async ({
  page,
}) => {
  const assets = resolve(import.meta.dirname, '../dist/assets');
  const names = await readdir(assets);
  expect(names.some((name) => /F1|FixtureDataSource/.test(name))).toBe(false);
  const javascript = (
    await Promise.all(
      names
        .filter((name) => name.endsWith('.js'))
        .map((name) => readFile(resolve(assets, name), 'utf8')),
    )
  ).join('\n');
  expect(javascript).not.toContain('fixture://F1');
  expect(javascript).not.toContain('Only F1 is implemented');
  expect(javascript).not.toContain('fixtures/F1.json');
  expect(javascript).toContain('app_info');
  expect(javascript).toContain('db_counts');
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.name));
  await page.goto('/settings?fixture=F1');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Settings');
  await expect(
    page.getByText('Browser preview · Open the desktop app to read local data.', { exact: true }),
  ).toBeVisible();
  await expect(page.locator('.xt-fixture-badge')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Local database' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});
