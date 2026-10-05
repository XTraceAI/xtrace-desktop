import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardLaneSession } from '../src/data/generated/DashboardLaneSession';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionParentLink } from '../src/data/generated/SessionParentLink';
import type { SessionRow } from '../src/data/generated/SessionRow';

/**
 * Verified sub-sessions in a real engine, over synthetic sessions (not real
 * history) served in place of the development F1 export: a child with a long
 * saved title and a long repository whose parent has a long saved title, the
 * parent itself, and an ordinary session. The child names its parent inside
 * its own name cell; every row keeps its height, the lane track and cost
 * keep exactly the room they have without the relation, and the Dashboard
 * still fits the supported minimum window without scrolling.
 */
const PARENT = '01a0aaaa-0000-7000-8000-000000000001';
const CHILD = '01a0bbbb-0000-7000-8000-000000000002';
const OTHER = '01a0cccc-0000-7000-8000-000000000003';
const PARENT_TITLE = 'A saved parent title long enough to need its own ellipsis in a row';
const CHILD_TITLE = 'A saved sub-session title that is also far too long for a lane row';
const LONG_REPO = '/Users/developer/code/organisation/a-very-long-repository-name-for-layout';

function exportWith(related: boolean): FixtureExport {
  const out = structuredClone(fixture as FixtureExport);
  const parent: SessionParentLink | null = related
    ? { session_id: PARENT, host: 'codex', title: PARENT_TITLE, evidence: 'native_spawn' }
    : null;
  for (const report of out.dashboards) {
    const minute = 60_000;
    const span = (session_id: string, offset: number, length: number) => ({
      session_id,
      host: 'codex',
      start_ms: report.lane_start_ms + offset * minute,
      end_ms: report.lane_start_ms + (offset + length) * minute,
    });
    const context = (
      session_id: string,
      extra: Partial<DashboardLaneSession>,
    ): DashboardLaneSession => ({
      session_id,
      host: 'codex',
      repo: LONG_REPO,
      branch: 'feature/a-long-branch-name-for-layout-checks',
      title: null,
      started_at_ms: report.lane_start_ms + 60 * minute,
      pr_links: 3,
      inferred_pr_links: 1,
      // The widest amount the cost column is sized for: whole dollars, partial.
      cost: {
        total_usd: null,
        priced_subtotal_usd: 1_234.4,
        selected_observations: 9,
        priced_observations: 8,
        unpriced_observations: 1,
        assumed_tier_observations: 0,
        unpriced: [],
      },
      parent: null,
      ...extra,
      automated_review: extra.automated_review ?? false,
    });
    report.lanes = [span(CHILD, 2400, 40), span(OTHER, 1800, 90), span(PARENT, 1200, 1300)];
    report.lanes_total = 3;
    report.lanes_truncated = false;
    report.lane_sessions = [
      context(CHILD, { title: CHILD_TITLE, parent }),
      context(OTHER, {
        repo: '/code/acme-api',
        branch: 'main',
        cost: {
          total_usd: 0,
          priced_subtotal_usd: 0,
          selected_observations: 1,
          priced_observations: 1,
          unpriced_observations: 0,
          assumed_tier_observations: 0,
          unpriced: [],
        },
      }),
      context(PARENT, { title: PARENT_TITLE }),
    ];
  }
  for (const page of out.sessions) {
    const template = page.rows[0];
    const row = (id: string, extra: Partial<SessionRow>): SessionRow => ({
      ...structuredClone(template),
      id,
      host: 'codex',
      repo: LONG_REPO,
      branch: 'feature/a-long-branch-name-for-layout-checks',
      parent: null,
      ...extra,
    });
    page.rows = [
      row(CHILD, { title: CHILD_TITLE, parent }),
      row(OTHER, { repo: '/code/acme-api', branch: 'main' }),
      row(PARENT, { title: PARENT_TITLE }),
    ];
  }
  return out;
}

async function open(page: Page, related: boolean, path: string, width: number, height: number) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(exportWith(related))};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.goto(path);
  await page.evaluate(() => document.fonts.ready);
}

