import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import { decimalHours } from '../src/app/agent-duration';
import { leverageText } from '../src/app/metric-format';
import { syntheticMergedPage, syntheticOverview } from '../src/app/dashboard/overview.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The Overview card and the Effort card's "human h" timeline over the
 * test-only synthetic report (not real history) served in place of the F1
 * export. `OVERVIEW_SHOTS=DIR` also saves a screenshot of each case there.
 */
const synthetic = structuredClone(fixture as FixtureExport);
synthetic.dashboards = synthetic.dashboards.map((report) => syntheticOverview(report));
synthetic.pr_analytics = synthetic.pr_analytics.map(syntheticMergedPage);
// One very busy day in the 7-day range: a three-digit-hour agent total
// (123 h 45 m, written `123.8 h`), which the agents column must hold whole.
const LONG_AGENT_MS = (123 * 60 + 45) * 60_000;
{
  const week = synthetic.dashboards.find((r) => r.window.days === 7)!;
  const busy = week.leverage.by_day.at(-2)!;
  week.leverage.agent_ms += LONG_AGENT_MS - busy.agent_ms;
  busy.agent_ms = LONG_AGENT_MS;
  // And large leverages on other days (`123.4×`, `36,000×` and the widest
  // the leverage column must hold whole, `99,999.9×`), as a day with a
  // minute of yours and many agent hours has. Test data only: they are not
  // those days' agent time divided by yours.
  for (const [at, value] of [
    [-4, 123.4],
    [-5, 99_999.9],
    [-6, 36_000],
  ] as const)
    Object.assign(week.leverage.by_day.at(at)!, { value, at_or_below_even: false });
  // And a day at or below even (`0.8×`), which the report flags.
  Object.assign(week.leverage.by_day.at(-7)!, { value: 0.8, at_or_below_even: true });
}
const shots = process.env.OVERVIEW_SHOTS;

/** `#8b80ff` → `rgb(139, 128, 255)`, as computed styles write colours. */
const rgb = (hex: string) =>
  `rgb(${[1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16)).join(', ')})`;

async function open(
  page: Page,
  scheme: 'light' | 'dark',
  width = 1440,
  height = 900,
  data: FixtureExport = synthetic,
) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(data)};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByTestId('overview-grid')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const range = (page: Page, value: '7d' | '14d' | '30d') =>
  page.getByRole('radio', { name: value, exact: true }).click();
const measure = (page: Page, name: 'human + agent' | 'agent h' | 'human h' | 'cost') =>
  page.getByRole('radio', { name, exact: true }).click();
const effortCard = (page: Page) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Effort', exact: true }),
  });

/** The timeline's geometry: rows, their height, the scroll position and the bars. */
async function timeline(page: Page) {
  return page.getByTestId('human-timeline-body').evaluate((body) => {
    const rows = [...body.querySelectorAll<HTMLElement>('[data-testid="human-day"]')];
    const tracks = rows.map((row) => row.querySelector('.xt-human-track')!.getBoundingClientRect());
    const axis = document.querySelector('.xt-human-axis')!.getBoundingClientRect();
    return {
      rows: rows.length,
      rowHeight: rows[0]!.getBoundingClientRect().height,
      trackHeight: Math.max(...tracks.map((track) => track.height)),
      clientHeight: body.clientHeight,
      scrollHeight: body.scrollHeight,
      scrollTop: body.scrollTop,
      axisAboveBody: axis.bottom <= body.getBoundingClientRect().top + 1,
      lastVisible:
        rows.at(-1)!.getBoundingClientRect().bottom <= body.getBoundingClientRect().bottom + 1,
      lastDate: rows.at(-1)!.dataset.date,
    };
  });
}

