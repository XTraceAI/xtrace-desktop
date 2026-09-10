import { expect, test } from '@playwright/test';
import { readdir, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

test('production excludes the gallery route, frame entry and illustrative data despite enabled flags', async ({
  page,
}) => {
  const dist = resolve(import.meta.dirname, '../dist');
  const files = await readdir(dist, { recursive: true });
  expect(files.some((path) => /gallery|stories|sample/i.test(path))).toBe(false);
  const code = (
    await Promise.all(
      files
        .filter((path) => /\.(js|json|html|css)$/.test(path))
        .map((path) => readFile(resolve(dist, path), 'utf8')),
    )
  ).join('\n');
  for (const marker of [
    '/gallery',
    '[SAMPLE] Illustrative component data',
    'Illustrative task A',
    'gallery-frame',
    'fixture://F1',
  ])
    expect(code).not.toContain(marker);
  await page.goto('/gallery?story=sidebar%2Factive-dashboard');
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Component gallery' })).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});
