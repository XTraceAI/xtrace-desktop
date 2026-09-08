import { expect, test } from '@playwright/test';
import inventory from '../src/gallery/inventory.json' with { type: 'json' };

test('every declared story mounts in isolated dark and light frames without overflow', async ({
  page,
}, info) => {
  const errors: string[] = [],
    external: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.protocol.startsWith('http') && url.origin !== 'http://127.0.0.1:5183')
      external.push(url.origin);
  });
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' });
  for (const story of inventory) {
    await test.step(story.id, async () => {
      await page.goto(`/gallery?story=${encodeURIComponent(story.id)}`);
      await expect(page.getByRole('heading', { name: story.id, exact: true })).toBeVisible();
      await expect(
        page.getByText('[SAMPLE] Illustrative component data', { exact: true }),
      ).toBeVisible();
      await expect(page.getByRole('navigation').getByRole('button')).toHaveCount(inventory.length);
      for (const [index, theme] of ['dark', 'light'].entries()) {
        const frame = page.frameLocator('iframe').nth(index);
        const root = frame.locator('.gallery-frame');
        await expect(root).toHaveAttribute('data-story-id', story.id);
        await expect(root).toHaveAttribute('data-theme', theme);
        await frame.locator('body').evaluate(() => document.fonts.ready);
        if (story.id === 'modal/open') await expect(frame.getByRole('dialog')).toBeVisible();
        if (
          /^(popover\/open|hub\/|rulepopover\/open-context|filtermenu\/(open|empty))/.test(
            story.id,
          ) ||
          story.id === 'sidebar/hub-popover-open'
        )
          await expect(frame.locator(':popover-open')).toHaveCount(1);
        const box = await root.evaluate((element) => ({
          width: element.clientWidth,
          height: element.clientHeight,
          scrollWidth: element.scrollWidth,
          scrollHeight: element.scrollHeight,
        }));
        expect(box.width).toBe(story.size[0]);
        expect(box.height).toBe(story.size[1]);
        if (story.id.startsWith('sidebar/')) {
          const sidebar = await frame.locator('.xt-sidebar').boundingBox();
          expect(sidebar?.width).toBe(228);
          expect(sidebar?.height).toBe(900);
        }
        expect(box.scrollWidth, `${story.id} ${theme} horizontal overflow`).toBeLessThanOrEqual(
          box.width,
        );
        expect(box.scrollHeight, `${story.id} ${theme} vertical overflow`).toBeLessThanOrEqual(
          box.height,
        );
      }
      expect(errors).toEqual([]);
      expect(external).toEqual([]);
      if (
        [
          'sidebar/hub-popover-open',
          'topbar/default',
          'button/variants-heights-disabled-icons',
          'datatable/rulebook-44-expanded',
          'modal/open',
        ].includes(story.id)
      )
        await page.screenshot({
          path: info.outputPath(`${story.id.replaceAll('/', '-')}.png`),
          clip: {
            x: 0,
            y: 0,
            width: Math.max(1240, 244 + 48 + 2 * story.size[0] + 24),
            height: Math.max(560, story.size[1] + 220),
          },
        });
    });
  }
});

test('story navigation resets local state and theme frames own independent controls and top layers', async ({
  page,
}) => {
  await page.goto('/gallery?story=toggle%2Foff');
  const dark = page.frameLocator('iframe').nth(0);
  const light = page.frameLocator('iframe').nth(1);
  await dark.getByRole('switch').click();
  await expect(dark.getByRole('switch')).toBeChecked();
  await expect(light.getByRole('switch')).not.toBeChecked();
  await page.getByRole('button', { name: 'modal/open', exact: true }).click();
  await expect(dark.getByRole('dialog')).toBeVisible();
  await expect(light.getByRole('dialog')).toBeVisible();
  await dark.getByRole('dialog').getByRole('textbox').focus();
  await page.keyboard.press('Escape');
  await expect(dark.getByRole('dialog')).not.toBeVisible();
  await expect(light.getByRole('dialog')).toBeVisible();
  await page.getByRole('button', { name: 'toggle/off', exact: true }).click();
  await expect(dark.getByRole('switch')).not.toBeChecked();
  await expect(light.getByRole('switch')).not.toBeChecked();
});