for (const scheme of ['light', 'dark'] as const)
  test(`the Overview card and the human h timeline at 7d and 30d, ${scheme}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, scheme);
    // The tile row and the Environment card are gone; Overview holds the four.
    await expect(page.locator('.xt-dash-tiles')).toHaveCount(0);
    await expect(page.getByRole('heading', { level: 2, name: 'Environment' })).toHaveCount(0);
    await expect(page.getByTestId('overview-tile')).toHaveCount(4);
    await expect(
      page.getByTestId('overview-tile').evaluateAll((tiles) => tiles.map((t) => t.dataset.label)),
    ).resolves.toEqual(['Leverage', 'Concurrency', 'Merged PRs', 'Hands-off median']);
    await expect(page.getByTestId('overview-prs').locator('li').first()).toBeVisible();
    await expect(page.getByTestId('day-line')).toHaveCount(3);
    const heights: number[] = [];
    for (const preset of ['7d', '30d'] as const) {
      await range(page, preset);
      await measure(page, 'agent h');
      await expect(page.getByTestId('effort-chart')).toBeVisible();
      heights.push((await effortCard(page).boundingBox())!.height);
      if (shots)
        await page.screenshot({ path: `${shots}/dashboard-${preset}-agent-${scheme}.png` });
      await measure(page, 'human h');
      await expect(page.getByTestId('human-timeline')).toBeVisible();
      heights.push((await effortCard(page).boundingBox())!.height);
      const geometry = await timeline(page);
      const days = synthetic.dashboards.find((r) => r.window.days === Number(preset.slice(0, -1)))!
        .human_hours.current.by_day;
      expect(geometry.rows).toBe(days.length);
      // The newest day is the last row, in view; the hour axis stays above the rows.
      expect(geometry.lastDate).toBe(days.at(-1)!.date);
      expect(geometry.lastVisible).toBe(true);
      expect(geometry.axisAboveBody).toBe(true);
      // Eight rows fill the plot; the bars keep space between rows.
      expect(geometry.rowHeight).toBeCloseTo(
        Math.max(28, Math.floor(geometry.clientHeight / 8)),
        0,
      );
      expect(geometry.trackHeight).toBeLessThanOrEqual(30);
      expect(geometry.trackHeight).toBeLessThan(geometry.rowHeight);
      if (preset === '30d') {
        // Longer ranges scroll, opened on today.
        expect(geometry.scrollHeight).toBeGreaterThan(geometry.clientHeight);
        expect(geometry.scrollTop + geometry.clientHeight).toBeGreaterThanOrEqual(
          geometry.scrollHeight - 1,
        );
      } else expect(geometry.scrollHeight).toBeLessThanOrEqual(geometry.clientHeight + 1);
      if (shots)
        await page.screenshot({ path: `${shots}/dashboard-${preset}-human-${scheme}.png` });
    }
    // The Effort card keeps one height in every range and view.
    for (const height of heights) expect(height).toBeCloseTo(heights[0]!, 0);
    expect(errors).toEqual([]);
  });

test('the Effort switch and a day of the timeline work from the keyboard', async ({ page }) => {
  await open(page, 'light');
  const agent = page.getByRole('radio', { name: 'agent h', exact: true });
  await agent.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('radio', { name: 'human h', exact: true })).toBeChecked();
  await expect(page.getByTestId('human-timeline')).toBeVisible();
  // One tab stop: today's row. Tab reaches it, arrows move between days,
  // and the focused day's card opens.
  const timeline = page.getByTestId('human-timeline');
  await expect(timeline.locator('[tabindex="0"]')).toHaveCount(1);
  const days = page.getByTestId('human-day');
  const last = days.last();
  await expect(last).toHaveAttribute('tabindex', '0');
  await expect(last).toHaveAttribute('aria-label', /^Sep 7: /);
  await last.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(last).toBeFocused();
  // These visual-only day popups are aria-hidden and their triggers carry
  // no described-by/controls link. Select the active card for the fixed day,
  // so another day's closing animation cannot make the query ambiguous.
  const activeDayCard = (day: string) =>
    page.locator('.xt-effort-tip[data-open]').filter({
      has: page.locator('.xt-effort-tip-head > span', { hasText: new RegExp(`^${day}$`) }),
    });
  await expect(page.locator('.xt-effort-tip[data-open]')).toHaveCount(1);
  await expect(activeDayCard('Sep 7')).toHaveCount(1);
  await expect(activeDayCard('Sep 7')).toBeVisible();
  await expect(activeDayCard('Sep 7').locator('.xt-effort-tip-head')).toContainText('Sep 7');
  await page.keyboard.press('ArrowUp');
  const before = days.nth(-2);
  await expect(before).toBeFocused();
  await expect(before).toHaveAttribute('aria-label', /^Sep 6: /);
  await expect(page.locator('.xt-effort-tip[data-open]')).toHaveCount(1);
  await expect(activeDayCard('Sep 6')).toHaveCount(1);
  await expect(activeDayCard('Sep 6').locator('.xt-effort-tip-head')).toContainText('Sep 6');
  await expect(timeline.locator('[tabindex="0"]')).toHaveCount(1);
  await page.keyboard.press('Home');
  await expect(days.first()).toBeFocused();
});

for (const scheme of ['light', 'dark'] as const)
  test(`the human + agent tab: first option, one track per day with your bands and an agents line, icon keys, totals and the plain human h view unchanged, ${scheme}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, scheme);
    const options = page.getByRole('radiogroup', { name: 'Effort measure' }).getByRole('radio');
    await expect(options).toHaveText(['human + agent', 'agent h', 'human h', 'cost']);
    // The card opens on human + agent, with its timeline and no bar chart.
    await expect(page.getByRole('radio', { name: 'human + agent', exact: true })).toBeChecked();
    await expect(page.getByTestId('human-timeline')).toHaveAttribute('data-variant', 'paired');
    await expect(page.getByTestId('effort-chart')).toHaveCount(0);
    await measure(page, 'agent h');
    await expect(page.getByTestId('effort-chart')).toBeVisible();
    const report = synthetic.dashboards.find((r) => r.window.days === 7)!;
    const top = report.leverage.agents_scale_top;
    await measure(page, 'human + agent');
    const timelineView = page.getByTestId('human-timeline');
    await expect(timelineView).toHaveAttribute('data-variant', 'paired');
    const days = page.getByTestId('human-day');
    await expect(days).toHaveCount(report.leverage.by_day.length);
    // One track per day; a line on every day with agent time; no filled
    // ridge or strip.
    await expect(timelineView.locator('.xt-human-track')).toHaveCount(
      report.leverage.by_day.length,
    );
    await expect(page.getByTestId('agent-line')).toHaveCount(
      report.leverage.by_day.filter((day) => day.quarter_agents.some((agents) => agents > 0))
        .length,
    );
    await expect(page.locator('.xt-agent-strip, .xt-agent-ridge')).toHaveCount(0);
    const fills = await page
      .getByTestId('agent-line')
      .locator('path')
      .evaluateAll((paths) => paths.map((path) => getComputedStyle(path).fill));
    for (const fill of fills) expect(fill).toBe('none');
    // Headline: named icon keys instead of the words, then the totals and
    // the leverage; no break threshold.
    const headline = page.getByTestId('effort-headline');
    await expect(headline).toHaveText(
      /^\d+ h \d+ m\d+ h \d+ m[\d.]+×agent time per hour of yours$/,
    );
    await expect(headline.getByRole('img', { name: 'Your time' })).toBeVisible();
    await expect(headline.getByRole('img', { name: 'Agent time' })).toBeVisible();
    // The person icon heads your column, in your colour, and the robot the
    // agents' column beside it.
    const head = page.getByTestId('human-total-head');
    await expect(head.locator('svg')).toBeVisible();
    await expect(page.getByTestId('agent-total-head').locator('svg')).toBeVisible();
    await expect(page.getByTestId('leverage-total-head').locator('svg')).toBeVisible();
    const columns = await page.evaluate(() => {
      const head = document.querySelector('[data-testid="human-total-head"]')!;
      const agentHead = document.querySelector('[data-testid="agent-total-head"]')!;
      const leverageHead = document.querySelector('[data-testid="leverage-total-head"]')!;
      const totals = document
        .querySelector('[data-testid="human-day"]')!
        .querySelectorAll('.xt-human-total');
      const swatch = document.querySelector('.xt-paired-swatch[data-key="band"]')!;
      return {
        headRight: head.getBoundingClientRect().right,
        totalRight: totals[0]!.getBoundingClientRect().right,
        agentHeadRight: agentHead.getBoundingClientRect().right,
        agentTotalRight: totals[1]!.getBoundingClientRect().right,
        leverageHeadRight: leverageHead.getBoundingClientRect().right,
        leverageTotalRight: totals[2]!.getBoundingClientRect().right,
        humanRightOfLeft:
          totals[1]!.getBoundingClientRect().left - totals[0]!.getBoundingClientRect().right,
        headColor: getComputedStyle(head).color,
        human: getComputedStyle(swatch).getPropertyValue('--human').trim(),
      };
    });
    expect(Math.abs(columns.headRight - columns.totalRight)).toBeLessThanOrEqual(1);
    expect(Math.abs(columns.agentHeadRight - columns.agentTotalRight)).toBeLessThanOrEqual(1);
    expect(Math.abs(columns.leverageHeadRight - columns.leverageTotalRight)).toBeLessThanOrEqual(1);
    expect(columns.humanRightOfLeft).toBeGreaterThanOrEqual(0);
    expect(columns.human).toBe(scheme === 'dark' ? '#2dd4bf' : '#0d9488');
    expect(columns.headColor).toBe(scheme === 'dark' ? 'rgb(45, 212, 191)' : 'rgb(13, 148, 136)');
    await expect(page.getByTestId('effort-leverage')).toHaveText(/^[\d.]+×$/);
    // The leverage sits at the right, its right edge on the agents column's,
    // with both totals keys on the left of the same line.
    const edges = await page.evaluate(() => {
      // The figure's visible text: its last word's right edge.
      const leverage = document
        .querySelector('[data-testid="effort-leverage"]')!
        .closest('.xt-paired-stat')!
        .querySelector('small')!
        .getBoundingClientRect();
      const figure = document
        .querySelector('[data-testid="effort-leverage"]')!
        .getBoundingClientRect();
      const last = document
        .querySelector('[data-testid="leverage-row-total"]')!
        .getBoundingClientRect();
      const keys = [...document.querySelectorAll('.xt-paired-key')].map((key) =>
        key.getBoundingClientRect(),
      );
      return {
        leverageRight: leverage.right,
        lastRight: last.right,
        leverageTop: leverage.top,
        keyTops: keys.map((key) => key.top),
        keyRights: keys.map((key) => key.right),
        leverageLeft: figure.left,
      };
    });
    expect(Math.abs(edges.leverageRight - edges.lastRight)).toBeLessThanOrEqual(1);
    for (const right of edges.keyRights) expect(right).toBeLessThan(edges.leverageLeft);
    for (const top of edges.keyTops) expect(Math.abs(top - edges.leverageTop)).toBeLessThan(12);
    await expect(headline).not.toContainText('break threshold');
    // Each row: your total, the agents' whole-day total and the day's
    // leverage from the report, all whole on one line and centred with the
    // track.
    await expect(page.getByTestId('agent-day-total')).toHaveCount(0);
    const rowTotals = await days.evaluateAll((rows) =>
      rows.map((row) => {
        const track = row.querySelector('.xt-human-track')!.getBoundingClientRect();
        const totals = [...row.querySelectorAll<HTMLElement>('.xt-human-total')];
        return {
          texts: totals.map((total) => total.textContent),
          centres: totals.map((total) => {
            const box = total.getBoundingClientRect();
            return box.top + box.height / 2 - (track.top + track.height / 2);
          }),
          whole: totals.every((total) => total.scrollWidth <= total.clientWidth),
          oneLine: totals.every((total) => total.getBoundingClientRect().height < 20),
          agentColor: getComputedStyle(totals[1]!).color,
          accent: getComputedStyle(totals[1]!).getPropertyValue('--accent').trim(),
          humanColor: getComputedStyle(totals[0]!).color,
          humanInk: getComputedStyle(totals[0]!).getPropertyValue('--human-ink').trim(),
          leverageColor: getComputedStyle(totals[2]!).color,
          secondary: getComputedStyle(totals[2]!).getPropertyValue('--secondary').trim(),
          meta: getComputedStyle(totals[2]!).getPropertyValue('--meta').trim(),
          danger: getComputedStyle(totals[2]!).getPropertyValue('--danger').trim(),
        };
      }),
    );
    expect(rowTotals).toHaveLength(report.leverage.by_day.length);
    expect(rowTotals.at(-2)!.texts[1]).toBe('123.8 h');
    expect(rowTotals.at(-4)!.texts[2]).toBe('123.4×');
    expect(rowTotals.at(-7)!.texts[2]).toBe('0.8×');
    expect(rowTotals.at(-7)!.leverageColor).toBe(rgb(rowTotals.at(-7)!.danger));
    expect(rowTotals.at(-1)!.leverageColor).not.toBe(rgb(rowTotals.at(-1)!.danger));
    expect(rowTotals.at(-5)!.texts[2]).toBe('99,999.9×');
    expect(rowTotals.at(-6)!.texts[2]).toBe('36,000×');
    rowTotals.forEach((row, index) => {
      expect(row.texts).toHaveLength(3);
      // The day's leverage is the report's value, written as the headline
      // writes it; a day without one says so in the meta colour.
      const value = report.leverage.by_day[index]!.value;
      expect(row.texts[2]).toBe(value === null ? '—' : leverageText(value));
      // Red exactly on the days the report flags at or below even.
      const flagged = report.leverage.by_day[index]!.at_or_below_even === true;
      expect(row.leverageColor).toBe(
        rgb(value === null ? row.meta : flagged ? row.danger : row.secondary),
      );
      // Both time columns in decimal hours, from the report's own numbers.
      const human = report.leverage.by_day[index]!.human_ms;
      expect(row.texts[0]).toBe(human === null ? '—' : decimalHours(human));
      expect(row.texts[1]).toBe(decimalHours(report.leverage.by_day[index]!.agent_ms));
      for (const centre of row.centres) expect(Math.abs(centre)).toBeLessThanOrEqual(1);
      expect(row.whole).toBe(true);
      expect(row.oneLine).toBe(true);
      expect(row.agentColor).not.toBe(row.humanColor);
      // Your totals are your colour's text-safe shade.
      expect(row.humanInk).toBe(scheme === 'dark' ? '#2dd4bf' : '#0f766e');
      if (row.texts[0] !== '—') expect(row.humanColor).toBe(rgb(row.humanInk));
      // The agents' totals are the agent line's purple.
      expect(row.agentColor).toBe(rgb(row.accent));
    });
    // The line's highest point is the day's busiest average on the report's
    // one scale, in its box from 25 (no agents) up to 2 (the top); your time
    // is full-height bands in its own colour behind it.
    const last = report.leverage.by_day.at(-1)!;
    const geometry = await days.last().evaluate((row) => {
      const track = row.querySelector('.xt-human-track')!;
      const box = track.getBoundingClientRect();
      const line = row.querySelector<SVGPathElement>('[data-testid="agent-line"] path')!.getBBox();
      const bars = [...row.querySelectorAll('.xt-human-bar')].map((bar) =>
        bar.getBoundingClientRect(),
      );
      return {
        row: row.getBoundingClientRect().height,
        track: box.height,
        lineTop: line.y,
        lineBottom: line.y + line.height,
        lineLast: track.lastElementChild?.getAttribute('data-testid'),
        barHeights: bars.map((bar) => bar.height),
        barTops: bars.map((bar) => bar.top - box.top),
        barColor: bars.length
          ? getComputedStyle(row.querySelector('.xt-human-bar')!).backgroundColor
          : '',
      };
    });
    expect(geometry.row).toBeGreaterThanOrEqual(34);
    expect(geometry.track).toBeGreaterThanOrEqual(26);
    expect(geometry.lineLast).toBe('agent-line');
    expect(geometry.lineBottom).toBeCloseTo(25, 1);
    expect((25 - geometry.lineTop) / 23).toBeCloseTo(
      Math.min(Math.max(...last.quarter_agents), top) / top,
      2,
    );
    expect(geometry.barHeights.length).toBeGreaterThan(0);
    for (const height of geometry.barHeights) expect(height).toBeCloseTo(geometry.track, 0);
    for (const top of geometry.barTops) expect(Math.abs(top)).toBeLessThanOrEqual(1);
    // A see-through colour mix (your colour at about a fifth), not a solid fill.
    expect(geometry.barColor).toMatch(/^(color\(srgb|rgba\(|oklab)/);
    // Pointing at a day's track draws the hairline and names that quarter;
    // the card keeps the day's agent total.
    const track = days.last().locator('.xt-human-track');
    const box = (await track.boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.5 + 2, box.y + box.height / 2);
    await expect(page.getByTestId('agent-hairline')).toBeVisible();
    await expect(page.getByTestId('agent-quarter')).toBeVisible();
    const card = page.locator('.xt-effort-tip[data-open]');
    await expect(card).toContainText('Agent time');
    await expect(card.getByTestId('agent-day-total')).toHaveText(/^\d+ h \d+ m$/);
    if (shots) await page.screenshot({ path: `${shots}/dashboard-7d-paired-${scheme}.png` });
    await page.mouse.move(0, 0);
    // In a narrow card the headline wraps whole items onto a second line and
    // grows, never running into the hour axis below it.
    const narrow = await timelineView.evaluate((view) => {
      view.style.width = '420px';
      const headline = view.querySelector('.xt-paired-headline')!.getBoundingClientRect();
      const items = [...view.querySelectorAll('.xt-paired-headline > *')].map((item) =>
        item.getBoundingClientRect(),
      );
      const axis = view.querySelector('.xt-human-axis')!.getBoundingClientRect();
      const result = {
        lines: new Set(items.map((item) => Math.round(item.top))).size,
        itemsBottom: Math.max(...items.map((item) => item.bottom)),
        headlineBottom: headline.bottom,
        axisTop: axis.top,
      };
      view.style.width = '';
      return result;
    });
    expect(narrow.lines).toBeGreaterThan(1);
    expect(narrow.itemsBottom).toBeLessThanOrEqual(narrow.headlineBottom + 0.5);
    expect(narrow.headlineBottom).toBeLessThanOrEqual(narrow.axisTop + 0.5);
    // 30 days scroll, opened on today.
    await range(page, '30d');
    const body = page.getByTestId('human-timeline-body');
    const scroll = await body.evaluate((element) => ({
      top: element.scrollTop,
      client: element.clientHeight,
      height: element.scrollHeight,
    }));
    expect(scroll.height).toBeGreaterThan(scroll.client);
    expect(scroll.top + scroll.client).toBeGreaterThanOrEqual(scroll.height - 1);
    // The plain human h view has no line, bands or icons and keeps its
    // headline and inset blue bars.
    await measure(page, 'human h');
    await expect(timelineView).not.toHaveAttribute('data-variant', 'paired');
    await expect(page.getByTestId('agent-line')).toHaveCount(0);
    await expect(page.getByTestId('human-total-head')).toHaveCount(0);
    await expect(page.getByTestId('agent-total-head')).toHaveCount(0);
    await expect(page.getByTestId('agent-row-total')).toHaveCount(0);
    await expect(page.getByTestId('leverage-row-total')).toHaveCount(0);
    await expect(page.getByTestId('leverage-total-head')).toHaveCount(0);
    await expect(page.getByTestId('human-day').last().locator('.xt-human-total')).toHaveCount(1);
    await expect(page.getByTestId('effort-headline')).toContainText('break threshold');
    // Its totals keep the plain total colour.
    const plainTotals = await page.getByTestId('human-day').evaluateAll((rows) =>
      rows.map((row) => {
        const total = row.querySelector('.xt-human-total')!;
        return [
          getComputedStyle(total).color,
          getComputedStyle(total).getPropertyValue('--secondary').trim(),
        ];
      }),
    );
    for (const [color, secondary] of plainTotals) expect(color).toBe(rgb(secondary!));
    const plainBar = await page
      .getByTestId('human-day')
      .last()
      .locator('.xt-human-bar')
      .first()
      .evaluate((bar) => bar.getBoundingClientRect().height);
    expect(plainBar).toBeGreaterThan(4);
    expect(errors).toEqual([]);
  });

