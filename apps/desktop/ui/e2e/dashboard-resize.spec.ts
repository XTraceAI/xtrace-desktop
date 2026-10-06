import { expect, test, type Locator, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardLane } from '../src/data/generated/DashboardLane';
import type { DashboardLaneSession } from '../src/data/generated/DashboardLaneSession';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The Dashboard's resizable layout. Synthetic lane sessions (not real
 * history) in place of the development F1 export: three, the case where
 * Sessions used to shrink to its rows and hand Effort and Overview the
 * rest of the window, and fourteen, more than the eight Sessions reserves.
 *
 * By default Sessions keeps room for eight rows; the boundary between the
 * effort row and Sessions and the one between Effort and Overview can be
 * dragged or moved from the keyboard, each pane keeps its minimum, the choice
 * survives a reload and a window resize as a proportion, and a double-click
 * or Enter puts the default back.
 */
const STORAGE = 'xt.dashboard.layout.v1';
const ROW = 23;
const HEAD = 24;
/** The Sessions card around its lane table: header, borders, padding and margin. */
const CARD_CHROME = 50;
const sessionsHeight = (rows: number) => CARD_CHROME + HEAD + rows * ROW - 1;
const EFFORT_MIN = 480;
const OVERVIEW_MIN = 300;

function withSessions(count: number): FixtureExport {
  const out = structuredClone(fixture as FixtureExport);
  for (const report of out.dashboards) {
    const start = report.lane_start_ms;
    const minute = 60_000;
    report.lanes = [];
    report.lane_sessions = [];
    for (let index = 0; index < count; index += 1) {
      const id = `resize-session-${String(index).padStart(2, '0')}`;
      const offset = 2400 - index * 120;
      const lane: DashboardLane = {
        session_id: id,
        host: ['claude', 'codex', 'cursor'][index % 3],
        start_ms: start + offset * minute,
        end_ms: start + (offset + 40) * minute,
      };
      const session: DashboardLaneSession = {
        session_id: id,
        host: lane.host,
        repo: `/Users/developer/code/resize-${index}`,
        branch: 'main',
        title: `Synthetic resize session ${index}`,
        automated_review: false,
        started_at_ms: lane.start_ms,
        pr_links: 0,
        inferred_pr_links: 0,
        cost: null,
      };
      report.lanes.push(lane);
      report.lane_sessions.push(session);
    }
    report.lanes_total = count;
    report.lanes_truncated = false;
  }
  return out;
}

