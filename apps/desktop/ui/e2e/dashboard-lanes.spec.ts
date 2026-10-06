import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardLane } from '../src/data/generated/DashboardLane';
import type { DashboardLaneCost } from '../src/data/generated/DashboardLaneCost';
import type { DashboardLaneSession } from '../src/data/generated/DashboardLaneSession';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * Synthetic lane context (not real history): a long repository path, a long
 * branch, a long saved title, a start years before the axis, a three-digit
 * link count with inferred evidence, a measured zero, a four-figure partial cost
 * and an unknown neighbour, served in place of the development F1 export. The row must stay one 22px line at both
 * supported widths in both schemes — the identity and the number keep their
 * room, and only the repository is allowed to end in an ellipsis.
 */
// JSON imports widen literal unions; the export is the generated shape.
const contextual = structuredClone(fixture as FixtureExport);
for (const report of contextual.dashboards) {
  const start = report.lane_start_ms;
  const minute = 60_000;
  const span = (session_id: string, offset: number): DashboardLane => ({
    session_id,
    host: 'claude',
    start_ms: start + offset * minute,
    end_ms: start + (offset + 20) * minute,
  });
  const context = (
    session_id: string,
    repo: string | null,
    branch: string | null,
    cost: DashboardLaneCost | null,
    extra: Partial<DashboardLaneSession> = {},
  ): DashboardLaneSession => ({
    session_id,
    host: 'claude',
    repo,
    branch,
    title: null,
    started_at_ms: null,
    pr_links: 0,
    inferred_pr_links: 0,
    cost,
    ...extra,
    automated_review: extra.automated_review ?? false,
  });
  const priced = (total: number, patch: Partial<DashboardLaneCost> = {}): DashboardLaneCost => ({
    total_usd: total,
    priced_subtotal_usd: total,
    selected_observations: 1,
    priced_observations: 1,
    unpriced_observations: 0,
    assumed_tier_observations: 0,
    unpriced: [],
    ...patch,
  });
  report.lanes = [span('lane-long', 2400), span('lane-short', 1200), span('lane-bare', 60)];
  report.lanes_total = 3;
  report.lanes_truncated = false;
  report.lane_sessions = [
    context(
      'lane-bare',
      null,
      null,
      // An indexed session with nothing selected to price, whose host
      // recorded no start.
      priced(0, { total_usd: null, selected_observations: 0, priced_observations: 0 }),
      { pr_links: 2 },
    ),
    context(
      'lane-long',
      '/Users/developer/code/organisation/a-very-long-repository-name-for-layout',
      'feature/a-long-branch-name-for-layout-checks',
      // The widest amount the column is sized for: whole dollars, partial.
      priced(0, {
        total_usd: null,
        priced_subtotal_usd: 1_234.4,
        selected_observations: 9,
        priced_observations: 8,
        unpriced_observations: 1,
      }),
      {
        title: 'A saved session title that is far too long for the lane row at any width',
        automated_review: false,
        started_at_ms: Date.UTC(2025, 11, 31, 23, 59),
        pr_links: 128,
        inferred_pr_links: 7,
      },
    ),
    context('lane-short', '/Users/developer/code/acme-api', 'main', priced(0), {
      automated_review: false,
      started_at_ms: start + 1_150 * minute,
      pr_links: 0,
    }),
  ];
}