/**
 * Neither a day's agent hours nor its leverage has an upper bound: very
 * large values widen their columns instead of running into a neighbour.
 * Test data only: 1,234.5 h of agent time and a 123,456.7× leverage.
 */
for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  test(`very large agent hours and leverage widen their columns, ${width}x${height}`, async ({
    page,
  }) => {
    const huge = structuredClone(synthetic);
    const week = huge.dashboards.find((r) => r.window.days === 7)!;
    week.leverage.by_day.at(-2)!.agent_ms = 1234.5 * 3_600_000;
    Object.assign(week.leverage.by_day.at(-5)!, { value: 123_456.7, at_or_below_even: false });
    await open(page, 'light', width, height, huge);
    await expect(page.getByTestId('human-timeline')).toHaveAttribute('data-variant', 'paired');
    await expect(page.getByTestId('agent-row-total').nth(-2)).toHaveText('1,234.5 h');
    await expect(page.getByTestId('leverage-row-total').nth(-5)).toHaveText('123,456.7×');
    const g = await page.getByTestId('human-timeline').evaluate((view) => {
      const box = (element: Element) => element.getBoundingClientRect();
      const rows = [...view.querySelectorAll('[data-testid="human-day"]')].map((row) => {
        const cells = [...row.querySelectorAll('.xt-human-total')];
        return {
          rights: cells.map((cell) => box(cell).right),
          lefts: cells.map((cell) => box(cell).left),
          // Each text inside its own cell: whole, and clear of its neighbours.
          whole: cells.every((cell) => {
            const text = cell.querySelector('.xt-human-total-text')!;
            return (
              box(text).left >= box(cell).left - 0.5 && box(text).right <= box(cell).right + 0.5
            );
          }),
          trackRight: box(row.querySelector('.xt-human-track')!).right,
        };
      });
      const heads = ['human-total-head', 'agent-total-head', 'leverage-total-head'].map(
        (id) => box(view.querySelector(`[data-testid="${id}"]`)!).right,
      );
      const leverage = box(
        view
          .querySelector('[data-testid="effort-leverage"]')!
          .closest('.xt-paired-stat')!
          .querySelector('small')!,
      );
      return { rows, heads, headlineRight: leverage.right };
    });
    for (const row of g.rows) {
      expect(row.whole).toBe(true);
      // Cells in order, none running into the next or into the track.
      expect(row.lefts[0]!).toBeGreaterThanOrEqual(row.trackRight);
      for (let index = 1; index < row.lefts.length; index++)
        expect(row.lefts[index]!).toBeGreaterThanOrEqual(row.rights[index - 1]!);
      // Every row's columns line up with the head icons above them.
      row.rights.forEach((right, index) =>
        expect(Math.abs(right - g.heads[index]!)).toBeLessThanOrEqual(1),
      );
    }
    expect(Math.abs(g.headlineRight - g.rows[0]!.rights.at(-1)!)).toBeLessThanOrEqual(1);
  });

