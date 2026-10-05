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
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
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
  // The fixture export carries the disabled native index the app reports in fixture mode.
  const index = fixture.native_index;
  expect(index.phase.phase).toBe('disabled');
  expect(await page.locator('.xt-native-index-summary dd').allTextContents()).toEqual([
    `Disabled: ${index.phase.reason}`,
    'Not watching yet',
    `Unavailable: ${index.python.reason}`,
    `Unavailable: ${index.readers.reason}`,
  ]);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  await expect(page.locator('aside [aria-current="page"]')).toHaveCount(0);
  await page.getByRole('combobox', { name: 'Appearance' }).selectOption('light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Dashboard', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
  await page.mouse.move(600, 300);
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('shell-light.png') });
  expect(errors).toEqual([]);
});

test('reloads every route and preserves canonical PR query context', async ({ page }) => {
  for (const [path, title] of [
    ['/first-launch', 'Welcome to XTrace'],
    ['/dashboard', 'What your agents did'],
    ['/sessions', 'Sessions'],
    ['/prs', 'Pull requests'],
    ['/rulebook', 'Rulebook'],
    ['/rulebook/sample-rule', 'Rule detail'],
    ['/rulebook/fires', 'Recorded rule activity'],
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
  await expect(page.getByText(/Pull request filtering is not available yet/)).toBeVisible();
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Sessions');
  await expect(page.getByText(/Pull request filtering is not available yet/)).toBeVisible();
});

// The app keeps the kit's default Leaderboard row: nothing ranks in this local
// build, so the row is disabled, drawn in meta ink at 0.7 opacity with a "soon"
// badge and no hover fill, and never navigates by pointer or keyboard, while a
// typed or bookmarked /leaderboard still opens its named placeholder.
test('keeps Leaderboard unavailable in the sidebar while its address still opens', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.name));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push('console error');
  });
  const leaderboard = page.getByRole('button', { name: /Leaderboard/ });
  const rulebook = page.getByRole('button', { name: 'Rulebook', exact: true });
  const treatment = () =>
    page.evaluate(() => {
      const row = document.querySelector<HTMLButtonElement>('.xt-nav-item:disabled');
      const probe = document.createElement('span');
      probe.style.color = 'var(--meta)';
      document.body.append(probe);
      const meta = getComputedStyle(probe).color;
      probe.remove();
      const style = row && getComputedStyle(row);
      return style
        ? {
            color: style.color,
            opacity: style.opacity,
            cursor: style.cursor,
            fill: style.backgroundColor,
            meta,
          }
        : null;
    });
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/rulebook');
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rulebook');
    await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
    await expect(leaderboard).toBeDisabled();
    await expect(leaderboard.locator('.xt-soon')).toHaveText('soon');
    await expect(leaderboard).not.toHaveAttribute('aria-current', 'page');
    const rest = await treatment();
    expect(rest).toMatchObject({ opacity: '0.7', cursor: 'default' });
    expect(rest?.color).toBe(rest?.meta);
    await page.locator('.xt-sidebar').screenshot({
      path: info.outputPath(`sidebar-leaderboard-soon-${scheme}.png`),
    });
    // Hovering draws no fill, and a pointer press changes nothing.
    await leaderboard.hover({ force: true });
    expect(await treatment()).toMatchObject({ fill: rest?.fill, color: rest?.meta });
    await leaderboard.click({ force: true });
    await expect(page).toHaveURL(/\/rulebook$/);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rulebook');
    // Tab passes over the disabled row, then through Usage's forecast note,
    // its refresh and two account details before the Hub control.
    await rulebook.focus();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'How the forecast works' })).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'Refresh usage' })).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByLabel('Claude account usage: Usage unavailable')).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByLabel('Codex account usage: Usage unavailable')).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'XTrace Hub' })).toBeFocused();
    await expect(leaderboard).not.toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(leaderboard).not.toBeFocused();
    await expect(page).toHaveURL(/\/rulebook$/);
  }
  await page.goto('/leaderboard');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Leaderboard');
  await expect(page.getByText('This view is coming next.')).toBeVisible();
  await expect(leaderboard).toBeDisabled();
  await expect(leaderboard).toHaveAttribute('aria-current', 'page');
  expect(errors).toEqual([]);
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

// The ordinary development server explicitly leaves the gallery disabled.
test('gallery stays unavailable without its development flag', async ({ page }) => {
  await page.goto('/gallery');
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Component gallery' })).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});
