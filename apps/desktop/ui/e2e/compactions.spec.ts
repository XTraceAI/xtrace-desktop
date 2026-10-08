import { expect, test } from '@playwright/test';

test('dashboard scrolling keeps resolved compactions, ticks and running glow during delayed reads', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto('/e2e/compactions.html?view=dashboard&readDelay=600');
  const first = page.locator('.xt-data-row').filter({
    has: page.locator('[data-visible-id="compaction-1"]'),
  });
  const later = page.locator('.xt-data-row').filter({
    has: page.locator('[data-visible-id="compaction-20"]'),
  });
  await expect(first.getByLabel('Recorded compactions: 1', { exact: true })).toBeVisible();
  await expect(first.getByRole('img', { name: 'Codex · Running' })).toBeVisible();
  await expect(first.locator('.xt-compaction-tick')).toHaveCount(1);
  // This row has not been read yet: learning its count after the wheel proves
  // that scrolling changed the IDs sent to the source.
  await expect(later.getByLabel('Recorded compactions: unknown', { exact: true })).toHaveCount(1);
  const expected = { count: 'Recorded compactions: 1', ticks: 1, running: true };
  await first.evaluate((row) => {
    const read = () => ({
      count: row.querySelector('.xt-compaction')?.getAttribute('aria-label'),
      ticks: row.querySelectorAll('.xt-compaction-tick').length,
      running: row.querySelector('.xt-lane-live-host') !== null,
    });
    const changes: ReturnType<typeof read>[] = [];
    const observer = new MutationObserver(() => changes.push(read()));
    observer.observe(row, { subtree: true, childList: true, attributes: true });
    Object.assign(window, { scrollRowStates: changes, readScrollRow: read });
  });
  const scroll = page.getByRole('region', { name: 'Session lanes scroll area' });
  expect(
    await scroll.evaluate((element) => element.scrollHeight - element.clientHeight),
  ).toBeGreaterThan(3 * 23);
  await scroll.hover();
  await page.mouse.wheel(0, 400);
  await expect.poll(() => scroll.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await expect(later.getByLabel('Recorded compactions: 20', { exact: true })).toBeInViewport();
  // Check the actual nested scroll area, not just movement of its scrollbar.
  await expect
    .poll(() =>
      first.evaluate((row) => {
        const scroller = row.closest('.xt-table-scroll')!;
        return row.getBoundingClientRect().bottom <= scroller.getBoundingClientRect().top;
      }),
    )
    .toBe(true);
  await expect(first).not.toBeInViewport();
  const state = () =>
    page.evaluate(() => (window as unknown as { readScrollRow: () => unknown }).readScrollRow());
  expect(await state()).toEqual(expected);
  await page.mouse.wheel(0, -400);
  await expect(first.getByLabel('Recorded compactions: 1', { exact: true })).toBeInViewport();
  expect(await state()).toEqual(expected);
  const changes = await page.evaluate(
    () => (window as unknown as { scrollRowStates: unknown[] }).scrollRowStates,
  );
  for (const change of changes) expect(change).toEqual(expected);
  await expect(page.locator('.xt-live-session-badge')).toHaveCount(0);
});