test('in human + agent, hovering a date opens its sessions box, the rest of the row its card, one at a time, and the keyboard opens the box too', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await open(page, 'light');
  await expect(page.getByTestId('human-timeline')).toHaveAttribute('data-variant', 'paired');
  // F1's one session with agent time, on its last day, as the native read
  // exported it.
  const id = '00000000-0000-4000-8000-000000000001';
  const href = `/sessions/${id}?q=${id}&host=claude&range=7d`;
  const url = new RegExp(`${href.replace(/[?]/g, '\\?')}$`);
  const days = page.getByTestId('human-day');
  const today = days.last();
  const shown = page.locator('.xt-effort-tip[data-open]');
  const centre = async (locator: ReturnType<Page['locator']>, across = 0.5) => {
    const box = (await locator.boundingBox())!;
    return [box.x + box.width * across, box.y + box.height / 2] as const;
  };
  /** Where a just-opened box's link has settled: its positioner may still move it. */
  const settled = async (locator: ReturnType<Page['locator']>) => {
    let last = '';
    await expect
      .poll(async () => {
        const now = JSON.stringify(await locator.boundingBox());
        const same = now === last;
        last = now;
        return same;
      })
      .toBe(true);
    return centre(locator);
  };
  // The track shows the day's card as before, without sessions.
  await page.mouse.move(...(await centre(today.locator('.xt-human-track'), 0.3)));
  await expect(shown).toHaveCount(1);
  await expect(shown.getByTestId('agent-day-total')).toBeVisible();
  await expect(page.getByTestId('day-sessions-head')).toHaveCount(0);
  // The date opens its sessions box instead, and the card closes.
  const date = today.getByRole('button', { name: 'Sessions on Sep 7' });
  await page.mouse.move(...(await centre(date)), { steps: 5 });
  const box = page.getByRole('dialog', { name: 'Sessions on Sep 7' });
  await expect(box.getByTestId('day-sessions-head')).toHaveText('Sessions on Sep 7');
  await expect(shown).toHaveCount(1);
  await expect(page.getByTestId('agent-day-total')).toHaveCount(0);
  // The date shows, quietly, that it opens something.
  expect(
    await date.evaluate((element) => [
      getComputedStyle(element).cursor,
      getComputedStyle(element).textDecorationLine,
    ]),
  ).toEqual(['pointer', 'underline']);
  const link = box.getByRole('link', { name: `Open session ${id}, 23 minutes this day` });
  await expect(link).toHaveText('Session 000000000 h 23 m');
  await expect(link).toHaveAttribute('href', href);
  // Into the box in small steps: it stays open, alone.
  await page.mouse.move(...(await settled(link)), { steps: 15 });
  await expect(link).toBeVisible();
  await expect(shown).toHaveCount(1);
  if (shots) await page.screenshot({ path: `${shots}/dashboard-7d-paired-day-sessions.png` });
  // Back onto the track: the box closes and the day's card returns.
  await page.mouse.move(...(await centre(today.locator('.xt-human-track'), 0.6)), { steps: 15 });
  await expect(box).toBeHidden();
  await expect(shown.getByTestId('agent-day-total')).toBeVisible();
  await expect(shown).toHaveCount(1);
  // And the date again, into the box, to open the session.
  await page.mouse.move(...(await centre(date)), { steps: 5 });
  await expect(link).toBeVisible();
  await page.mouse.move(...(await settled(link)), { steps: 15 });
  await page.mouse.down();
  await page.mouse.up();
  await expect(page).toHaveURL(url);
  // From the keyboard: focus shows the day's card; Enter moves onto its first
  // session, Escape returns to the day, arrows still move between days, and
  // Enter on the session opens it.
  await page.goBack();
  await expect(page.getByTestId('human-timeline')).toHaveAttribute('data-variant', 'paired');
  await page.mouse.move(0, 0);
  // Reached by Tab, as a keyboard user does: the card opens on visible focus.
  await today.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(today).toBeFocused();
  await expect(shown.getByTestId('agent-day-total')).toBeVisible();
  await expect(box).toBeHidden();
  await page.keyboard.press('Enter');
  await expect(link).toBeFocused();
  await expect(page.getByTestId('agent-day-total')).toHaveCount(0);
  await expect(shown).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(today).toBeFocused();
  await expect(box).toBeHidden();
  await expect(shown.getByTestId('agent-day-total')).toBeVisible();
  await page.keyboard.press('ArrowUp');
  await expect(days.nth(-2)).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(today).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(link).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(url);
  expect(errors).toEqual([]);
});

