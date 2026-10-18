import { expect, test, type Page } from '@playwright/test';

/**
 * The main window's telemetry notice floats over the bottom-right corner: the
 * Dashboard under it keeps exactly the box it has without the notice, the
 * card stays inside the window at small sizes, takes no focus, and sits above
 * the page but below popovers and dialogs. Synthetic telemetry controls over
 * the F1 export; nothing is saved or sent.
 */
async function open(page: Page, notice: boolean, scheme: 'light' | 'dark' = 'light') {
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto(`/e2e/telemetry-notice.html?notice=${notice ? 1 : 0}`);
  const dashboard = page.locator('.xt-dashboard');
  await expect(dashboard).toBeVisible();
  await expect(page.getByTestId('report-period')).toBeVisible();
  const card = page.getByRole('region', { name: 'Anonymous usage data' });
  if (notice) await expect(card).toBeVisible();
  else await expect(card).toHaveCount(0);
  return { dashboard, card };
}

const box = async (page: Page, notice: boolean) => {
  const { dashboard } = await open(page, notice);
  // Two frames, so any layout the notice could cause has happened.
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
  return dashboard.boundingBox();
};

for (const size of [
  { width: 1120, height: 720 },
  { width: 1440, height: 900 },
]) {
  test(`the notice takes no room from the Dashboard at ${size.width}x${size.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    const without = await box(page, false);
    const withNotice = await box(page, true);
    expect(withNotice).toEqual(without);
    const card = await page.locator('.xt-telemetry-floating').boundingBox();
    expect(card).not.toBeNull();
    // Bottom-right, 16px from the window's edges.
    expect(Math.round(card!.x + card!.width)).toBe(size.width - 16);
    expect(Math.round(card!.y + card!.height)).toBeLessThanOrEqual(size.height - 16);
    expect(card!.width).toBeLessThanOrEqual(380);
  });
}

const inside = async (page: Page, width: number, height: number) => {
  const card = (await page.locator('.xt-telemetry-floating').boundingBox())!;
  expect(card.x).toBeGreaterThanOrEqual(16);
  expect(card.y).toBeGreaterThanOrEqual(16);
  expect(card.x + card.width).toBeLessThanOrEqual(width - 16 + 0.5);
  expect(card.y + card.height).toBeLessThanOrEqual(height - 16 + 0.5);
  return card;
};

test('the card fits a 420x320 window', async ({ page }) => {
  await page.setViewportSize({ width: 420, height: 320 });
  await open(page, true);
  await inside(page, 420, 320);
});

test('in a very short window the card scrolls inside itself', async ({ page }) => {
  await page.setViewportSize({ width: 420, height: 190 });
  await open(page, true);
  await inside(page, 420, 190);
  const floating = page.locator('.xt-telemetry-floating');
  const overflow = await floating.evaluate((node) => ({
    scrolls: node.scrollHeight > node.clientHeight,
    style: getComputedStyle(node).overflowY,
  }));
  expect(overflow).toEqual({ scrolls: true, style: 'auto' });
  // The buttons stay reachable by keyboard inside the scrolled card.
  const turnOff = page.getByRole('button', { name: 'Turn off' });
  await turnOff.focus();
  await expect(turnOff).toBeFocused();
  await expect(turnOff).toBeInViewport();
});

test('a narrow card lists in one column', async ({ page }) => {
  await page.setViewportSize({ width: 340, height: 600 });
  await open(page, true);
  await inside(page, 340, 600);
  const lists = page.locator('.xt-telemetry-list');
  const [sent, never] = await Promise.all([lists.nth(0).boundingBox(), lists.nth(1).boundingBox()]);
  expect(never!.y).toBeGreaterThanOrEqual(sent!.y + sent!.height - 0.5);
  const popup = page.locator('.xt-telemetry-popup');
  expect(await popup.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  // At full width the two lists sit side by side.
  await page.setViewportSize({ width: 1120, height: 720 });
  const [wideSent, wideNever] = await Promise.all([
    lists.nth(0).boundingBox(),
    lists.nth(1).boundingBox(),
  ]);
  expect(wideNever!.x).toBeGreaterThan(wideSent!.x + wideSent!.width - 0.5);
});

test('the pop-up is one flat surface, much shorter than the old card', async ({ page }) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  await open(page, true);
  const card = (await page.locator('.xt-telemetry-floating').boundingBox())!;
  // About 200px (197-199px measured), well under the old card's ~352px; a
  // little room for font metrics.
  expect(card.height).toBeLessThanOrEqual(210);
  // The line naming the events stays on one line.
  const sub = page.locator('.xt-telemetry-popup-sub');
  expect(
    await sub.evaluate(
      (node) => node.getBoundingClientRect().height / parseFloat(getComputedStyle(node).lineHeight),
    ),
  ).toBeLessThan(1.5);
  const layers = await page.locator('.xt-telemetry-popup').evaluate(
    (node) =>
      [node, ...node.querySelectorAll('*')].filter((element) => {
        const style = getComputedStyle(element);
        return (
          style.borderTopWidth !== '0px' &&
          style.borderTopStyle !== 'none' &&
          style.borderTopColor !== 'rgba(0, 0, 0, 0)'
        );
      }).length,
  );
  // Only the pop-up's own hairline: the buttons' borders are transparent.
  expect(layers).toBe(1);
  // "Got it" is the filled, dominant control; Turn off is quiet.
  const fills = await page
    .locator('.xt-telemetry-popup .xt-button')
    .evaluateAll((buttons) =>
      buttons.map((button) => [button.textContent, getComputedStyle(button).backgroundColor]),
    );
  expect(fills[0][0]).toBe('Got it');
  expect(fills[0][1]).not.toBe('rgba(0, 0, 0, 0)');
  expect(fills[1]).toEqual(['Turn off', 'rgba(0, 0, 0, 0)']);
});

test('the card takes no focus, is recorded once visible, and sits below dialogs', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  const { card } = await open(page, true);
  await expect
    .poll(() => page.evaluate(() => (window as { telemetryShown?: number }).telemetryShown))
    .toBe(1);
  expect(await card.evaluate((node) => node.contains(document.activeElement))).toBe(false);
  const layers = await page.evaluate(() => {
    const floating = document.querySelector('.xt-telemetry-floating')!;
    const probe = document.createElement('div');
    probe.className = 'xt-modal';
    document.body.append(probe);
    const modal = Number(getComputedStyle(probe).zIndex);
    probe.remove();
    return { notice: Number(getComputedStyle(floating).zIndex), modal };
  });
  expect(layers.notice).toBeGreaterThan(1);
  expect(layers.notice).toBeLessThan(layers.modal);
  // Got it removes it and changes nothing that was saved.
  await card.getByRole('button', { name: 'Got it' }).click();
  await expect(card).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as { telemetryTurnedOff?: boolean }).telemetryTurnedOff),
  ).toBeUndefined();
});

test('Escape closes the card, which comes after the page in keyboard order', async ({ page }) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  const { card } = await open(page, true);
  const order = await page.evaluate(() => {
    const outlet = document.querySelector('.xt-shell-outlet')!;
    const floating = document.querySelector('.xt-telemetry-floating')!;
    return floating === outlet.lastElementChild;
  });
  expect(order).toBe(true);
  await page.keyboard.press('Escape');
  await expect(card).toHaveCount(0);
  // Closing changed nothing saved: it was never turned off.
  expect(
    await page.evaluate(() => (window as { telemetryTurnedOff?: boolean }).telemetryTurnedOff),
  ).toBeUndefined();
});

for (const scheme of ['light', 'dark'] as const) {
  test(`screenshot over the Dashboard, ${scheme}`, async ({ page }, info) => {
    const directory = process.env.TELEMETRY_SCREENSHOT_DIR;
    test.skip(!directory || info.project.name !== 'webkit', 'screenshots only on request');
    await page.setViewportSize({ width: 1280, height: 800 });
    await open(page, true, scheme);
    await page.screenshot({ path: `${directory}/telemetry-notice-${scheme}.png` });
  });
}
