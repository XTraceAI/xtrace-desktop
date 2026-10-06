import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardLaneSession } from '../src/data/generated/DashboardLaneSession';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionParentLink } from '../src/data/generated/SessionParentLink';

/**
 * Verified sub-sessions grouped in the Dashboard's compact Sessions table, over
 * synthetic sessions (not real history) served in place of the development F1
 * export: a capped report whose newest session is a child of a parent with no
 * span in it, a returned parent with three returned children, a session whose
 * parent is unknown, and enough ordinary sessions to scroll. Groups open from
 * the keyboard, their children are real rows with real links only while open,
 * and opening them grows the table up to the card's share of the window and
 * scrolls it beyond that, never the page.
 */
const id = (tag: string) => `01a0${tag}-0000-7000-8000-000000000000`;
const GONE = id('eeee');
const PARENT = id('aaaa');
const KID_A = id('ka01');
const KID_B = id('kb02');
const CHILD_1 = id('c101');
const CHILD_2 = id('c202');
const CHILD_3 = id('c303');
const UNKNOWN = id('uuuu');
const PARENT_TITLE = 'A saved parent title long enough to need its own ellipsis in a row';
const LONG_REPO = '/Users/developer/code/organisation/a-very-long-repository-name-for-layout';
const ORDINARY = Array.from({ length: 8 }, (_, index) => id(`o${index}0${index}`));

// Newest first, as the report returns them; `ordinary` shortens the list.
const orderWith = (ordinary: number) => [
  KID_A,
  ORDINARY[0],
  PARENT,
  CHILD_1,
  UNKNOWN,
  KID_B,
  CHILD_2,
  CHILD_3,
  ...ORDINARY.slice(1, ordinary),
];

function exportWith(reviewer = false, ordinary = ORDINARY.length): FixtureExport {
  const ORDER = orderWith(ordinary);
  const out = structuredClone(fixture as FixtureExport);
  const link = (session_id: string, title: string | null): SessionParentLink => ({
    session_id,
    host: 'codex',
    title,
    evidence: 'native_spawn',
  });
  // A parent with no span returned and no saved title is named by its identity.
  const parentOf: Record<string, SessionParentLink | null> = {
    [KID_A]: { ...link(GONE, null), evidence: reviewer ? 'native_reviewer' : 'native_spawn' },
    [KID_B]: link(GONE, null),
    [CHILD_1]: link(PARENT, PARENT_TITLE),
    [CHILD_2]: link(PARENT, PARENT_TITLE),
    [CHILD_3]: link(PARENT, PARENT_TITLE),
  };
  for (const report of out.dashboards) {
    const minute = 60_000;
    report.lanes = ORDER.map((session_id, index) => ({
      session_id,
      host: 'codex',
      start_ms: report.lane_end_ms - (index + 1) * 90 * minute,
      end_ms: report.lane_end_ms - (index + 1) * 90 * minute + 45 * minute,
    }));
    // Capped: the report returned the most recent spans of more.
    report.lanes_total = 240;
    report.lanes_truncated = true;
    report.lane_sessions = ORDER.map((session_id, index): DashboardLaneSession => ({
      session_id,
      host: 'codex',
      repo: LONG_REPO,
      branch: 'feature/a-long-branch-name-for-layout-checks',
      title:
        session_id === PARENT
          ? PARENT_TITLE
          : session_id === CHILD_3
            ? 'A saved sub-session title that is also far too long for a lane row'
            : null,
      automated_review: reviewer && session_id === KID_A,
      started_at_ms: report.lane_start_ms + index * 60 * minute,
      pr_links: index,
      inferred_pr_links: 0,
      cost: {
        total_usd: index + 1,
        priced_subtotal_usd: index + 1,
        selected_observations: 1,
        priced_observations: 1,
        unpriced_observations: 0,
        assumed_tier_observations: 0,
        unpriced: [],
      },
      parent: parentOf[session_id] ?? null,
    }));
  }
  return out;
}

async function open(
  page: Page,
  width: number,
  height: number,
  scheme: 'light' | 'dark',
  reviewer = false,
  ordinary = ORDINARY.length,
) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(exportWith(reviewer, ordinary))};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await page.evaluate(() => document.fonts.ready);
}