async function open(page: Page, body: FixtureExport, width: number, height: number) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.goto('/dashboard');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await expect(page.getByTestId('overview-grid')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

type Layout = {
  main: number;
  sessions: number;
  effort: { top: number; width: number; height: number; left: number };
  overview: { top: number; width: number; height: number; left: number };
  /** Lane rows wholly inside the lane table's scroll area. */
  laneRows: number;
  /** The lane table's scroll area, header included. */
  lanesArea: number;
  overviewValues: number;
  effortPlot: number | null;
  outletScrolls: boolean;
  pageWidth: number;
  /** Panel children that overflow their own width (a squeezed header, a clipped label). */
  clipped: string[];
  headerHeights: { effort: number; overview: number };
};

function measure(): Layout {
  const card = (name: string) =>
    [...document.querySelectorAll('.xt-dashboard h2')]
      .find((heading) => heading.textContent === name)!
      .closest('section')!;
  const box = (element: Element) => element.getBoundingClientRect();
  const inside = (row: Element, frame: DOMRect) => {
    const b = box(row);
    return b.top >= frame.top - 0.5 && b.bottom <= frame.bottom + 0.5;
  };
  const lanes = document.querySelector('.xt-lanes .xt-table-scroll')!;
  const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
  const rect = (element: Element) => {
    const b = box(element);
    return { top: b.top, width: b.width, height: b.height, left: b.left };
  };
  const effort = card('Effort');
  const overview = card('Overview');
  return {
    main: box(document.querySelector('.xt-dash-main-row')!).height,
    sessions: box(card('Sessions')).height,
    effort: rect(effort),
    overview: rect(overview),
    laneRows: [...lanes.querySelectorAll('.xt-data-row')].filter((row) => inside(row, box(lanes)))
      .length,
    lanesArea: box(lanes).height,
    // Overview tiles whose number is wholly inside the card.
    overviewValues: [...overview.querySelectorAll('[data-testid="overview-value"]')].filter(
      (value) => inside(value, box(overview)),
    ).length,
    effortPlot: (() => {
      const plot = document.querySelector('.xt-effort-plot');
      return plot ? box(plot).height : null;
    })(),
    outletScrolls: outlet.scrollHeight > outlet.clientHeight + 1,
    pageWidth: document.documentElement.scrollWidth,
    clipped: [...document.querySelectorAll<HTMLElement>('.xt-dash-card-header *')]
      .filter((element) => {
        const style = getComputedStyle(element);
        const clips = style.overflow !== 'visible' || style.textOverflow === 'ellipsis';
        return clips && element.scrollWidth > element.clientWidth + 1;
      })
      .map((element) => `${element.className}: ${element.textContent?.slice(0, 40)}`),
    headerHeights: {
      effort: box(effort.querySelector('.xt-dash-card-header')!).height,
      overview: box(overview.querySelector('.xt-dash-card-header')!).height,
    },
  };
}

const layout = (page: Page) => page.evaluate(measure);
const stored = (page: Page) => page.evaluate((key) => window.localStorage.getItem(key), STORAGE);
const rowsHandle = (page: Page) =>
  page.getByRole('separator', { name: 'Resize Effort and Overview against Sessions' });
const columnsHandle = (page: Page) =>
  page.getByRole('separator', { name: 'Resize Effort and Overview', exact: true });

/** Drags a handle by `dx`, `dy` from its centre, in small steps. */
async function drag(page: Page, handle: Locator, dx: number, dy: number) {
  const b = (await handle.boundingBox())!;
  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y + dy / 2, { steps: 4 });
  await page.mouse.move(x + dx, y + dy, { steps: 4 });
  await page.mouse.up();
}

const SIZES = [
  [1440, 900],
  [1120, 720],
] as const;

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  test.info().attach = test.info().attach.bind(test.info());
  (page as Page & { errors?: string[] }).errors = errors;
});
test.afterEach(async ({ page }) => {
  expect((page as Page & { errors?: string[] }).errors ?? []).toEqual([]);
});

for (const [width, height] of SIZES)
  test(`Sessions keeps room for eight rows with three sessions at ${width}x${height}`, async ({
    page,
  }, info) => {
    await open(page, withSessions(3), width, height);
    const g = await layout(page);
    await info.attach('layout', { body: JSON.stringify(g, null, 2) });
    await page.screenshot({ path: info.outputPath(`default-3-sessions-${width}x${height}.png`) });
    // The card is the eight-row height; its three rows sit at the top of it.
    expect(g.sessions).toBeCloseTo(sessionsHeight(8), 0);
    expect(g.lanesArea).toBeCloseTo(HEAD + 8 * ROW - 1, 0);
    expect(g.laneRows).toBe(3);
    // Effort and Overview share the rest at 1.6 to 1, side by side.
    expect(g.effort.top).toBeCloseTo(g.overview.top, 0);
    expect(g.effort.height).toBeCloseTo(g.overview.height, 0);
    expect(g.effort.width / g.overview.width).toBeCloseTo(1.6, 1);
    expect(g.outletScrolls).toBe(false);
    expect(g.pageWidth).toBe(width);
    // Nothing is stored until a boundary is moved.
    expect(await stored(page)).toBeNull();
    for (const handle of [rowsHandle(page), columnsHandle(page)])
      await expect(handle).toHaveAttribute('aria-valuetext', /, default layout$/);
  });

test('more than eight sessions still grow Sessions in a taller window', async ({ page }) => {
  await open(page, withSessions(14), 1120, 720);
  const short = await layout(page);
  expect(short.sessions).toBeGreaterThanOrEqual(sessionsHeight(8) - 0.5);
  await page.setViewportSize({ width: 1440, height: 900 });
  const tall = await layout(page);
  expect(tall.sessions).toBeGreaterThan(short.sessions + 40);
  expect(tall.laneRows).toBeGreaterThan(short.laneRows);
  expect(tall.outletScrolls).toBe(false);
});

