import { expect, test } from '@playwright/test';
import { rules } from '../src/kit/rules';

for (const theme of ['dark', 'light'] as const) {
  test(`metric geometry, unknown values, and native rule tooltips in ${theme}`, async ({
    page,
  }, info) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/e2e/metrics.html');
    await page.evaluate(() => document.fonts.ready);
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    const tiles = page.locator('.xt-stat-tile');
    await expect(tiles).toHaveCount(9);
    for (const tile of await tiles.all()) {
      const box = await tile.boundingBox();
      expect(box!.width).toBe(300);
      expect(box!.height).toBe(52);
      expect(await tile.locator('button').count()).toBe(0);
    }
    await expect(page.getByRole('button', { name: /Unmeasured tokens/ })).toContainText(
      'Unmeasured: Cursor Agent CLI usage is absent',
    );
    await expect(
      page.getByRole('button', { name: /Unmeasured tokens/ }).locator('.xt-stat-delta'),
    ).toHaveCount(0);
    await expect(
      page.getByRole('button', { name: /Peak sessions/ }).locator('.xt-metric-cell'),
    ).toHaveText('0');
    await expect(page.locator('.xt-section-header').first()).toHaveCSS('height', '40px');
    await expect(page.locator('.xt-section-header').last()).toHaveCSS('height', '36px');
    expect(
      await page
        .locator('.xt-stat-aside')
        .evaluate((element) => element.scrollWidth > element.clientWidth),
    ).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: info.outputPath(`metrics-${theme}.png`) });

    const input = page.getByRole('textbox', { name: 'Outside focus' });
    const trigger = page.getByTestId('short-rule').getByRole('button');
    const tooltip = page.getByRole('tooltip');
    await input.focus();
    await trigger.hover();
    await expect(tooltip).toHaveText(`M-06 · ${rules['M-06']}`);
    await expect(tooltip).toHaveCSS('pointer-events', 'none');
    await expect(input).toBeFocused();
    await page.mouse.move(10, 10);
    await expect(tooltip).toBeHidden();
    await expect(input).toBeFocused();
    await input.evaluate((element) => element.blur());
    await trigger.hover();
    await expect(tooltip).toBeVisible();
    await page.mouse.move(10, 10);
    await expect(tooltip).toBeHidden();
    expect(await page.evaluate(() => document.activeElement === document.body)).toBe(true);

    await input.focus();
    await page.keyboard.press('Tab');
    await expect(trigger).toBeFocused();
    await expect(tooltip).toBeVisible();
    await expect(trigger).toHaveAccessibleDescription(`M-06 · ${rules['M-06']}`);
    await expect(tooltip.locator('button, a, input, [tabindex]')).toHaveCount(0);
    await page.keyboard.press('Escape');
    await expect(tooltip).toBeHidden();
    await expect(trigger).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'Next control' })).toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(tooltip).toBeVisible();
    await page.keyboard.press('Tab');
    await expect(tooltip).toBeHidden();

    for (const [testId, ruleId] of [
      ['long-rule', 'M-04'],
      ['coverage-rule', 'C-08'],
    ] as const) {
      await page.getByTestId(testId).getByRole('button').focus();
      await expect(tooltip).toHaveText(`${ruleId} · ${rules[ruleId]}`);
      // offsetWidth/Height round the native layout dimensions to integer CSS pixels.
      const box = await tooltip.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(8);
      expect(box!.y).toBeGreaterThanOrEqual(8);
      expect(box!.x + box!.width).toBeLessThanOrEqual(1113);
      expect(box!.y + box!.height).toBeLessThanOrEqual(713);
      expect(
        await tooltip.evaluate((element) => element.scrollHeight <= element.clientHeight),
      ).toBe(true);
      if (ruleId === 'M-04')
        await page.screenshot({ path: info.outputPath(`definition-${theme}.png`) });
      await page.keyboard.press('Escape');
    }

    // Keep the trigger focused while the pointer clicks a control beneath the top-layer text.
    await page.getByTestId('click-rule').getByRole('button').focus();
    await expect(tooltip).toBeVisible();
    const underneath = await page.getByTestId('under-tooltip').boundingBox();
    const overlay = await tooltip.boundingBox();
    const x = Math.max(underneath!.x, overlay!.x) + 20;
    const y = Math.max(underneath!.y, overlay!.y) + 10;
    expect(y).toBeLessThan(
      Math.min(underneath!.y + underneath!.height, overlay!.y + overlay!.height),
    );
    await page.mouse.click(x, y);
    await expect(page.getByTestId('under-tooltip')).toHaveText('Underlying control · clicks 1');
    expect(errors).toEqual([]);
  });
}
