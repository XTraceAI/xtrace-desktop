import { expect, test, type Page } from '@playwright/test';
import { performance } from 'node:perf_hooks';
import fixture from '../fixtures/F1.json' with { type: 'json' };

test.use({ viewport: { width: 1120, height: 720 } });

/** The clock's documented maximum age, `windowClockMaxAgeMs`. */
const maxAge = 15 * 60_000;
type Reads = Record<string, number>;

const reads = (page: Page) =>
  page.evaluate(() => ({ ...(window as unknown as { __reads: Reads }).__reads }));
/** Only the reads that began between two counts, by method. */
const began = (before: Reads, after: Reads) =>
  Object.fromEntries(
    Object.entries(after)
      .filter(([method, count]) => count !== before[method])
      .map(([method, count]) => [method, count - before[method]]),
  );
/** The counts once no read has begun for a moment. */
const settled = async (page: Page) => {
  let last = '';
  await expect
    .poll(
      async () => {
        const now = JSON.stringify(await reads(page));
        const same = now === last;
        last = now;
        return same;
      },
      { intervals: [300] },
    )
    .toBe(true);
  return reads(page);
};
/** The app's own Shell over F1, on a fake clock that still flows. */
const open = async (
  page: Page,
  path: string,
  pages = 1,
  hold?: 'refresh' | 'next',
  groups = false,
) => {
  await page.clock.install({ time: new Date('2026-09-23T12:00:00Z') });
  await page.goto(
    `/e2e/window-clock.html?path=${encodeURIComponent(path)}&pages=${pages}${hold ? `&hold=${hold}` : ''}${groups ? '&groups' : ''}`,
  );
};
const setVisibility = (page: Page, state: DocumentVisibilityState) =>
  page.evaluate((state) => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
    document.dispatchEvent(new Event('visibilitychange'));
  }, state);
const focusWindow = (page: Page) =>
  page.evaluate(() => {
    window.dispatchEvent(new Event('focus'));
    window.dispatchEvent(new Event('focus'));
  });
const geometry = (page: Page) =>
  page.evaluate(() =>
    ['.xt-sidebar', '.xt-topbar-tools', '.xt-dashboard'].map((selector) => {
      const box = document.querySelector(selector)!.getBoundingClientRect();
      return [box.x, box.y, box.width, box.height].map(Math.round);
    }),
  );
const firstVisible = (page: Page) =>
  page.evaluate(() => {
    const scroll = document.querySelector<HTMLElement>('.xt-table-scroll')!;
    const top =
      scroll.getBoundingClientRect().top +
      scroll.querySelector<HTMLElement>('.xt-table-head')!.getBoundingClientRect().height;
    const block = [...scroll.querySelectorAll<HTMLElement>('.xt-table-block')].find(
      (row) => row.getBoundingClientRect().bottom > top,
    )!;
    return {
      name: block.querySelector('.xt-session-open')!.getAttribute('aria-label'),
      offset: Math.round(block.getBoundingClientRect().top - top),
    };
  });
const scrollToRow = (page: Page, index: number) =>
  page
    .locator('.xt-table-block')
    .nth(index)
    .evaluate((block) => {
      block.scrollIntoView({ block: 'start' });
      block.closest('.xt-table-scroll')!.dispatchEvent(new Event('scroll'));
    });
const releaseList = (page: Page) =>
  page.evaluate(() => (window as unknown as { __releaseList: () => void }).__releaseList());
const holdNextList = (page: Page) =>
  page.evaluate(() => Object.assign(window, { __holdNextList: true }));