for (const colorScheme of ['dark', 'light'] as const) {
  test(`recorded compactions on three surfaces fit 1120×720 in ${colorScheme}`, async ({
    page,
  }, info) => {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1120, height: 720 });
    await page.goto('/e2e/compactions.html?view=startup');
    await expect(page.getByRole('status')).toHaveText('Opening XTrace…');
    const splash = page.locator('.xt-startup');
    await expect(splash).toHaveCSS('opacity', '1');
    await expect(page.locator('.xt-startup-cut')).toHaveCount(4);
    await expect(page.locator('.xt-startup-cut').first()).toHaveCSS(
      'animation-name',
      'xt-startup-cut',
    );
    const mark = await page.locator('.xt-startup-mark').boundingBox();
    expect(mark).not.toBeNull();
    expect(Math.abs(mark!.x + mark!.width / 2 - 560)).toBeLessThan(2);
    await page.screenshot({ path: info.outputPath(`startup-${colorScheme}.png`) });
    await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' });
    await expect(splash).toHaveCSS('animation-name', 'none');
    await expect(page.locator('.xt-startup-cut').first()).toHaveCSS('animation-name', 'none');
    await page.emulateMedia({ colorScheme, reducedMotion: 'no-preference' });
    await page.goto('/e2e/compactions.html?view=sessions');
    const nine = page.getByLabel('Recorded compactions: 9', { exact: true });
    await expect(nine).toHaveCount(1);
    await expect(page.getByLabel('Recorded compactions: unknown', { exact: true })).toHaveCount(1);
    await expect(page.getByLabel('Recorded compactions: 0', { exact: true })).toHaveCount(1);
    await expect(nine.first()).toHaveAttribute('data-step', '5');
    await expect(page.getByLabel('Recorded compactions: 8', { exact: true })).toHaveCount(0);
    // A fork shows its own count plus the count before the fork, even zero.
    const fork = (own: number, from: string) =>
      page.getByLabel(`Recorded compactions: ${own} + ${from}`, { exact: true });
    await expect(fork(3, '0')).toHaveCount(1);
    await expect(fork(3, '0').first()).toHaveText('↺ 3 + 0');
    await expect(fork(2, 'unknown').first()).toHaveText('↺ 2 + ?');
    const scroll = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
    );
    expect(scroll).toBe(false);
    await nine.last().scrollIntoViewIfNeeded();
    await expect(
      page
        .locator('[role="row"]')
        .filter({ has: nine })
        .getByRole('img', { name: 'Codex · Running', exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('columnheader', { name: 'Compactions', exact: true }),
    ).toBeVisible();
    await expect(page.locator('.xt-session-name .xt-compaction')).toHaveCount(0);
    await nine.last().focus();
    await expect(page.getByRole('tooltip')).toContainText('not just this range');
    await expect(page.getByRole('tooltip')).toContainText('not a quality score');
    await page.keyboard.press('Escape');
    await fork(2, 'unknown').last().scrollIntoViewIfNeeded();
    await fork(2, 'unknown').last().focus();
    await expect(page.getByRole('tooltip')).toContainText(
      '2 in this conversation + an unknown number in the conversation it was forked from, before the fork. The saved conversation it was forked from could not be found.',
    );
    await page.keyboard.press('Escape');
    await page.screenshot({ path: info.outputPath(`sessions-compactions-${colorScheme}.png`) });
    await page.goto('/e2e/compactions.html?view=dashboard');
    await expect(page.getByLabel('Recorded compactions: 0', { exact: true })).toBeVisible();
    await expect(
      page.getByRole('columnheader', { name: 'Compactions', exact: true }),
    ).toBeVisible();
    await expect(page.locator('.xt-lane-name .xt-compaction')).toHaveCount(0);
    await expect(fork(3, '0')).toHaveText('↺ 3 + 0');
    // Its lane's ticks are its own compactions only.
    await expect(
      page
        .locator('[role="row"]')
        .filter({ has: fork(3, '0') })
        .locator('.xt-compaction-tick'),
    ).toHaveCount(3);
    await expect(
      page
        .locator('[role="row"]')
        .filter({ has: page.getByLabel('Recorded compactions: 0', { exact: true }) })
        .getByRole('img', { name: 'Codex · Running', exact: true }),
    ).toBeVisible();
    const rows = page.locator('.xt-lane-name');
    await expect(rows.first()).toBeVisible();
    const bounds = await rows.evaluateAll((rows) =>
      rows.map((row) => row.getBoundingClientRect().height),
    );
    expect(bounds.every((height) => height <= 24)).toBe(true);
    // Recorded compaction times are ticks on the lanes, focusable and named.
    const ticks = page.locator('.xt-compaction-tick');
    expect(await ticks.count()).toBeGreaterThan(0);
    const manual = page.getByLabel(/^Compaction recorded .+\. Manual\.$/).first();
    await manual.scrollIntoViewIfNeeded();
    await manual.focus();
    await expect(page.getByRole('tooltip')).toContainText('Compaction ·');
    await expect(page.getByRole('tooltip')).toContainText('Manual');
    const tick = await manual.boundingBox();
    const track = await manual.locator('xpath=..').boundingBox();
    expect(tick && track).toBeTruthy();
    expect(tick!.x + tick!.width / 2).toBeGreaterThanOrEqual(track!.x - 1);
    expect(tick!.x + tick!.width / 2).toBeLessThanOrEqual(track!.x + track!.width + 1);
    await page.screenshot({ path: info.outputPath(`dashboard-compactions-${colorScheme}.png`) });
  });
}