for (const [width, height] of SIZES)
  test(`dragging both boundaries resizes, persists and resets at ${width}x${height}`, async ({
    page,
  }, info) => {
    await open(page, withSessions(3), width, height);
    const before = await layout(page);

    // Up 80px: the effort row loses what Sessions gains.
    await drag(page, rowsHandle(page), 0, -80);
    const rows = await layout(page);
    expect(rows.main).toBeCloseTo(before.main - 80, 0);
    expect(rows.sessions).toBeCloseTo(before.sessions + 80, 0);
    expect(rows.laneRows).toBe(3);
    // Left 40px: Effort narrows, Overview widens by the same amount.
    await drag(page, columnsHandle(page), -40, 0);
    const both = await layout(page);
    expect(both.effort.width).toBeCloseTo(before.effort.width - 40, 0);
    expect(both.overview.width).toBeCloseTo(before.overview.width + 40, 0);
    expect(both.main).toBeCloseTo(rows.main, 0);
    expect(both.outletScrolls).toBe(false);
    expect(both.clipped).toEqual([]);
    await page.screenshot({ path: info.outputPath(`dragged-${width}x${height}.png`) });

    const saved = JSON.parse((await stored(page))!) as { rows: number; columns: number };
    expect(saved.rows).toBeCloseTo(both.main / (both.main + both.sessions), 3);
    expect(saved.columns).toBeCloseTo(
      both.effort.width / (both.effort.width + both.overview.width),
      3,
    );

    // A reload, as a restart is, draws the same layout before anything moves.
    await page.reload();
    await expect(page.getByTestId('dashboard-summary')).toBeVisible();
    const reloaded = await layout(page);
    expect(reloaded.main).toBeCloseTo(both.main, 0);
    expect(reloaded.effort.width).toBeCloseTo(both.effort.width, 0);

    // A resized window keeps the proportions.
    const [otherWidth, otherHeight] = width === 1440 ? [1280, 800] : [1440, 900];
    await page.setViewportSize({ width: otherWidth, height: otherHeight });
    const resized = await layout(page);
    expect(resized.main / (resized.main + resized.sessions)).toBeCloseTo(saved.rows, 2);
    expect(resized.effort.width / (resized.effort.width + resized.overview.width)).toBeCloseTo(
      saved.columns,
      2,
    );
    expect(resized.outletScrolls).toBe(false);
    await page.setViewportSize({ width, height });

    // A double-click on each boundary puts its default back and forgets it.
    await columnsHandle(page).dblclick();
    expect(JSON.parse((await stored(page))!)).toEqual({ rows: saved.rows, columns: null });
    await rowsHandle(page).dblclick();
    expect(await stored(page)).toBeNull();
    const reset = await layout(page);
    expect(reset.main).toBeCloseTo(before.main, 0);
    expect(reset.sessions).toBeCloseTo(before.sessions, 0);
    expect(reset.effort.width).toBeCloseTo(before.effort.width, 0);
  });

test('each pane keeps its minimum however far a boundary is dragged', async ({ page }, info) => {
  await open(page, withSessions(3), 1120, 720);
  // Sessions down to its header and three rows.
  await drag(page, rowsHandle(page), 0, 600);
  const low = await layout(page);
  expect(low.sessions).toBeCloseTo(sessionsHeight(3), 0);
  expect(low.laneRows).toBe(3);
  expect(low.outletScrolls).toBe(false);
  await expect(rowsHandle(page)).toHaveAttribute(
    'aria-valuenow',
    (await rowsHandle(page).getAttribute('aria-valuemax')) ?? '',
  );
  // The effort row down to its own minimum: its headers and a 50px plot.
  await drag(page, rowsHandle(page), 0, -900);
  const high = await layout(page);
  expect(high.effortPlot).toBeGreaterThanOrEqual(50);
  expect(high.overviewValues).toBe(4);
  expect(high.headerHeights).toEqual({ effort: 36, overview: 36 });
  expect(high.outletScrolls).toBe(false);
  expect(high.main).toBeLessThan(low.main);
  await page.screenshot({ path: info.outputPath('minimum-effort-row-1120x720.png') });

  // Effort and Overview each down to their minimum width, headers on one line.
  await drag(page, columnsHandle(page), -900, 0);
  const narrowEffort = await layout(page);
  expect(narrowEffort.effort.width).toBeCloseTo(EFFORT_MIN, 0);
  expect(narrowEffort.headerHeights.effort).toBe(36);
  expect(narrowEffort.clipped).toEqual([]);
  await page.screenshot({ path: info.outputPath('minimum-effort-1120x720.png') });
  await drag(page, columnsHandle(page), 1200, 0);
  const narrowOverview = await layout(page);
  expect(narrowOverview.overview.width).toBeCloseTo(OVERVIEW_MIN, 0);
  expect(narrowOverview.headerHeights.overview).toBe(36);
  expect(narrowOverview.overviewValues).toBe(4);
  expect(narrowOverview.clipped).toEqual([]);
  expect(narrowOverview.pageWidth).toBe(1120);
  await page.screenshot({ path: info.outputPath('minimum-overview-1120x720.png') });
});

