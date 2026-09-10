import { expect, test } from '@playwright/test';
import { comparisons } from './map';

for (const entry of comparisons) {
  test(entry.id, async ({ page }) => {
    const errors: string[] = [];
    const external: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    page.on('request', (request) => {
      const url = new URL(request.url());
      if (url.protocol.startsWith('http') && url.origin !== 'http://127.0.0.1:5184')
        external.push(url.origin);
    });
    await page.goto(`/gallery?story=${encodeURIComponent(entry.story)}`);
    const frame = page.frameLocator(`iframe[title="${entry.story} · ${entry.theme}"]`);
    const root = frame.locator('.gallery-frame');
    await expect(root).toHaveAttribute('data-story-id', entry.story);
    await expect(root).toHaveAttribute('data-theme', entry.theme);
    await frame.locator('body').evaluate(async () => {
      await document.fonts.ready;
      await Promise.all(Array.from(document.images, (image) => image.decode()));
      if (
        !document.fonts.check('400 12px Manrope') ||
        !document.fonts.check('400 12px "Geist Mono"')
      )
        throw new Error('Bundled fonts did not load');
    });
    const bounds = await root.boundingBox();
    expect(bounds?.width).toBe(entry.size[0]);
    expect(bounds?.height).toBe(entry.size[1]);
    if (process.env.XTRACE_PARITY_PROBE === 'visible-drift') {
      // Change actual component layout in the disposable browser document only.
      await root.locator('.xt-sidebar').evaluate((element) => {
        (element as HTMLElement).style.borderLeft = '12px solid var(--accent)';
      });
    }
    await expect(root).toHaveScreenshot(`${entry.id}.png`);
    expect(errors).toEqual([]);
    expect(external).toEqual([]);
  });
}