test('re-reads the Dashboard route once at its due time and keeps range, focus and geometry', async ({
  page,
}) => {
  await open(page, '/dashboard');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
  const range = page.getByRole('radiogroup', { name: 'Date range' });
  await range.getByRole('radio', { name: '14d' }).click();
  await expect(page.getByTestId('report-period')).toBeVisible();
  const before = await settled(page);
  const boxes = await geometry(page);
  await range.getByRole('radio', { name: '14d' }).focus();

  await page.clock.fastForward(maxAge - 30_000);
  await focusWindow(page);
  expect(began(before, await settled(page))).toEqual({});

  await page.clock.fastForward(60_000);
  const after = await settled(page);
  expect(began(before, after)).toEqual({ dashboard: 1, tokensByHost: 1 });
  // The Dashboard mounts Overview, not Environment; an inactive query is not refetched.
  expect(after.environment).toBe(before.environment);
  await expect(range.getByRole('radio', { name: '14d' })).toBeChecked();
  await expect(range.getByRole('radio', { name: '14d' })).toBeFocused();
  await expect(page.getByTestId('report-period')).toBeVisible();
  expect(await geometry(page)).toEqual(boxes);

  // Hidden: nothing is read however long it stays hidden; shown and focused
  // together, what fell due is read once.
  await setVisibility(page, 'hidden');
  await page.clock.fastForward(3 * maxAge);
  expect(began(after, await settled(page))).toEqual({});
  await setVisibility(page, 'visible');
  await focusWindow(page);
  const shown = await settled(page);
  expect(began(after, shown)).toEqual({ dashboard: 1, tokensByHost: 1 });
  expect(shown.environment).toBe(after.environment);
});

test('re-reads the Sessions summary, host tokens and loaded list page together', async ({
  page,
}) => {
  await open(page, '/sessions');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Sessions');
  const before = await settled(page);
  await page.clock.fastForward(maxAge);
  const after = await settled(page);
  expect(began(before, after)).toEqual({ dashboard: 1, tokensByHost: 1, sessionsList: 1 });
  await setVisibility(page, 'hidden');
  await page.clock.fastForward(3 * maxAge);
  expect(began(after, await settled(page))).toEqual({});
  await setVisibility(page, 'visible');
  await focusWindow(page);
  expect(began(after, await settled(page))).toEqual({
    dashboard: 1,
    tokensByHost: 1,
    sessionsList: 1,
  });
});

test('replays exactly three loaded Sessions pages and keeps the visible row and focus', async ({
  page,
}, testInfo) => {
  await open(page, '/sessions', 3);
  const blocks = page.locator('.xt-table-block');
  await expect(blocks).toHaveCount(50);
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect(blocks).toHaveCount(100);
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect(blocks).toHaveCount(150);
  await scrollToRow(page, 110);
  const before = await firstVisible(page);
  const expand = page.getByRole('button', {
    name: new RegExp(`^(Expand|Collapse) ${before.name!.split(' ').at(-1)}$`),
  });
  await expand.click();
  await expect(expand).toBeFocused();
  const anchored = await firstVisible(page);
  const counts = await settled(page);
  const begun = performance.now();
  await page.clock.fastForward(maxAge);
  await expect.poll(async () => (await reads(page)).sessionsList).toBe(counts.sessionsList + 3);
  await expect.poll(async () => (await firstVisible(page)).name).toBe(anchored.name);
  const elapsedMs = Math.round(performance.now() - begun);
  testInfo.annotations.push({ type: 'fixture-refresh-ms', description: String(elapsedMs) });
  expect(began(counts, await settled(page))).toEqual({
    dashboard: 1,
    tokensByHost: 1,
    sessionsList: 3,
  });
  const after = await firstVisible(page);
  expect(Math.abs(after.offset - anchored.offset)).toBeLessThanOrEqual(1);
  await expect(expand).toBeFocused();
  await expect(expand).toHaveAttribute('aria-expanded', 'true');
});

test('keeps the row a user scrolls to during a pending Sessions refresh', async ({ page }) => {
  await open(page, '/sessions', 3, 'refresh');
  const blocks = page.locator('.xt-table-block');
  await expect(blocks).toHaveCount(50);
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect(blocks).toHaveCount(100);
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect(blocks).toHaveCount(150);
  await scrollToRow(page, 110);
  const original = await firstVisible(page);
  const before = await settled(page);
  await holdNextList(page);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect.poll(async () => (await reads(page)).sessionsList).toBe(before.sessionsList + 1);
  await scrollToRow(page, 125);
  const moved = await firstVisible(page);
  expect(moved.name).not.toBe(original.name);
  await releaseList(page);
  await expect.poll(async () => (await reads(page)).sessionsList).toBe(before.sessionsList + 3);
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  const after = await firstVisible(page);
  expect(after.name).toBe(moved.name);
  expect(Math.abs(after.offset - moved.offset)).toBeLessThanOrEqual(1);
});