test('in human + agent, a click on a date neither pins nor toggles its sessions box: one box at a time, and moving off closes it', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await open(page, 'light');
  await expect(page.getByTestId('human-timeline')).toHaveAttribute('data-variant', 'paired');
  const days = page.getByTestId('human-day');
  const centre = async (locator: ReturnType<Page['locator']>, across = 0.5) => {
    const box = (await locator.boundingBox())!;
    return [box.x + box.width * across, box.y + box.height / 2] as const;
  };
  const date = (day: string) => page.getByRole('button', { name: `Sessions on ${day}` });
  const boxes = page.getByRole('dialog', { name: /^Sessions on / });
  const shown = page.locator('.xt-effort-tip[data-open]');
  // Hover Sep 6's date, then click it: its box stays open, as hovering keeps it.
  await page.mouse.move(...(await centre(date('Sep 6'))), { steps: 5 });
  await expect(page.getByRole('dialog', { name: 'Sessions on Sep 6' })).toBeVisible();
  await page.mouse.down();
  await page.mouse.up();
  await expect(page.getByRole('dialog', { name: 'Sessions on Sep 6' })).toBeVisible();
  // Hovering Sep 7's date then shows its box alone: the click pinned nothing.
  await page.mouse.move(...(await centre(date('Sep 7'))), { steps: 5 });
  await expect(page.getByRole('dialog', { name: 'Sessions on Sep 7' })).toBeVisible();
  await expect(boxes).toHaveCount(1);
  await expect(shown).toHaveCount(1);
  // Click Sep 7's date, then move onto its track: the box closes and the
  // day's card shows, alone.
  await page.mouse.down();
  await page.mouse.up();
  await expect(page.getByRole('dialog', { name: 'Sessions on Sep 7' })).toBeVisible();
  await page.mouse.move(...(await centre(days.last().locator('.xt-human-track'), 0.6)), {
    steps: 15,
  });
  await expect(boxes).toHaveCount(0);
  await expect(shown.getByTestId('agent-day-total')).toBeVisible();
  await expect(shown).toHaveCount(1);
  // And a click on a date, then off the timeline: nothing stays open.
  await page.mouse.move(...(await centre(date('Sep 7'))), { steps: 5 });
  await expect(page.getByRole('dialog', { name: 'Sessions on Sep 7' })).toBeVisible();
  await page.mouse.down();
  await page.mouse.up();
  await page.mouse.move(0, 0, { steps: 5 });
  await expect(boxes).toHaveCount(0);
  await expect(shown).toHaveCount(0);
  expect(errors).toEqual([]);
});

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  test(`in human + agent, walking the pointer along the dates opens each date’s sessions box in turn, never covering the next date, ${width}x${height}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, 'light', width, height);
    await expect(page.getByTestId('human-timeline')).toHaveAttribute('data-variant', 'paired');
    const boxes = page.getByRole('dialog', { name: /^Sessions on / });
    // Every date in view, top to bottom, with its centre.
    const dates = await page.getByTestId('human-timeline-body').evaluate((body) => {
      const view = body.getBoundingClientRect();
      return [...body.querySelectorAll<HTMLElement>('.xt-human-day')]
        .map((date) => ({ label: date.textContent!, box: date.getBoundingClientRect() }))
        .filter(({ box }) => box.top >= view.top && box.bottom <= view.bottom)
        .map(({ label, box }) => ({
          label,
          x: box.left + box.width / 2,
          y: box.top + box.height / 2,
          top: view.top,
          bottom: view.bottom,
        }));
    });
    expect(dates.length).toBeGreaterThanOrEqual(5);
    // Down the column from its top edge, then up from its bottom edge, in
    // small steps: each date's box opens, alone, as the pointer reaches it.
    for (const order of [dates, [...dates].reverse()]) {
      await page.mouse.move(0, 0);
      await expect(boxes).toHaveCount(0);
      const start = order === dates ? order[0]!.top + 1 : order[0]!.bottom - 1;
      await page.mouse.move(order[0]!.x, start);
      for (const date of order) {
        await page.mouse.move(date.x, date.y, { steps: 13 });
        await expect(page.getByRole('dialog', { name: `Sessions on ${date.label}` })).toBeVisible();
        await expect(boxes).toHaveCount(1);
      }
    }
    expect(errors).toEqual([]);
  });