test('the boundaries move from the keyboard and say where they are', async ({ page }, info) => {
  await open(page, withSessions(3), 1440, 900);
  const rows = rowsHandle(page);
  await expect(rows).toHaveAttribute('aria-orientation', 'horizontal');
  await expect(rows).toHaveAttribute('tabindex', '0');
  const value = async (handle: Locator, name: string) =>
    Number(await handle.getAttribute(`aria-${name}`));
  const before = await layout(page);
  const now = await value(rows, 'valuenow');
  expect(now).toBe(Math.round((before.main / (before.main + before.sessions)) * 100));
  expect(await value(rows, 'valuemin')).toBeLessThan(now);
  expect(await value(rows, 'valuemax')).toBeGreaterThan(now);

  // Focused, each arrow moves it 16px, Shift 64px, and every move is
  // stored at once.
  await rows.focus();
  await page.keyboard.press('ArrowUp');
  expect((await layout(page)).main).toBeCloseTo(before.main - 16, 0);
  await page.keyboard.press('Shift+ArrowDown');
  expect((await layout(page)).main).toBeCloseTo(before.main + 48, 0);
  expect(await stored(page)).not.toBeNull();
  await page.keyboard.press('Home');
  await expect(rows).toHaveAttribute('aria-valuenow', String(await value(rows, 'valuemin')));
  await page.keyboard.press('End');
  await expect(rows).toHaveAttribute('aria-valuenow', String(await value(rows, 'valuemax')));
  expect((await layout(page)).sessions).toBeCloseTo(sessionsHeight(3), 0);
  await expect(rows).toHaveAttribute('aria-valuetext', /^Effort and Overview \d+%, Sessions \d+%$/);
  await page.keyboard.press('Enter');
  await expect(rows).toHaveAttribute('aria-valuetext', /, default layout$/);
  expect((await layout(page)).main).toBeCloseTo(before.main, 0);
  expect(await stored(page)).toBeNull();

  const columns = columnsHandle(page);
  await expect(columns).toHaveAttribute('aria-orientation', 'vertical');
  await columns.focus();
  // Up and Down do nothing to a vertical boundary.
  await page.keyboard.press('ArrowDown');
  expect(await stored(page)).toBeNull();
  await page.keyboard.press('ArrowLeft');
  expect((await layout(page)).effort.width).toBeCloseTo(before.effort.width - 16, 0);
  await page.keyboard.press('Home');
  expect((await layout(page)).effort.width).toBeCloseTo(EFFORT_MIN, 0);
  await page.keyboard.press('End');
  expect((await layout(page)).overview.width).toBeCloseTo(OVERVIEW_MIN, 0);
  await page.keyboard.press('Enter');
  expect((await layout(page)).effort.width).toBeCloseTo(before.effort.width, 0);
  expect(await stored(page)).toBeNull();
  // A visible focus line on the focused boundary, reached by Tab from the
  // control before it.
  await page
    .locator('.xt-effort-card')
    .locator('button:visible, a[href]:visible, [tabindex="0"]:visible')
    .last()
    .focus();
  await page.keyboard.press('Tab');
  await expect(columns).toBeFocused();
  await expect
    .poll(() => columns.evaluate((node) => getComputedStyle(node, '::after').opacity))
    .toBe('1');
  await page.screenshot({ path: info.outputPath('focused-column-boundary-1440x900.png') });
});