async function serveContext(page: Page) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(contextual)};`,
    }),
  );
}

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`lane context and cost stay readable at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      await serveContext(page);
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/dashboard');
      const table = page.getByRole('table', { name: 'Session lanes' });
      await expect(table.getByRole('row')).toHaveCount(4);
      await page.evaluate(() => document.fonts.ready);
      const measured = await page.evaluate(() => {
        const table = document.querySelector<HTMLElement>('.xt-lanes .xt-data-table')!;
        const fits = (element: HTMLElement | null) =>
          element ? element.scrollWidth <= element.clientWidth + 1 : true;
        const cells = (row: HTMLElement) => [...row.querySelectorAll<HTMLElement>('[role="cell"]')];
        const rows = [...table.querySelectorAll<HTMLElement>('.xt-data-row')].map((row) => {
          const box = row.getBoundingClientRect();
          const cost = row.querySelector<HTMLElement>('.xt-metric-cell');
          return {
            text: row.textContent,
            height: box.height,
            // An unmeasured cell carries its reason for assistive technology
            // beside the visible dash; compare only what is shown.
            cost:
              cost?.querySelector('[aria-hidden="true"]')?.textContent ?? cost?.textContent ?? null,
            costFits: fits(cost),
            // Every cell must stay inside the row it belongs to.
            inside: cells(row).every(
              (cell) =>
                cell.getBoundingClientRect().left >= box.left - 0.5 &&
                cell.getBoundingClientRect().right <= box.right + 0.5,
            ),
            identity: row.querySelector<HTMLElement>('.xt-lane-find')!.textContent,
            identityClipped: !fits(row.querySelector<HTMLElement>('.xt-lane-find')),
            prs: [...row.querySelectorAll<HTMLElement>('.xt-lane-prs > [aria-hidden="true"]')].at(
              -1,
            )?.textContent,
            prsFits: fits(row.querySelector<HTMLElement>('.xt-lane-prs')),
            inferred: row.querySelector('.xt-lane-prs .xt-evidence') !== null,
            started:
              row.querySelector<HTMLElement>('time')?.textContent ??
              cells(row)[4].querySelector('[aria-hidden="true"]')?.textContent ??
              null,
            startedFits: fits(cells(row)[4]),
            context: row.querySelector<HTMLElement>('.xt-lane-repo')!.textContent,
            contextClipped: !fits(row.querySelector<HTMLElement>('.xt-lane-repo')),
          };
        });
        const scroll = document.querySelector<HTMLElement>('.xt-lanes .xt-table-scroll')!;
        const costHeader = table.querySelector<HTMLElement>(
          '.xt-table-metric-header[aria-label^="Whole-session cost"]',
        )!;
        const header = [
          ...table.querySelectorAll<HTMLElement>('.xt-table-head [role="columnheader"]'),
        ]
          .map((cell) => cell.textContent?.trim())
          .filter(Boolean);
        // The activity column's visible name: rendered, whole, inside its
        // header cell and the 24px band, on the same line as the tick row,
        // and overlapped by no tick.
        const head = table.querySelector<HTMLElement>('.xt-table-head')!.getBoundingClientRect();
        const label = table.querySelector<HTMLElement>('.xt-lanes-activity-label')!;
        const labelBox = label.getBoundingClientRect();
        const labelCell = label
          .closest<HTMLElement>('[role="columnheader"]')!
          .getBoundingClientRect();
        const labelStyle = getComputedStyle(label);
        const ticks = [
          ...table.querySelectorAll<HTMLElement>('.xt-lanes-axis > span:not(.sr-only)'),
        ]
          .map((tick) => tick.getBoundingClientRect())
          .filter((box) => box.width > 0);
        const activity = {
          text: label.innerText,
          visible:
            labelStyle.visibility === 'visible' &&
            labelStyle.display !== 'none' &&
            Number(labelStyle.opacity) > 0 &&
            labelBox.width > 0 &&
            labelBox.height > 0,
          fits: fits(label),
          insideCell:
            labelBox.left >= labelCell.left - 0.5 && labelBox.right <= labelCell.right + 0.5,
          insideHead: labelBox.top >= head.top - 0.5 && labelBox.bottom <= head.bottom + 0.5,
          sameLine: ticks.every(
            (tick) => tick.top < labelBox.bottom - 0.5 && tick.bottom > labelBox.top + 0.5,
          ),
          clearOfTicks: ticks.every(
            (tick) => tick.left >= labelBox.right - 0.5 || tick.right <= labelBox.left + 0.5,
          ),
          ticksApart: ticks.slice(1).every((tick, index) => tick.left >= ticks[index]!.right - 0.5),
          ticksInsideHead: ticks.every(
            (tick) => tick.top >= head.top - 0.5 && tick.bottom <= head.bottom + 0.5,
          ),
          ticks: ticks.length,
          headHeight: head.height,
        };
        return {
          activity,
          rows,
          header,
          horizontalOverflow: scroll.scrollWidth - scroll.clientWidth,
          costHeader: {
            text: costHeader.textContent,
            fits: fits(costHeader),
          },
        };
      });
      await info.attach('lanes', { body: JSON.stringify(measured, null, 2) });
      await page.locator('.xt-lanes').screenshot({
        path: info.outputPath(`lanes-${width}-${scheme}.png`),
      });
      // The measured column names its own window beside the axis it is not,
      // in full rather than in an ellipsis.
      expect(measured.header.slice(0, 5)).toEqual([
        'Host',
        'session',
        'Compactions',
        'PRs',
        'started',
      ]);
      expect(measured.header.at(-1)).toBe('cost');
      // The activity column is named on screen, not only for assistive
      // technology, and naming it keeps the header band and its ticks.
      expect(measured.activity).toMatchObject({
        text: 'ACTIVITY',
        visible: true,
        fits: true,
        insideCell: true,
        insideHead: true,
        sameLine: true,
        clearOfTicks: true,
        ticksApart: true,
        ticksInsideHead: true,
      });
      expect(measured.activity.ticks).toBeGreaterThanOrEqual(3);
      expect(measured.activity.headHeight).toBeCloseTo(24, 0);
      await expect(page.locator('.xt-lanes-activity-label')).toBeVisible();
      expect(measured.header.join(' ')).not.toContain('first seen');
      expect(measured.costHeader).toEqual({ text: 'cost', fits: true });
      expect(measured.horizontalOverflow).toBeLessThanOrEqual(1);
      expect(measured.rows).toHaveLength(3);
      for (const row of measured.rows) {
        expect(row.height).toBeCloseTo(22, 0);
        expect(row.inside).toBe(true);
        expect(row.costFits).toBe(true);
        expect(row.prsFits).toBe(true);
        expect(row.startedFits).toBe(true);
      }
      // Recorded links: a three-digit count with inferred evidence marked, a
      // two, and a measured zero, all whole in their column.
      expect(measured.rows.map((row) => [row.prs, row.inferred])).toEqual([
        ['128', true],
        ['0', false],
        ['2', false],
      ]);
      // A start years before the axis reads as itself; unknown stays a dash.
      expect(measured.rows[0].started).toMatch(/^Dec 31(,| at) 23:59$/);
      expect(measured.rows[2].started).toBe('—');
      // A partial cost, a measured zero and an unknown one stay distinguishable.
      expect(measured.rows.map((row) => row.cost)).toEqual(['$1,234+', '$0.00', '—']);
      // The identity is never dropped, however long the repository is: only
      // the context gives way to the ellipsis.
      expect(measured.rows.map((row) => row.identity)).toEqual([
        'A saved session title that is far too long for the lane row at any width',
        'Session lane-sho',
        'Session lane-bar',
      ]);
      // A long saved title ellipsizes only past most of the cell; an identity
      // fits whole; the context gives way first.
      expect(measured.rows.map((row) => row.identityClipped)).toEqual([true, false, false]);
      expect(measured.rows[0].contextClipped).toBe(true);
      expect(measured.rows[2].context).toBe('Unknown repository');
    });