/** Row, cell and page geometry, and whether each visible piece fits its box. */
const measure = (page: Page, table: string) =>
  page.evaluate((label) => {
    const grid = document.querySelector<HTMLElement>(`[role="table"][aria-label="${label}"]`)!;
    const fits = (element: HTMLElement | null) =>
      element ? element.scrollWidth <= element.clientWidth + 1 : null;
    const width = (element: HTMLElement | null) => element?.getBoundingClientRect().width ?? 0;
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    const rows = [...grid.querySelectorAll<HTMLElement>('.xt-data-row')].map((row) => {
      const box = row.getBoundingClientRect();
      const cells = [...row.querySelectorAll<HTMLElement>('[role="cell"]')];
      const marker = row.querySelector<HTMLElement>('.xt-subsession');
      const markerBox = marker?.getBoundingClientRect();
      const nameCell = marker?.closest<HTMLElement>('[role="cell"]')?.getBoundingClientRect();
      return {
        id: row.querySelector<HTMLElement>('[data-visible-id]')?.dataset.visibleId ?? null,
        height: box.height,
        cells: cells.map((cell) => Math.round(cell.getBoundingClientRect().width)),
        name: width(row.querySelector('.xt-lane-find, .xt-session-open')),
        context: width(row.querySelector('.xt-lane-repo, .xt-session-meta')),
        marker: marker
          ? {
              text: marker.textContent,
              // What is drawn: the words give way in a narrow cell.
              words:
                getComputedStyle(marker.querySelector('.xt-subsession-words')!).display !== 'none',
              container: width(marker.closest<HTMLElement>('.xt-lane-name, .xt-session-line')),
              // A lane's name keeps half its cell, so its words need more room.
              wordsFrom: marker.closest('.xt-lane-name') ? 400 : 320,
              width: markerBox!.width,
              parent: width(marker.querySelector<HTMLElement>('.xt-subsession-parent')),
              inside:
                markerBox!.left >= nameCell!.left - 0.5 &&
                markerBox!.right <= nameCell!.right + 0.5 &&
                markerBox!.top >= box.top - 0.5 &&
                markerBox!.bottom <= box.bottom + 0.5,
            }
          : null,
        outputFits: fits(row.querySelector<HTMLElement>('.xt-metric-cell')),
        track: width(row.querySelector('.xt-lane-track')),
      };
    });
    return {
      rows,
      page: {
        doc: [document.documentElement.scrollHeight, document.documentElement.clientHeight],
        outlet: [outlet.scrollHeight, outlet.clientHeight, outlet.scrollWidth, outlet.clientWidth],
      },
    };
  }, table);

/**
 * The marker sits inside its row's cell and leaves the parent's name real
 * room; its words show exactly when its cell is wide enough for both, and
 * the whole phrase is always in the DOM for its name and tooltip.
 */
function expectMarker(marker: {
  text: string | null;
  words: boolean;
  container: number;
  wordsFrom: number;
  width: number;
  parent: number;
  inside: boolean;
}) {
  expect(marker.inside).toBe(true);
  expect(marker.text).toBe(`↳Sub-session of ${PARENT_TITLE}`);
  expect(marker.words).toBe(marker.container >= marker.wordsFrom);
  expect(marker.parent).toBeGreaterThan(50);
}

const SIZES = [
  [1120, 720],
  [1440, 900],
] as const;