test('one listed session still keeps room for three rows at the smallest Sessions', async ({
  page,
}) => {
  await open(page, withSessions(1), 1120, 720);
  expect((await layout(page)).sessions).toBeCloseTo(sessionsHeight(8), 0);
  await drag(page, rowsHandle(page), 0, 600);
  const low = await layout(page);
  expect(low.sessions).toBeCloseTo(sessionsHeight(3), 0);
  expect(low.lanesArea).toBeCloseTo(HEAD + 3 * ROW - 1, 0);
  expect(low.laneRows).toBe(1);
});

test('unreadable stored layout draws the default', async ({ page }) => {
  await page.addInitScript(
    (key) => window.localStorage.setItem(key, '{"rows": 7, "columns": "x"'),
    STORAGE,
  );
  await open(page, withSessions(3), 1440, 900);
  const g = await layout(page);
  expect(g.sessions).toBeCloseTo(sessionsHeight(8), 0);
  expect(g.effort.width / g.overview.width).toBeCloseTo(1.6, 1);
});

/**
 * Narrow enough that Effort stacks over Overview: there is no column
 * boundary, and the row boundary still resizes the stacked pair against
 * Sessions. A stored column share is kept for when the window widens again.
 */
test('the stacked layout has only the row boundary', async ({ page }, info) => {
  await open(page, withSessions(3), 1440, 900);
  await drag(page, columnsHandle(page), -60, 0);
  const wide = await layout(page);
  const saved = await stored(page);
  await page.setViewportSize({ width: 860, height: 900 });
  await expect(columnsHandle(page)).toBeHidden();
  const stacked = await layout(page);
  expect(stacked.overview.top).toBeGreaterThanOrEqual(stacked.effort.top + stacked.effort.height);
  expect(stacked.overview.width).toBeCloseTo(stacked.effort.width, 0);
  expect(stacked.pageWidth).toBe(860);
  await page.screenshot({ path: info.outputPath('stacked-860x900.png') });
  await page.setViewportSize({ width: 860, height: 720 });
  await page.screenshot({ path: info.outputPath('stacked-860x720.png') });
  await page.setViewportSize({ width: 860, height: 900 });
  // The stacked pair is at its own minimum here, so the boundary moves down.
  await drag(page, rowsHandle(page), 0, 40);
  const moved = await layout(page);
  expect(moved.sessions).toBeCloseTo(stacked.sessions - 40, 0);
  expect(moved.main).toBeCloseTo(stacked.main + 40, 0);
  await page.screenshot({ path: info.outputPath('stacked-dragged-860x900.png') });
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(columnsHandle(page)).toBeVisible();
  expect((await layout(page)).effort.width).toBeCloseTo(wide.effort.width, 0);
  expect(JSON.parse((await stored(page))!).columns).toBe(JSON.parse(saved!).columns);
});

/**
 * Below the supported height, Sessions gives way from its eight-row start to
 * its three-row minimum before the page scrolls, whether it lists three
 * sessions or fourteen.
 */
for (const count of [3, 14])
  for (const [width, height] of [
    [1120, 600],
    [896, 576],
    [860, 720],
  ] as const)
    test(`a short window shrinks Sessions toward three rows before scrolling, ${count} sessions at ${width}x${height}`, async ({
      page,
    }) => {
      await open(page, withSessions(count), width, height);
      const g = await layout(page);
      expect(g.sessions).toBeGreaterThanOrEqual(sessionsHeight(3) - 0.5);
      expect(g.sessions).toBeLessThan(sessionsHeight(8) - 0.5);
      expect(g.laneRows).toBeGreaterThanOrEqual(3);
      // The page scrolls only once Sessions is down to its minimum.
      if (g.outletScrolls) expect(g.sessions).toBeCloseTo(sessionsHeight(3), 0);
    });