/** The lane table's drawn rows, their fit, and the page's scroll state. */
const measure = (page: Page) =>
  page.evaluate(() => {
    const scroll = document.querySelector<HTMLElement>('.xt-lanes .xt-table-scroll')!;
    const area = scroll.getBoundingClientRect();
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    const panel = scroll.closest('.xt-section-panel')!.getBoundingClientRect();
    const card = scroll.closest('section')!.getBoundingClientRect();
    const rows = [...scroll.querySelectorAll<HTMLElement>('.xt-data-row')].map((row) => {
      const box = row.getBoundingClientRect();
      const cells = [...row.querySelectorAll<HTMLElement>('[role="cell"]')];
      const toggle = row.querySelector<HTMLElement>('.xt-lane-toggle');
      const toggleBox = toggle?.getBoundingClientRect();
      const nameCell = cells[1].getBoundingClientRect();
      return {
        // What is drawn: the row's description is for assistive technology.
        name: [...row.querySelector('.xt-lane-name')!.childNodes]
          .filter((node) => !(node instanceof HTMLElement && node.classList.contains('sr-only')))
          .map((node) => node.textContent)
          .join(''),
        id: row.querySelector<HTMLElement>('[data-visible-id]')?.dataset.visibleId ?? null,
        height: box.height,
        cells: cells.map((cell) => Math.round(cell.getBoundingClientRect().width)),
        // Blank but for its cost: no lane, link count, start or compactions.
        blank: cells.slice(2, -1).every((cell) => cell.textContent === ''),
        cost: cells.at(-1)!.textContent,
        // The context's drawn width, and the name link's: the absent parent's
        // name ends in an ellipsis inside its cell rather than past it.
        context: row.querySelector('.xt-lane-repo')?.getBoundingClientRect().width ?? null,
        nameInside:
          (row.querySelector('.xt-lane-find')?.getBoundingClientRect().right ?? 0) <=
          nameCell.right + 0.5,
        inside: cells.every(
          (cell) =>
            cell.getBoundingClientRect().left >= box.left - 0.5 &&
            cell.getBoundingClientRect().right <= box.right + 0.5,
        ),
        toggle: toggleBox
          ? {
              expanded: toggle!.getAttribute('aria-expanded'),
              inside:
                toggleBox.left >= nameCell.left - 0.5 &&
                toggleBox.right <= nameCell.right + 0.5 &&
                toggleBox.top >= box.top - 0.5 &&
                toggleBox.bottom <= box.bottom + 0.5,
            }
          : null,
        whollyVisible: box.top >= area.top - 0.5 && box.bottom <= area.bottom + 0.5,
      };
    });
    return {
      rows,
      visible: rows.filter((row) => row.whollyVisible).length,
      table: {
        height: area.height,
        scrollHeight: scroll.scrollHeight,
        clientHeight: scroll.clientHeight,
        horizontal: scroll.scrollWidth - scroll.clientWidth,
      },
      // No caption follows the rows, and the panel ends under them.
      caption: document.querySelectorAll('[data-testid="lanes-disclosure"]').length,
      blank: panel.bottom - area.bottom,
      // From the card's bottom to the window's: the page's 8px padding.
      below: outlet.getBoundingClientRect().bottom - card.bottom,
      card: card.height,
      page: {
        doc: [document.documentElement.scrollHeight, window.innerHeight],
        outlet: [outlet.scrollHeight, outlet.clientHeight, outlet.scrollWidth, outlet.clientWidth],
        tops: [window.scrollY, outlet.scrollTop],
      },
    };
  });
type Geometry = Awaited<ReturnType<typeof measure>>;

function expectFits(g: Geometry) {
  const [docScroll, docClient] = g.page.doc;
  expect(docScroll).toBeLessThanOrEqual(docClient);
  const [scrollH, clientH, scrollW, clientW] = g.page.outlet;
  expect(scrollH).toBeLessThanOrEqual(clientH + 1);
  expect(scrollW).toBeLessThanOrEqual(clientW + 1);
  expect(g.page.tops).toEqual([0, 0]);
  expect(g.table.horizontal).toBeLessThanOrEqual(1);
  expect(g.visible).toBeGreaterThanOrEqual(3);
  expect(g.caption).toBe(0);
  expect(g.blank).toBeLessThanOrEqual(3.5);
  for (const row of g.rows) {
    expect(row.height, row.name!).toBeCloseTo(22, 0);
    expect(row.inside, row.name!).toBe(true);
    expect(row.cells, row.name!).toEqual(g.rows[0].cells);
    if (row.toggle) expect(row.toggle.inside, row.name!).toBe(true);
    expect(row.nameInside, row.name!).toBe(true);
    // A parent keeps a readable share of its context beside its disclosure.
    // (A sub-session gives its context up to its parent marker, as it
    // does ungrouped.)
    if (row.toggle && row.context !== null) expect(row.context, row.name!).toBeGreaterThan(30);
  }
}

