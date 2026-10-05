import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 900, height: 800 } });

for (const theme of ['light', 'dark'] as const) {
  test(`formatted answers are bounded, inert and keyboard-reachable in ${theme}`, async ({
    page,
    browserName,
  }) => {
    const errors: string[] = [];
    const foreign: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => {
      if (!request.url().startsWith('http://127.0.0.1:')) foreign.push(request.url());
    });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/e2e/transcript-markdown.html');
    const region = page.getByRole('region', { name: 'Session transcript' });
    await expect(region.locator('[data-format="markdown"]').first()).toBeVisible();

    // No horizontal page scroll, whatever the table's width.
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );

    // The person's prompt is literal; the answer is formatted.
    await expect(region.locator('[data-block-id="e2e-user:0"]')).toHaveText(
      'Keep **this** `literal`, please.',
    );
    await expect(region.locator('[data-block-id="e2e-answer:0"] h4')).toHaveText('Summary');

    // The table is bounded in both directions and scrolls from the keyboard.
    const table = region.getByRole('region', { name: 'Table, scrollable' }).first();
    const tableBox = await table.evaluate((el) => ({
      wide: el.scrollWidth > el.clientWidth,
      tall: el.scrollHeight > el.clientHeight,
      width: el.getBoundingClientRect().width,
    }));
    expect(tableBox.wide).toBe(true);
    expect(tableBox.tall).toBe(true);
    expect(tableBox.width).toBeLessThanOrEqual(720);

    await page
      .getByRole('region', { name: 'Session transcript' })
      .click({ position: { x: 2, y: 2 } });
    // Tab from the top of the transcript reaches the table first: the prompt and the
    // heading are not stops.
    await page.keyboard.press('Tab');
    await expect(table).toBeFocused();
    const ring = await table.evaluate((el) => getComputedStyle(el).outlineStyle);
    expect(ring).not.toBe('none');
    // Page keys scroll it in both engines. Arrow keys are asserted in Chromium only: the
    // WebKit build Playwright drives does not act on them for any focused scroll box here —
    // the transcript's existing code surface included — so that is recorded as a limit, not
    // claimed.
    await page.keyboard.press('PageDown');
    await expect.poll(() => table.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
    if (browserName === 'chromium') {
      await page.keyboard.press('ArrowRight');
      await page.keyboard.press('ArrowRight');
      await expect.poll(() => table.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
    }

    // The fence is the existing code surface: the next stop, bounded, and it scrolls.
    await page.keyboard.press('Tab');
    const code = region.getByRole('region', { name: 'text, scrollable code' });
    await expect(code).toBeFocused();
    expect(await code.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
    await page.keyboard.press('PageDown');
    await expect.poll(() => code.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);

    // The hostile answer made nothing: no link, no image, nothing ran, nothing fetched.
    const hostile = region.locator('[data-block-id="e2e-answer:2"]');
    await expect(hostile).toContainText('<script>window.__ran = 1</script>');
    await expect(hostile).toContainText('go (javascript:window.__ran=2)');
    await expect(hostile).toContainText('Image not shown: pixel');
    expect(await region.locator('a, img, script, [href], [src]').count()).toBe(0);
    await hostile.locator('[data-md-link]').first().click();
    expect(await page.evaluate(() => (window as { __ran?: unknown }).__ran)).toBeUndefined();
    expect(page.url()).toContain('/e2e/transcript-markdown.html');

    // The long answer is a literal preview until opened, and nothing focusable hides under it.
    const long = region.locator('[data-block-id="e2e-long:0"]');
    // Found by class, not by name: its name flips to "Show less" once it is pressed.
    const toggle = long.locator('.xt-clamp-toggle');
    await expect(toggle).toHaveText('Show the rest of this block');
    // From the fence, the next stop is the toggle: nothing between them takes focus. (The
    // click on inert link text above moved WebKit's Tab start point, so start from the fence.)
    // WebKit follows Safari's default and leaves buttons out of plain Tab; Option+Tab is how a
    // Safari reader reaches this (unchanged) button. The table and code boxes above are
    // `tabindex="0"` regions and are reached by plain Tab in both engines.
    await code.focus();
    await page.keyboard.press(browserName === 'webkit' ? 'Alt+Tab' : 'Tab');
    await expect(toggle).toBeFocused();
    expect(await long.locator('[data-clamped] [tabindex], [data-clamped] table').count()).toBe(0);
    await page.keyboard.press('Enter');
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await expect(toggle).toBeFocused();
    await expect(long).toContainText('Final synthetic sentence.');
    await expect(long.locator('tbody tr')).toHaveCount(60);
    const longTable = long.getByRole('region', { name: 'Table, scrollable' });
    expect(await longTable.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
    await page.keyboard.press('Space');
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle).toBeFocused();

    // Tokens resolve in this theme: text and table header read against their backgrounds.
    const colours = await region.locator('[data-block-id="e2e-answer:0"]').evaluate((el) => {
      const md = el.querySelector('.xt-md')!;
      const th = el.querySelector('th')!;
      return {
        ink: getComputedStyle(md).color,
        header: getComputedStyle(th).backgroundColor,
        scheme: document.documentElement.dataset.theme,
      };
    });
    expect(colours.scheme).toBe(theme);
    expect(colours.ink).not.toBe(colours.header);
    const luminance = (rgb: string) => {
      const [r, g, b] = rgb.match(/[\d.]+/g)!.map(Number);
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    if (theme === 'light') expect(luminance(colours.ink)).toBeLessThan(luminance(colours.header));
    else expect(luminance(colours.ink)).toBeGreaterThan(luminance(colours.header));

    expect(foreign).toEqual([]);
    expect(errors).toEqual([]);
  });
}