test('a drag cancelled by Escape or a cancelled pointer puts the layout back', async ({ page }) => {
  await open(page, withSessions(3), 1440, 900);
  await drag(page, rowsHandle(page), 0, -60);
  const before = await layout(page);
  const saved = await stored(page);
  for (const cancel of ['Escape', 'pointercancel'] as const) {
    const handle = rowsHandle(page);
    const b = (await handle.boundingBox())!;
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2 - 100, { steps: 4 });
    expect((await layout(page)).main).toBeCloseTo(before.main - 100, 0);
    if (cancel === 'Escape') await page.keyboard.press('Escape');
    else await handle.dispatchEvent('pointercancel', { pointerId: 1, isPrimary: true });
    await page.mouse.up();
    const after = await layout(page);
    expect(after.main, cancel).toBeCloseTo(before.main, 0);
    expect(after.sessions, cancel).toBeCloseTo(before.sessions, 0);
    expect(await stored(page), cancel).toBe(saved);
    await expect(handle).not.toHaveAttribute('data-dragging');
  }
  // From the default layout, a cancelled drag leaves it default and unstored.
  await rowsHandle(page).dblclick();
  const fresh = await layout(page);
  const b = (await rowsHandle(page).boundingBox())!;
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2 + 50, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  expect((await layout(page)).sessions).toBeCloseTo(fresh.sessions, 0);
  expect(await stored(page)).toBeNull();
  await expect(rowsHandle(page)).toHaveAttribute('aria-valuetext', /, default layout$/);
});

/**
 * The spoken range follows what the panes hold and the window's size: a
 * stored layout pinned at both minimums reads as at its minimum once
 * Overview's tiles have arrived, and a window resized while a boundary has
 * focus re-measures its range.
 */
test('the spoken range is re-measured after loading and after a resize', async ({ page }) => {
  await page.addInitScript(
    (key) => window.localStorage.setItem(key, '{"rows": 0.03, "columns": 0.03}'),
    STORAGE,
  );
  await open(page, withSessions(3), 1120, 720);
  const read = async (handle: Locator) =>
    Object.fromEntries(
      await Promise.all(
        ['valuenow', 'valuemin', 'valuemax'].map(
          async (name) => [name, Number(await handle.getAttribute(`aria-${name}`))] as const,
        ),
      ),
    ) as Record<'valuenow' | 'valuemin' | 'valuemax', number>;
  for (const handle of [rowsHandle(page), columnsHandle(page)])
    await expect
      .poll(async () => {
        const v = await read(handle);
        return v.valuenow === v.valuemin && v.valuemin < v.valuemax;
      })
      .toBe(true);

  // Reset the rows, drag at 1440x900, keep the focus, and resize.
  await rowsHandle(page).dblclick();
  await page.setViewportSize({ width: 1440, height: 900 });
  await drag(page, rowsHandle(page), 0, -40);
  await expect(rowsHandle(page)).toBeFocused();
  await page.setViewportSize({ width: 1120, height: 720 });
  const g = await layout(page);
  const expected = Math.round((g.main / (g.main + g.sessions)) * 100);
  // What Home and End reach is what the range says.
  await expect.poll(async () => (await read(rowsHandle(page))).valuenow).toBe(expected);
  await page.waitForTimeout(400);
  const settled = await read(rowsHandle(page));
  await page.keyboard.press('Home');
  expect((await read(rowsHandle(page))).valuenow).toBe(settled.valuemin);
  await page.keyboard.press('End');
  expect((await read(rowsHandle(page))).valuenow).toBe(settled.valuemax);
});

test('each boundary shows a faint grip before it is hovered', async ({ page }) => {
  await open(page, withSessions(3), 1440, 900);
  await page.mouse.move(5, 5);
  for (const handle of [rowsHandle(page), columnsHandle(page)]) {
    const [content, grip, line] = await handle.evaluate((node) => [
      getComputedStyle(node, '::before').content,
      getComputedStyle(node, '::before').opacity,
      getComputedStyle(node, '::after').opacity,
    ]);
    // Drawn, and fainter than the hover line.
    expect(content).not.toBe('none');
    expect(Number(grip)).toBeGreaterThan(0);
    expect(Number(grip)).toBeLessThan(1);
    expect(line).toBe('0');
  }
});

test('measuring a boundary keeps every list where it was scrolled', async ({ page }) => {
  await open(page, withSessions(14), 1120, 720);
  const lanes = page.getByRole('region', { name: 'Session lanes scroll area' });
  await lanes.evaluate((node) => (node.scrollTop = node.scrollHeight));
  const scrolled = await lanes.evaluate((node) => node.scrollTop);
  expect(scrolled).toBeGreaterThan(0);
  // Focusing and moving a boundary measures how far each pane can shrink.
  await rowsHandle(page).focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowUp');
  await page.waitForTimeout(300);
  expect(await lanes.evaluate((node) => node.scrollTop)).toBeCloseTo(scrolled, 0);
});
