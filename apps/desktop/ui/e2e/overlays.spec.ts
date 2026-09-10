import { expect, test } from '@playwright/test';
import contract from '../../../../design/token-contract.json' with { type: 'json' };
import { writeFile } from 'node:fs/promises';

test.beforeEach(async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
});

test('appearance follows emulated system, persists overrides, and renders both themes', async ({
  page,
}, info) => {
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(22, 21, 25)');
  const tokenValues = async () =>
    page.locator('html').evaluate(
      (element, keys) =>
        Object.fromEntries(
          keys.map((key) => [
            key,
            getComputedStyle(element)
              .getPropertyValue('--' + key)
              .replace(/\s+/g, ''),
          ]),
        ),
      Object.keys(contract.dark),
    );
  const normalized = (values: Record<string, string>) =>
    Object.fromEntries(
      Object.entries(values).map(([key, value]) => [key, value.replace(/\s+/g, '')]),
    );
  expect(await tokenValues()).toEqual(normalized(contract.dark));
  await page.screenshot({ path: info.outputPath('dark.png') });
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByLabel('Appearance').selectOption('dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByLabel('Appearance').selectOption('light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(241, 241, 245)');
  expect(await tokenValues()).toEqual(normalized(contract.light));
  await page.screenshot({ path: info.outputPath('light.png') });
  await page.getByLabel('Appearance').selectOption('system');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});

test('local fonts load with external network denied and remain usable offline', async ({
  page,
  context,
  baseURL,
}, info) => {
  const localOrigin = new URL(baseURL!).origin;
  const fontRequests: string[] = [];
  const external: string[] = [];
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== localOrigin) {
      external.push(url.origin);
      await route.abort();
    } else {
      if (url.pathname.startsWith('/fonts/')) fontRequests.push(url.pathname);
      await route.continue();
    }
  });
  await page.goto('/');
  const loaded = await page.evaluate(async () => {
    const results = [];
    for (const [family, weights] of [
      ['Manrope', [400, 500, 600, 700, 800]],
      ['Geist Mono', [400, 500, 600]],
    ] as const) {
      for (const weight of weights)
        results.push(
          (await document.fonts.load(`${weight} 16px "${family}"`, 'XTrace 0123456789')).length,
        );
    }
    return results;
  });
  expect(loaded).toEqual(Array(8).fill(1));
  expect(new Set(fontRequests).size).toBe(8);
  expect(external).toEqual([]);
  await context.setOffline(true);
  await page.getByLabel('Appearance').selectOption('light');
  expect(
    await page.evaluate(
      () =>
        document.fonts.check('800 16px Manrope') && document.fonts.check('600 16px "Geist Mono"'),
    ),
  ).toBe(true);
  await writeFile(
    info.outputPath('offline-font-network.json'),
    JSON.stringify({ fontRequests, external, offlineRender: true }, null, 2) + '\n',
  );
  await info.attach('offline-font-network.json', {
    body: JSON.stringify({ fontRequests, external, offlineRender: true }, null, 2),
    contentType: 'application/json',
  });
});