for (const [width, height] of SIZES)
  test(`the Dashboard names a lane's parent without moving its track or cost at ${width}x${height}`, async ({
    page,
  }, info) => {
    await open(page, false, '/dashboard', width, height);
    await expect(page.getByRole('table', { name: 'Session lanes' }).getByRole('row')).toHaveCount(
      4,
    );
    const plain = await measure(page, 'Session lanes');
    await page.unrouteAll();
    await open(page, true, '/dashboard', width, height);
    const marker = page.getByRole('link', {
      name: `Sub-session of ${PARENT_TITLE}, open parent session ${PARENT}`,
    });
    // The child is listed under its parent, behind the parent row's
    // disclosure; opened, it is its own row with the marker.
    await expect(marker).toHaveCount(0);
    await page.getByRole('button', { name: `1 returned sub-session of ${PARENT_TITLE}` }).click();
    await expect(marker).toBeVisible();
    const related = await measure(page, 'Session lanes');
    await info.attach('lanes', { body: JSON.stringify({ plain, related }, null, 2) });
    await page.locator('.xt-lanes').screenshot({ path: info.outputPath(`lanes-${width}.png`) });

    // The same rows — the group at its newest member's place, the child under
    // its parent — each one 22px line with every cell where it was.
    expect(plain.rows.map((row) => row.id)).toEqual([CHILD, OTHER, PARENT]);
    expect(related.rows.map((row) => row.id)).toEqual([PARENT, CHILD, OTHER]);
    for (const row of related.rows) {
      const before = plain.rows.find((candidate) => candidate.id === row.id)!;
      expect(row.height).toBeCloseTo(22, 0);
      expect(row.cells).toEqual(before.cells);
      expect(row.track).toBeCloseTo(before.track, 0);
      expect(row.outputFits).toBe(true);
    }
    // Only the child carries the marker, inside its own name cell and row,
    // with room for its words; the child's own name keeps a real share.
    expect(related.rows.map((row) => row.marker !== null)).toEqual([false, true, false]);
    const child = related.rows[1];
    expectMarker(child.marker!);
    expect(child.name).toBeGreaterThan(60);
    // No page scroll at the supported minimum or above.
    expect(related.page.doc[0]).toBeLessThanOrEqual(related.page.doc[1]);
    const [scrollH, clientH, scrollW, clientW] = related.page.outlet;
    expect(scrollH).toBeLessThanOrEqual(clientH + 1);
    expect(scrollW).toBeLessThanOrEqual(clientW + 1);
    expect(related.page.outlet).toEqual(plain.page.outlet);

    // Keyboard: the marker takes focus in the row and opens the exact parent
    // with a Back that finds it.
    await marker.focus();
    await expect(marker).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(
      new RegExp(`/sessions/${PARENT}\\?q=${PARENT}&host=codex&range=7d$`),
    );
    await expect(page.getByRole('link', { name: '← All sessions' })).toHaveAttribute(
      'href',
      `/sessions?q=${PARENT}&host=codex&range=7d`,
    );
  });

for (const [width, height] of SIZES)
  test(`Sessions names a row’s parent inside its 40px row and returns to the same list at ${width}x${height}`, async ({
    page,
  }, info) => {
    await open(page, true, '/sessions?q=01a0&host=codex&range=14d', width, height);
    const table = page.getByRole('table', { name: 'Indexed sessions' });
    await expect(table.getByRole('link', { name: /^Open session / })).toHaveCount(3);
    const measured = await measure(page, 'Indexed sessions');
    await info.attach('sessions', { body: JSON.stringify(measured, null, 2) });
    await page
      .locator('.xt-sessions')
      .screenshot({ path: info.outputPath(`sessions-${width}.png`) });
    expect(measured.rows.map((row) => row.height)).toEqual([40, 40, 40]);
    expect(measured.rows.map((row) => row.marker !== null)).toEqual([true, false, false]);
    expectMarker(measured.rows[0].marker!);
    // The row's own context still has room beside the parent.
    expect(measured.rows[0].context).toBeGreaterThan(40);
    expect(measured.page.doc[0]).toBeLessThanOrEqual(measured.page.doc[1]);

    const marker = table.getByRole('link', {
      name: `Sub-session of ${PARENT_TITLE}, open parent session ${PARENT}`,
    });
    await marker.focus();
    await expect(marker).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(new RegExp(`/sessions/${PARENT}\\?q=01a0&host=codex&range=14d$`));
    await page.getByRole('link', { name: '← All sessions' }).click();
    await expect(page).toHaveURL(/\/sessions\?q=01a0&host=codex&range=14d$/);
    await expect(page.getByLabel('Search sessions')).toHaveValue('01a0');
    await expect(marker).toBeVisible();
  });
