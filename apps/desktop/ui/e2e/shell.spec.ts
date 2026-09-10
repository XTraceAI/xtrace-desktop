import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

test.use({ viewport: { width: 1120, height: 720 } });

const assertFits = async (page: Page) => {
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1120);
  const sidebar = await page.locator('.xt-sidebar').boundingBox();
  expect(sidebar?.width).toBe(228);
  expect(sidebar?.height).toBe(720);
  expect(await page.locator('.xt-window-chrome').count()).toBe(0);
};

test('navigates real shell routes with canonical F1 counts, shortcuts and both themes', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.name));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push('console error');
  });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/');
  await expect(page).toHaveURL(/\/dashboard$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Dashboard');
  await expect(
    page.getByRole('status', { name: '' }).filter({ hasText: 'fixture F1' }),
  ).toBeVisible();
  await expect(page.locator('.xt-brand-mark img')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Dashboard', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('shell-dark.png') });
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Sessions');
  await page.getByRole('button', { name: 'Pull requests', exact: true }).click();
  await expect(page.getByRole('navigation', { name: 'Breadcrumb' })).toContainText('pull-requests');
  await expect(page.getByRole('button', { name: 'Pull requests', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.keyboard.press('Meta+,');
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Settings');
  await expect(page.getByText(fixture.app_info.data_dir, { exact: true })).toBeVisible();
  const values = await page.locator('.xt-database-summary dd').allTextContents();
  expect(values).toEqual([
    fixture.app_info.data_dir,
    String(fixture.app_info.schema_version),
    String(fixture.db_counts.sessions),
    String(fixture.db_counts.records),
    String(fixture.db_counts.usage),
  ]);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(page.locator('aside [aria-current="page"]')).toHaveCount(0);
  await page.getByRole('combobox', { name: 'Appearance' }).selectOption('light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Dashboard', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Dashboard');
  await page.mouse.move(600, 300);
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('shell-light.png') });
  expect(errors).toEqual([]);
});

test('reloads every placeholder route and preserves canonical PR query context', async ({
  page,
}) => {
  for (const [path, title] of [
    ['/first-launch', 'Welcome to XTrace'],
    ['/dashboard', 'Dashboard'],
    ['/sessions', 'Sessions'],
    ['/prs', 'Pull requests'],
    ['/rulebook', 'Rulebook'],
    ['/rulebook/sample-rule', 'Rule detail'],
    ['/rulebook/fires', 'Rule fires'],
    ['/settings', 'Settings'],
    ['/leaderboard', 'Leaderboard'],
    ['/missing', 'Page not found'],
  ]) {
    await page.goto(path);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(title);
    await expect(page.getByText('fixture F1', { exact: true })).toBeVisible();
    await assertFits(page);
  }
  await page.goto('/sessions?pr=https%3A%2F%2Fgithub.com%2Fexample%2Fproject%2Fpull%2F1');
  await expect(
    page.getByText('Pull request filter: https://github.com/example/project/pull/1', {
      exact: true,
    }),
  ).toBeVisible();
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Sessions');
  await expect(
    page.getByText('Pull request filter: https://github.com/example/project/pull/1', {
      exact: true,
    }),
  ).toBeVisible();
});

test('renders bundled fonts and remains navigable with external network denied', async ({
  page,
  context,
}) => {
  const external: string[] = [];
  const fonts = new Set<string>();
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (!['127.0.0.1', 'localhost'].includes(url.hostname)) {
      external.push(url.origin);
      await route.abort();
    } else {
      if (url.pathname.startsWith('/fonts/')) fonts.add(url.pathname);
      await route.continue();
    }
  });
  await page.goto('/dashboard');
  await expect(page.getByText('fixture F1', { exact: true })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  expect(fonts.size).toBeGreaterThan(0);
  expect(external).toEqual([]);
  await context.setOffline(true);
  await page.getByRole('button', { name: 'Rulebook', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rulebook');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByText('fixture://F1', { exact: true })).toBeVisible();
});