for (const [width, height] of [
  [1120, 720],
  [1440, 900],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`sub-sessions group under their parent and open from the keyboard at ${width}x${height} ${scheme}`, async ({
      page,
      browserName,
    }, info) => {
      await open(page, width, height, scheme);
      const table = page.getByRole('table', { name: 'Session lanes' });
      const absent = page.getByRole('button', {
        name: '2 sub-sessions of Session 01a0eeee',
      });
      const present = page.getByRole('button', {
        name: `3 sub-sessions of ${PARENT_TITLE}`,
      });
      await expect(absent).toHaveAttribute('aria-expanded', 'false');
      await expect(present).toHaveAttribute('aria-expanded', 'false');

      // Collapsed: every returned session but the five children has its own
      // row; the absent parent has one named row; no child link is in the page.
      const closed = await measure(page);
      await info.attach('closed', { body: JSON.stringify(closed, null, 2) });
      await page
        .locator('.xt-lanes')
        .screenshot({ path: info.outputPath(`groups-closed-${width}-${scheme}.png`) });
      expectFits(closed);
      expect(closed.rows.map((row) => row.id)).toEqual([
        null,
        ORDINARY[0],
        PARENT,
        UNKNOWN,
        ...ORDINARY.slice(1),
      ]);
      expect(closed.rows[0].name).toBe('›2Sub-sessions of Session 01a0eeee');
      expect(closed.rows[0].blank).toBe(true);
      // Collapsed groups show the sum of the rows opening them lists: the two
      // sub-sessions' $1 and $6, and the parent's $3 with its $4, $7 and $8.
      expect(closed.rows[0].cost).toBe('Σ $7.00');
      expect(closed.rows[2].cost).toBe('Σ $22.00');
      expect(closed.rows.filter((row) => row.blank)).toHaveLength(1);
      for (const child of [KID_A, KID_B, CHILD_1, CHILD_2, CHILD_3])
        await expect(table.locator(`a[href*="${child}"]`)).toHaveCount(0);
      // The unknown parent stays an ordinary row, with no disclosure.
      await expect(table.getByRole('button', { name: /sub-session/ })).toHaveCount(2);

      // Keyboard: from the last header control, Tab reaches the first group's
      // disclosure with a visible ring; Enter opens it, and Tab walks into the
      // children it listed, each by its own link and its parent marker.
      await table.getByRole('button', { name: /^Whole-session cost/ }).focus();
      await page.keyboard.press('Tab');
      await expect(absent).toBeFocused();
      const ring = await absent.evaluate((element) => {
        const style = getComputedStyle(element);
        return [style.outlineStyle, style.outlineWidth];
      });
      expect(ring).toEqual(['solid', '2px']);
      await page.keyboard.press('Enter');
      await expect(absent).toHaveAttribute('aria-expanded', 'true');
      await expect(absent).toBeFocused();
      const listed = [
        `Open parent session Session 01a0eeee, ${GONE}`,
        `Open session ${KID_A}`,
        `Sub-session of Session 01a0eeee, open parent session ${GONE}`,
        `Open session ${KID_B}`,
        `Sub-session of Session 01a0eeee, open parent session ${GONE}`,
      ];
      const tabs: (string | null)[] = [];
      for (let step = 0; step < (browserName === 'webkit' ? 1 : 5); step += 1) {
        await page.keyboard.press('Tab');
        tabs.push(await page.evaluate(() => document.activeElement!.getAttribute('aria-label')));
      }
      if (browserName === 'webkit') {
        // WebKit, like Safari and the app's own web view by default, leaves
        // links out of the Tab order — every lane link, grouped or not — so
        // Tab moves to the next disclosure, and each listed link still takes
        // focus.
        expect(tabs).toEqual([`3 sub-sessions of ${PARENT_TITLE}`]);
        for (const [index, name] of listed.entries()) {
          const link = table.getByRole('link', { name }).nth(index === 4 ? 1 : 0);
          await link.focus();
          await expect(link).toBeFocused();
        }
      } else expect(tabs).toEqual(listed);

      // Space opens the returned parent's group; its children follow it in
      // recency order, each its own row with its own values.
      await present.focus();
      await page.keyboard.press('Space');
      await expect(present).toHaveAttribute('aria-expanded', 'true');
      const opened = await measure(page);
      await info.attach('opened', { body: JSON.stringify(opened, null, 2) });
      expectFits(opened);
      expect(opened.rows.map((row) => row.id)).toEqual([
        null,
        KID_A,
        KID_B,
        ORDINARY[0],
        PARENT,
        CHILD_1,
        CHILD_2,
        CHILD_3,
        UNKNOWN,
        ...ORDINARY.slice(1),
      ]);
      expect(opened.rows.filter((row) => row.blank)).toHaveLength(1);
      // A child's own link, as the report sent it (its cost: the test below).
      const child = table
        .getByRole('row')
        .filter({ has: page.locator(`[data-visible-id="${CHILD_2}"]`) });
      await expect(child.getByRole('link', { name: `Open session ${CHILD_2}` })).toHaveAttribute(
        'href',
        `/sessions/${CHILD_2}?q=${CHILD_2}&host=codex&range=7d`,
      );
      await expect(
        child.getByRole('link', {
          name: `Sub-session of ${PARENT_TITLE}, open parent session ${PARENT}`,
        }),
      ).toBeVisible();
      // Opening grows the table, and the card with it, up to the card's share
      // of the window, and the rest scrolls inside the table; the page itself
      // never scrolls.
      expect(opened.table.height).toBeGreaterThanOrEqual(closed.table.height - 0.5);
      if (closed.table.scrollHeight <= closed.table.clientHeight + 1)
        expect(opened.table.height).toBeGreaterThan(closed.table.height);
      expect(opened.table.scrollHeight).toBeGreaterThan(opened.table.clientHeight);
      expect(opened.page).toEqual(closed.page);
      await page
        .locator('.xt-lanes')
        .screenshot({ path: info.outputPath(`groups-opened-${width}-${scheme}.png`) });
      await page.locator('.xt-lanes .xt-table-scroll').evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
      await expect(
        table.getByRole('link', { name: `Open session ${ORDINARY.at(-1)}` }),
      ).toBeInViewport();
      const scrolled = await measure(page);
      expect(scrolled.page).toEqual(closed.page);

      // Enter closes it again: its children and their links leave the page,
      // and focus stays on the disclosure.
      await present.focus();
      await page.keyboard.press('Enter');
      await expect(present).toHaveAttribute('aria-expanded', 'false');
      await expect(present).toBeFocused();
      for (const id of [CHILD_1, CHILD_2, CHILD_3])
        await expect(table.locator(`a[href*="${id}"]`)).toHaveCount(0);
      await expect(table.getByRole('link', { name: `Open session ${KID_A}` })).toBeVisible();

      // Closing the other group too returns the table and the card to the
      // collapsed rows, focus kept on the disclosure, and the page never moved.
      await absent.focus();
      await page.keyboard.press('Enter');
      await expect(absent).toHaveAttribute('aria-expanded', 'false');
      await expect(absent).toBeFocused();
      const reclosed = await measure(page);
      expectFits(reclosed);
      expect(reclosed.table.height).toBeCloseTo(closed.table.height, 0);
      expect(reclosed.page).toEqual(closed.page);
    });

/**
 * A short list: six rows, two of them collapsed groups. The card keeps room
 * for eight rows, so the two it does not list stay blank inside the table
 * rather than going to effort and environment, and no gap opens under the
 * card; opening a group to nine rows grows the card by the one row past the
 * eight it keeps, and closing it shrinks it back, focus kept.
 */
for (const [width, height] of [
  [1120, 720],
  [1440, 900],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`a short list fits its rows and grows with a group at ${width}x${height} ${scheme}`, async ({
      page,
    }) => {
      await open(page, width, height, scheme, false, 3);
      const present = page.getByRole('button', {
        name: `3 sub-sessions of ${PARENT_TITLE}`,
      });
      await expect(present).toHaveAttribute('aria-expanded', 'false');
      const closed = await measure(page);
      expectFits(closed);
      expect(closed.rows).toHaveLength(6);
      expect(closed.table.scrollHeight).toBeLessThanOrEqual(closed.table.clientHeight + 1);
      // The table's 24px header and eight 23px rows, six of them listed.
      expect(closed.table.height).toBeCloseTo(24 + 8 * 23 - 1, 0);
      expect(closed.below).toBeLessThanOrEqual(8.5);

      await present.focus();
      await page.keyboard.press('Enter');
      await expect(present).toHaveAttribute('aria-expanded', 'true');
      await expect(present).toBeFocused();
      const opened = await measure(page);
      expectFits(opened);
      expect(opened.rows).toHaveLength(9);
      // The card grows by the one row past the eight it keeps, or, where
      // that passes its share of the window, up to the share, and the rows
      // beyond it scroll inside the table.
      const grown = opened.table.height - closed.table.height;
      if (opened.table.scrollHeight <= opened.table.clientHeight + 1)
        expect(grown).toBeCloseTo(23, 0);
      else expect(grown).toBeGreaterThan(0);
      expect(opened.card - closed.card).toBeCloseTo(grown, 0);
      expect(opened.below).toBeLessThanOrEqual(8.5);

      await page.keyboard.press('Enter');
      await expect(present).toHaveAttribute('aria-expanded', 'false');
      await expect(present).toBeFocused();
      const reclosed = await measure(page);
      expect(reclosed.table.height).toBeCloseTo(closed.table.height, 0);
      expect(reclosed.card).toBeCloseTo(closed.card, 0);
      expect(reclosed.below).toBeLessThanOrEqual(8.5);
    });

test('an opened child shows its own cost, not its parent’s', async ({ page }) => {
  await open(page, 1120, 720, 'light');
  const table = page.getByRole('table', { name: 'Session lanes' });
  const row = (id: string) =>
    table.getByRole('row').filter({ has: page.locator(`[data-visible-id="${id}"]`) });
  // Collapsed, each group's row carries its Σ total: the parent's $3 with
  // its $4, $7 and $8, and the named parent's two sub-sessions' $1 and $6.
  await expect(row(PARENT).getByRole('cell').last()).toHaveText('Σ $22.00');
  const named = table.getByRole('row').filter({ hasText: 'Sub-sessions of Session 01a0eeee' });
  await expect(named.getByRole('cell').last()).toHaveText('Σ $7.00');
  await table.getByRole('button', { name: `3 sub-sessions of ${PARENT_TITLE}` }).click();
  await table.getByRole('button', { name: '2 sub-sessions of Session 01a0eeee' }).click();
  // Open, the named parent has no cost of its own and every row shows its
  // own, so the cells add up to the totals they replaced.
  await expect(named.getByRole('cell').last()).toHaveText('');
  await expect(row(KID_A).getByRole('cell').last()).toHaveText('$1.00');
  await expect(row(KID_B).getByRole('cell').last()).toHaveText('$6.00');
  await expect(row(CHILD_3).getByRole('cell').last()).toHaveText('$8.00');
  // Each row's cost is the report's own for that session: index + 1 dollars.
  await expect(row(CHILD_2).getByRole('cell').last()).toHaveText('$7.00');
  await expect(row(CHILD_1).getByRole('cell').last()).toHaveText('$4.00');
  await expect(row(PARENT).getByRole('cell').last()).toHaveText('$3.00');
});

test('an absent parent opens its own page from its group row', async ({ page }) => {
  await open(page, 1120, 720, 'light');
  const link = page.getByRole('link', { name: `Open parent session Session 01a0eeee, ${GONE}` });
  await link.focus();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(new RegExp(`/sessions/${GONE}\\?q=${GONE}&host=codex&range=7d$`));
});

test('a verified automated reviewer keeps its own row and opens its own detail', async ({
  page,
}) => {
  await open(page, 1120, 720, 'light', true);
  const table = page.getByRole('table', { name: 'Session lanes' });
  const group = table.getByRole('button', { name: '2 sub-sessions of Session 01a0eeee' });
  await group.click();
  const reviewer = table.getByRole('link', { name: `Open session Automated review, ${KID_A}` });
  await expect(reviewer).toBeVisible();
  await expect(reviewer).toHaveText('Automated review');
  expectFits(await measure(page));
  await reviewer.click();
  await expect(page).toHaveURL(new RegExp(`/sessions/${KID_A}\\?q=${KID_A}&host=codex&range=7d$`));
  await page.getByRole('link', { name: 'All sessions' }).click();
  await expect(page).toHaveURL(/\/sessions\?/);
});