test('denied storage does not break an appearance change', async ({ page }) => {
  await page.addInitScript(() => {
    for (const method of ['getItem', 'setItem'])
      Object.defineProperty(Storage.prototype, method, {
        value: () => {
          throw new DOMException('Denied', 'SecurityError');
        },
      });
  });
  await page.goto('/');
  await page.getByLabel('Appearance').selectOption('light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

test('popover keeps subtree tokens and fixed offsets, light-dismisses and restores keyboard focus', async ({
  page,
}, info) => {
  await page.goto('/e2e/overlays.html');
  const trigger = page.getByRole('button', { name: 'Open popover' });
  const popover = page.locator('#details');
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(popover).toBeVisible();
  await expect(popover).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await expect(page.getByTestId('utility-probe')).toHaveCSS(
    'background-color',
    'rgb(255, 255, 255)',
  );
  await expect(page.getByTestId('utility-probe')).toHaveCSS('color', 'rgb(26, 26, 26)');
  await expect(page.getByTestId('utility-probe')).toHaveCSS(
    'font-family',
    /Geist Mono.*Menlo.*monospace/,
  );
  const anchor = await trigger.boundingBox();
  const box = await popover.boundingBox();
  expect(box!.y).toBeCloseTo(anchor!.y + anchor!.height + 14, 0);
  await page.screenshot({ path: info.outputPath('popover-light-subtree.png') });
  await page.evaluate(() => window.scrollTo(0, 60));
  await expect
    .poll(async () => {
      const a = await trigger.boundingBox();
      const b = await popover.boundingBox();
      return Math.round(b!.y - a!.y - a!.height);
    })
    .toBe(14);
  await expect(page.getByRole('button', { name: 'Close popover' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(popover).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.getByTestId('outside').click();
  await expect(popover).toBeHidden();
  await page.getByRole('button', { name: 'Toggle page theme' }).click();
  await expect(page.getByTestId('dark-scope')).toHaveCSS('background-color', 'rgb(35, 34, 39)');
  await expect(page.getByTestId('dark-scope')).toHaveCSS('color', 'rgb(242, 241, 245)');
  for (let repeat = 0; repeat < 3; repeat++) {
    await trigger.click();
    await expect(popover).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(popover).toBeHidden();
  }
});

test('modal contains Tab, blocks background interaction, preserves theme and returns focus', async ({
  page,
}, info) => {
  await page.goto('/e2e/overlays.html');
  const trigger = page.getByRole('button', { name: 'Open modal' });
  const modal = page.getByRole('dialog', { name: 'Modal details' });
  await trigger.click();
  await expect(modal).toBeVisible();
  await expect(modal).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  const input = modal.getByRole('textbox', { name: 'Name' });
  const close = modal.getByRole('button', { name: 'Close modal' });
  await input.focus();
  for (let repeat = 0; repeat < 3; repeat++) {
    await page.keyboard.press('Tab');
    await expect(close).toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(input).toBeFocused();
  }
  await page.keyboard.press('Shift+Tab');
  await expect(close).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(input).toBeFocused();
  // The library hides background controls from assistive technology and blocks
  // pointer input with a backdrop. Programmatic .focus() is not a user action.
  await expect(page.getByRole('button', { name: 'Open modal' })).toHaveCount(0);
  const background = await page
    .getByRole('button', { name: 'Toggle page theme', includeHidden: true })
    .boundingBox();
  await page.mouse.click(background!.x + 4, background!.y + 4);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(modal).toBeVisible();
  await input.focus();
  await page.screenshot({ path: info.outputPath('modal-light-subtree.png') });
  await page.keyboard.press('Escape');
  await expect(modal).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await close.click();
  await expect(modal).toBeHidden();
  await expect(trigger).toBeFocused();
  await page.getByRole('button', { name: 'Open popover' }).focus();
  await page.keyboard.press('Tab');
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(modal).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(modal).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await expect(modal).toBeVisible();
  // A route/owner update may remove an open overlay independently of its state.
  await page
    .getByRole('button', { name: 'Mount overlays', includeHidden: true })
    .evaluate((element) => (element as HTMLButtonElement).click());
  await expect(modal).toBeHidden();
  await expect(trigger).toBeFocused();
});

for (const target of ['trigger', 'position']) {
  test(`popover tracks replacement ${target}, escapes clipping, and updates its scoped theme`, async ({
    page,
  }) => {
    await page.goto(`/e2e/overlays.html?anchors&${target}`);
    const trigger = page.getByRole('button', { name: 'Moving details', exact: true });
    const popup = page.getByRole('dialog', { name: 'Moving details', exact: true });
    const anchor = target === 'position' ? page.getByTestId('position-anchor') : trigger;
    await trigger.click();
    await expect(popup).toBeVisible();
    await expect(popup).toHaveCSS('background-color', 'rgb(255, 255, 255)');
    for (let i = 0; i < 2; i++) {
      const oldX = (await anchor.boundingBox())!.x;
      await popup.getByRole('button', { name: 'Replace anchor' }).click();
      await expect.poll(async () => (await anchor.boundingBox())!.x).toBeGreaterThan(oldX);
      await expect
        .poll(async () =>
          Math.abs((await popup.boundingBox())!.x - (await anchor.boundingBox())!.x),
        )
        .toBeLessThan(1);
      // Clickable content beyond the clipped, transformed parent proves portal hit testing.
      await popup.getByRole('button', { name: 'Change scope theme' }).click();
      await expect(popup).toHaveCSS(
        'background-color',
        i === 0 ? 'rgb(35, 34, 39)' : 'rgb(255, 255, 255)',
      );
    }
    await page.keyboard.press('Escape');
    await expect(popup).toBeHidden();
    await expect(trigger).toBeFocused();
    await trigger.click();
    await page.getByRole('button', { name: 'Outside state change' }).click();
    await expect(popup).toBeHidden();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await trigger.click();
    await expect(popup).toBeVisible();
  });
}

test('nested modal dismisses only the child and restores focus through both layers', async ({
  page,
}) => {
  await page.goto('/e2e/overlays.html?nested');
  const parentTrigger = page.getByRole('button', { name: 'Open parent' });
  await parentTrigger.click();
  const childTrigger = page.getByRole('button', { name: 'Open child' });
  await childTrigger.click();
  const child = page.getByRole('dialog', { name: 'Child dialog' });
  await expect(child).toBeVisible();
  await expect(child).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await page.keyboard.press('Escape');
  await expect(child).toBeHidden();
  await expect(page.getByRole('dialog', { name: 'Parent dialog' })).toBeVisible();
  await expect(childTrigger).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Parent dialog' })).toBeHidden();
  await expect(parentTrigger).toBeFocused();
});