test('keeps the row a user scrolls to while the next Sessions page is pending', async ({
  page,
}) => {
  await open(page, '/sessions', 3, 'next');
  const blocks = page.locator('.xt-table-block');
  await expect(blocks).toHaveCount(50);
  const original = await firstVisible(page);
  const before = await settled(page);
  await holdNextList(page);
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect.poll(async () => (await reads(page)).sessionsList).toBe(before.sessionsList + 1);
  await scrollToRow(page, 20);
  const moved = await firstVisible(page);
  expect(moved.name).not.toBe(original.name);
  await releaseList(page);
  await expect(blocks).toHaveCount(100);
  const after = await firstVisible(page);
  expect(after.name).toBe(moved.name);
  expect(Math.abs(after.offset - moved.offset)).toBeLessThanOrEqual(1);
});

test('keeps an open group and the visible row when its parent arrives late and the list refreshes', async ({
  page,
}) => {
  await open(page, '/sessions', 3, undefined, true);
  const blocks = page.locator('.xt-table-block');
  // Five first-page sub-sessions are collapsed under one row that names their
  // parent on the third page; opened, each is its own row.
  await expect(blocks).toHaveCount(46);
  const group = page.getByRole('button', { name: '5 loaded sub-sessions of Clock parent' });
  const absent = page.locator('[data-group="absent"]');
  await expect(absent).toHaveCount(1);
  await group.click();
  await expect(blocks).toHaveCount(51);
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect(blocks).toHaveCount(101);
  // The row naming the parent is the first visible row while the parent's
  // own page loads; the parent's row takes its place, still open.
  await scrollToRow(page, 10);
  const named = await firstVisible(page);
  expect(named.name).toBe('Open parent session Clock parent, clock-session-2-49');
  await page.getByRole('button', { name: 'Load more sessions' }).click();
  await expect(blocks).toHaveCount(150);
  await expect(absent).toHaveCount(0);
  await expect(group).toHaveAttribute('aria-expanded', 'true');
  const replaced = await firstVisible(page);
  expect(replaced.name).toBe('Open session Clock parent, clock-session-2-49');
  expect(Math.abs(replaced.offset - named.offset)).toBeLessThanOrEqual(1);
  await expect(blocks.nth(11).locator('.xt-session-open')).toHaveAttribute(
    'aria-label',
    'Open session clock-session-0-10',
  );

  // A timed refresh replays all three pages (the third now in another order)
  // and keeps the open child that was first visible where it was.
  await scrollToRow(page, 13);
  const anchored = await firstVisible(page);
  expect(anchored.name).toBe('Open session clock-session-0-12');
  const counts = await settled(page);
  await page.clock.fastForward(maxAge);
  await expect.poll(async () => (await reads(page)).sessionsList).toBe(counts.sessionsList + 3);
  await expect.poll(async () => (await firstVisible(page)).name).toBe(anchored.name);
  const after = await firstVisible(page);
  expect(Math.abs(after.offset - anchored.offset)).toBeLessThanOrEqual(1);
  await expect(group).toHaveAttribute('aria-expanded', 'true');
});

test('re-reads one session and the host tokens, never its transcript', async ({ page }) => {
  const id = fixture.sessions[0].rows[0].id;
  await open(page, `/sessions/${encodeURIComponent(id)}?range=7d`);
  await expect(page.getByRole('link', { name: '← All sessions' })).toBeVisible();
  const before = await settled(page);
  expect(before.sessionTranscript).toBeGreaterThan(0);
  await page.clock.fastForward(maxAge - 30_000);
  expect(began(before, await settled(page))).toEqual({});
  await page.clock.fastForward(60_000);
  expect(began(before, await settled(page))).toEqual({
    sessionRow: 1,
    sessionStretches: 1,
    tokensByHost: 1,
  });
});

test('re-reads the PRs report, and nothing while the inventory is shown', async ({ page }) => {
  await open(page, '/prs');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Pull requests');
  const before = await settled(page);
  expect(before.pullRequestAnalytics).toBeGreaterThan(0);
  await page.clock.fastForward(maxAge + 30_000);
  const after = await settled(page);
  expect(began(before, after)).toEqual({ pullRequestAnalytics: 1, tokensByHost: 1 });

  await page.getByRole('radio', { name: /inventory/i }).click();
  const inventory = await settled(page);
  await page.clock.fastForward(3 * maxAge);
  await focusWindow(page);
  expect(began(inventory, await settled(page))).toEqual({});
});
