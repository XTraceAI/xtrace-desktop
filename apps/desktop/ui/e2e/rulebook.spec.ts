import { expect, test, type Page } from '@playwright/test';

test.use({ viewport: { width: 1120, height: 720 } });

const routes = [
  ['/rulebook', 'Rulebook', 'overview'],
  ['/rulebook/fires', 'Recorded rule activity', 'fires'],
  ['/rulebook/not-a-real-rule', 'Rule detail', 'rule'],
] as const;

/**
 * The page fits the window: nothing scrolls the page or the outlet, nothing
 * runs past the page's right edge, the last panel meets the page's bottom
 * padding, and every list scrolls inside its panel with its last row
 * reachable and no sideways scroll.
 */
const assertFits = async (page: Page) => {
  const geometry = await page.evaluate(() => {
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    const root = document.querySelector<HTMLElement>('.xt-rulebook')!;
    const box = (element: Element) => element.getBoundingClientRect();
    const right = box(root).right + 0.5;
    // Content inside a list's own scroll area is judged by the list below.
    const outside = root.querySelectorAll(':not(.xt-table-scroll *)');
    const past = [...outside].filter((element) => box(element).right > right).length;
    const lists = [...root.querySelectorAll<HTMLElement>('.xt-table-scroll')].map((list) => {
      list.scrollTop = list.scrollHeight;
      const rows = list.querySelectorAll('[role="row"]');
      const last = rows[rows.length - 1]!;
      const reached = box(last).bottom <= box(list).bottom + 1 && box(last).top >= box(list).top;
      const sideways = list.scrollWidth - list.clientWidth;
      list.scrollTop = 0;
      return { reached, sideways, visible: box(list).bottom <= window.innerHeight };
    });
    const cards = [...root.querySelectorAll(':scope > .xt-section-card, .xt-rulebook-row')];
    return {
      page: document.documentElement.scrollHeight - document.documentElement.clientHeight,
      width: document.documentElement.scrollWidth,
      outlet: outlet.scrollHeight - outlet.clientHeight,
      spare: Math.round(box(outlet).bottom - box(cards.at(-1)!).bottom),
      past,
      lists: lists.every((list) => list.reached && list.sideways === 0 && list.visible),
    };
  });
  expect(geometry).toEqual({
    page: 0,
    width: page.viewportSize()!.width,
    outlet: 0,
    spare: 12,
    past: 0,
    lists: true,
  });
};
const status = (page: Page) => page.locator('.xt-rulebook-read [role="status"]');
type Calls = { reads: string[]; cancels: string[] };
/** Every read and cancel the harness's rule activity controls received. */
const calls = (page: Page) =>
  page.evaluate(() => (window as unknown as { __ruleActivity: Calls }).__ruleActivity);
const harness = (scenario: string, path = '/rulebook') =>
  `/e2e/rulebook.html?scenario=${scenario}&path=${encodeURIComponent(path)}`;

test('fits every Rulebook address in both themes on the fixture’s own answer', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.name));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push('console error');
  });
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    for (const [path, title, view] of routes) {
      await page.goto(path);
      await expect(page.getByRole('heading', { level: 1 })).toHaveText(title);
      await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
      await expect(page.getByRole('button', { name: 'Rulebook', exact: true })).toHaveAttribute(
        'aria-current',
        'page',
      );
      await expect(page.getByText('This view is coming next.')).toHaveCount(0);
      const root = page.locator('.xt-rulebook');
      await expect(root).toHaveAttribute('data-view', view);
      if (view !== 'rule') {
        // F1 has no rulebook source: said so, with no count or row.
        await expect(status(page)).toHaveText('Source unavailable');
        await expect(root.locator('.xt-rulebook-read .xt-rulebook-detail')).toContainText(
          'not configured on this device',
        );
        await expect(root.getByRole('table')).toHaveCount(0);
        await expect(root.locator('.xt-stat-value:not(:has(.xt-unmeasured))')).toHaveCount(0);
      }
      await assertFits(page);
      await page.mouse.move(1100, 700);
      await page.screenshot({ path: info.outputPath(`rulebook-${view}-${scheme}.png`) });
    }
  }
  expect(errors).toEqual([]);
});

test('lays the overview out as the reference: read state, four tiles, a note, then two panels', async ({
  page,
}) => {
  await page.goto('/rulebook');
  await expect(status(page)).toHaveText('Source unavailable');
  const tiles = page.locator('.xt-rulebook-tiles .xt-stat-tile');
  await expect(tiles).toHaveCount(4);
  const tops = await tiles.evaluateAll((all) => all.map((tile) => tile.getBoundingClientRect().y));
  expect(new Set(tops).size).toBe(1);
  // Every label and aside is shown whole, never cut to an ellipsis.
  const cut = await page
    .locator('.xt-rulebook-tiles .xt-stat-label, .xt-rulebook-tiles .xt-stat-aside')
    .evaluateAll((all) => all.filter((text) => text.scrollWidth > text.clientWidth).length);
  expect(cut).toBe(0);
  const headings = page.locator('.xt-rulebook h2');
  await expect(headings).toHaveText(['Observed groups', 'Recorded timeline']);
  const order = await page.evaluate(() =>
    [...document.querySelector('.xt-rulebook')!.children].map((child) => child.className),
  );
  expect(order).toEqual([
    'xt-rulebook-heading',
    'xt-rulebook-read',
    'xt-rulebook-tiles',
    'xt-rulebook-note',
    'xt-rulebook-row',
  ]);
  const [groups, fires] = await headings.evaluateAll((all) =>
    all.map((heading) => heading.closest('.xt-section-card')!.getBoundingClientRect().toJSON()),
  );
  expect(groups.top).toBe(fires.top);
  expect(groups.height).toBe(fires.height);
  expect(groups.right).toBeLessThan(fires.left);
});

test('lays out every answer at both window sizes in both themes without clipping', async ({
  page,
}, info) => {
  test.slow();
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.name));
  // Each answer's status line, and the Recorded fires tile's aside beside it.
  const scenarios = [
    ['populated', /^Read complete · 4 recorded fires/, '14d · exact'],
    ['lowerBound', /^Read complete · ≥ 4 recorded fires/, '14d · lower bound'],
    ['emptyExact', /^Read complete · 0 recorded fires/, '14d · exact'],
    ['emptyLowerBound', /^Read complete · ≥ 0 recorded fires/, '14d · lower bound'],
    ['truncated', /^Read complete · 220 recorded fires in 140 observed groups/, '14d · exact'],
    [
      'lowerBoundWide',
      /^Read complete · ≥ 10,000 recorded fires in ≥ 1 observed groups/,
      '14d · lower bound',
    ],
    ['pastCap', /^Read complete · 220 recorded fires in 140 observed groups/, '14d · exact'],
    ['identities', /^Read complete · 252 recorded fires in 8 observed groups/, '14d · exact'],
    ['unavailable', /^Source unavailable$/, 'unavailable'],
    ['sourceChanged', /^The source changed during the read$/, 'changed'],
    ['deadline', /^The read ran out of time$/, 'timed out'],
    ['busy', /^Another rule activity read is still running$/, 'busy'],
    ['failed', /^Rule activity could not be read$/, 'failed'],
  ] as const;
  for (const [width, height] of [
    [1120, 720],
    [1440, 900],
  ] as const) {
    await page.setViewportSize({ width, height });
    for (const scheme of ['dark', 'light'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      for (const [scenario, text, aside] of scenarios)
        for (const path of ['/rulebook', '/rulebook/fires']) {
          await page.goto(harness(scenario, path));
          await expect(status(page)).toHaveText(text);
          await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
          if (path === '/rulebook') {
            const fires = page.locator('.xt-rulebook-tiles .xt-stat-tile').first();
            await expect(fires.locator('.xt-stat-aside')).toHaveText(aside);
            await expect(fires).not.toContainText(/not read/i);
            // Without a snapshot every aside is shown whole. (A loaded
            // snapshot's aside is not held to this here.)
            if (!aside.startsWith('14d')) {
              const cutAsides = await page
                .locator('.xt-rulebook-tiles .xt-stat-aside')
                .evaluateAll(
                  (all) => all.filter((text) => text.scrollWidth > text.clientWidth).length,
                );
              expect(cutAsides).toBe(0);
            }
          }
          await assertFits(page);
          // One read per activation, even under the development double mount.
          expect((await calls(page)).reads).toHaveLength(1);
          // Every group count is shown whole, the widest lower bound included.
          const counts = page.locator('.xt-rulebook .xt-data-row .xt-metric-cell');
          const cut = await counts.evaluateAll(
            (all) => all.filter((cell) => cell.scrollWidth > cell.clientWidth).length,
          );
          expect(cut).toBe(0);
          if (scenario === 'lowerBoundWide' && path === '/rulebook')
            await expect(counts.first()).toHaveText('≥ 10,000');
          // A row's position number and ID control are never cut; at the
          // reference size its words are shown whole too.
          const rules = await page.locator('.xt-rulebook-rule').evaluateAll((all) =>
            all.map((rule) => {
              const cell = rule.closest('[role="cell"]')!.getBoundingClientRect();
              const within = (element: Element | null) => {
                if (!element) return true;
                const box = element.getBoundingClientRect();
                return box.left >= cell.left - 0.5 && box.right <= cell.right + 0.5;
              };
              const words = rule.querySelector<HTMLElement>(':scope > span > span')!;
              return {
                number: within(rule.querySelector('.xt-rulebook-position')),
                control: within(rule.querySelector('.xt-rulebook-identity')),
                words: words.scrollWidth <= words.clientWidth,
              };
            }),
          );
          expect(rules.filter((rule) => !rule.number || !rule.control)).toEqual([]);
          if (width === 1440) expect(rules.filter((rule) => !rule.words)).toEqual([]);
          if (['truncated', 'populated', 'lowerBoundWide', 'identities'].includes(scenario))
            await page.screenshot({
              path: info.outputPath(
                `rulebook-${scenario}-${path === '/rulebook' ? 'overview' : 'fires'}-${width}-${scheme}.png`,
              ),
            });
        }
    }
  }
  expect(errors).toEqual([]);
});

test('shows the returned window and the same bounded timeline on both views', async ({ page }) => {
  await page.goto(harness('truncated'));
  const source = page.locator('.xt-rulebook-source');
  await expect(source).toHaveText(
    'Default local rulebook source · window 2026-09-09T09:00:00Z to 2026-09-23T09:00:00Z · read 2026-09-23T09:00:00Z · 225 lines scanned',
  );
  const timeline = page.getByRole('table', { name: 'Recorded timeline' });
  const groups = page.getByRole('table', { name: 'Observed rule groups' });
  await expect(timeline.locator('.xt-data-row')).toHaveCount(100);
  await expect(groups.locator('.xt-data-row')).toHaveCount(100);
  await expect(page.locator('.xt-rulebook-truncated')).toHaveText([
    'Showing 100 of 140 observed groups; every group is counted above.',
    'Showing the latest 100 of 225 recorded rows in the window.',
  ]);
  const times = timeline.locator('time');
  const stamps = () => times.evaluateAll((all) => all.map((time) => time.getAttribute('datetime')));
  const overview = await stamps();
  await page.goto(harness('truncated', '/rulebook/fires'));
  await expect(source).toHaveText(/window 2026-09-09T09:00:00Z to 2026-09-23T09:00:00Z/);
  await expect(timeline.locator('.xt-data-row')).toHaveCount(100);
  expect(await stamps()).toEqual(overview);
  await expect(page.getByRole('link', { name: /View all/i })).toHaveCount(0);
  await expect(page.locator('.xt-rulebook').getByRole('link')).toHaveText(['← Rulebook']);
});

test('cancels on request and on leaving, keeping focus and asking again only by hand', async ({
  page,
}) => {
  await page.goto(harness('hold'));
  await expect(status(page)).toHaveText('Reading the default local rulebook source…');
  // The Recorded fires tile says a read is running, then that it was cancelled.
  const firesUnmeasured = page.locator('.xt-rulebook-tiles .xt-stat-tile').first();
  await expect(firesUnmeasured.locator('.xt-stat-aside')).toHaveText('reading');
  await expect(firesUnmeasured.locator('.xt-unmeasured .sr-only')).toHaveText(
    'Unmeasured: Reading the default local rulebook source…',
  );
  const refresh = page.locator('.xt-rulebook-read').getByRole('button', { name: 'Refresh' });
  const cancel = page.locator('.xt-rulebook-read').getByRole('button', { name: /Cancel|Stopping/ });
  await expect(refresh).toHaveAttribute('aria-disabled', 'true');
  await refresh.focus();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Tab');
  await expect(cancel).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(status(page)).toHaveText('Read cancelled');
  await expect(firesUnmeasured.locator('.xt-stat-aside')).toHaveText('cancelled');
  await expect(cancel).toBeFocused();
  await expect(cancel).toHaveAttribute('aria-disabled', 'true');
  let seen = await calls(page);
  expect(seen.reads).toHaveLength(1);
  expect(seen.cancels).toEqual(seen.reads);

  await page.keyboard.press('Shift+Tab');
  await expect(refresh).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(status(page)).toHaveText('Reading the default local rulebook source…');
  await expect(refresh).toBeFocused();
  seen = await calls(page);
  expect(seen.reads).toHaveLength(2);

  // Leaving cancels the read by its own name; the timeline is its own read.
  await page.getByRole('button', { name: 'Dashboard', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
  seen = await calls(page);
  expect(seen.cancels).toEqual(seen.reads);
  expect(seen.reads).toHaveLength(2);

  await page.goto(harness('hold', '/rulebook/fires'));
  await expect(status(page)).toHaveText('Reading the default local rulebook source…');
  await page.getByRole('link', { name: '← Rulebook' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rulebook');
  await expect(status(page)).toHaveText('Reading the default local rulebook source…');
  seen = await calls(page);
  expect(seen.reads).toHaveLength(2);
  expect(seen.cancels).toEqual([seen.reads[0]]);

  // An unmatched rule address reads nothing.
  await page.goto(harness('hold', '/rulebook/not-a-real-rule'));
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rule detail');
  await page.waitForTimeout(50);
  expect((await calls(page)).reads).toHaveLength(0);
});

test('moves focus through the controls, tiles and lists with a visible ring', async ({ page }) => {
  await page.goto(harness('truncated'));
  await expect(status(page)).toHaveText(/^Read complete/);
  const refresh = page.locator('.xt-rulebook-read').getByRole('button', { name: 'Refresh' });
  await refresh.focus();
  const ring = () =>
    page.evaluate(() => {
      const style = getComputedStyle(document.activeElement!);
      return style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) > 0;
    });
  const steps = [
    '.xt-rulebook-read .xt-button[data-variant="ghost"]',
    '.xt-rulebook-tiles .xt-stat-tile >> nth=0',
    '.xt-rulebook-tiles .xt-stat-tile >> nth=1',
    '.xt-rulebook-tiles .xt-stat-tile >> nth=2',
    '.xt-rulebook-tiles .xt-stat-tile >> nth=3',
  ];
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(refresh).toBeFocused();
  expect(await ring()).toBe(true);
  for (const step of steps) {
    await page.keyboard.press('Tab');
    await expect(page.locator(step)).toBeFocused();
  }
  await page.keyboard.press('Escape');
  // A focused list scrolls from the keyboard; then each row's ID control is
  // one stop, showing its IDs until Escape, which leaves focus in place.
  const tooltip = page.getByRole('tooltip');
  await page.keyboard.press('Tab');
  await expect(
    page.getByRole('region', { name: 'Observed rule groups scroll area' }),
  ).toBeFocused();
  expect(await ring()).toBe(true);
  await expect(tooltip).toHaveCount(0);
  for (const position of [1, 2]) {
    await page.keyboard.press('Tab');
    const control = page.getByRole('button', {
      name: `ID of observed group ${position}`,
      exact: true,
    });
    await expect(control).toBeFocused();
    expect(await ring()).toBe(true);
    await expect(tooltip).toContainText(`Rule IDsynthetic-rule-${position - 1}`);
  }
  await page.keyboard.press('Escape');
  await expect(tooltip).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'ID of observed group 2', exact: true }),
  ).toBeFocused();
  // The timeline's list comes before its first row's control.
  await page.getByRole('button', { name: 'ID of timeline row 1, observed group 1' }).focus();
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByRole('region', { name: 'Recorded timeline scroll area' })).toBeFocused();
  expect(await ring()).toBe(true);
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('shows each row’s full IDs on hover and focus, over lists scrolled inside their panels', async ({
  page,
}, info) => {
  test.slow();
  const tooltip = page.getByRole('tooltip');
  /** The tooltip lies wholly in the window, drawn on top, outside every list. */
  const onTop = () =>
    tooltip.evaluate((element) => {
      const box = element.getBoundingClientRect();
      // The tooltip takes no pointer; hit-test it for a moment to find what is on top.
      const passive = [element, element.parentElement!];
      for (const layer of passive) layer.style.pointerEvents = 'auto';
      const top = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      for (const layer of passive) layer.style.pointerEvents = '';
      const values = [...element.querySelectorAll('dd')];
      return {
        inside:
          box.left >= 0 &&
          box.top >= 0 &&
          box.right <= window.innerWidth &&
          box.bottom <= window.innerHeight,
        top: element.contains(top),
        list: element.closest('.xt-table-scroll') !== null,
        wrapped: values.every((value) => value.scrollWidth <= value.clientWidth),
      };
    });
  const expected = { inside: true, top: true, list: false, wrapped: true };
  const rule = (index: number) =>
    `${'a'.repeat(8)}-0000-4000-8000-${index.toString(16).padStart(12, '0')}`;
  const bookA = `${'b'.repeat(8)}-0000-4000-8000-00000000000a`;
  for (const [width, height] of [
    [1120, 720],
    [1440, 900],
  ] as const) {
    await page.setViewportSize({ width, height });
    for (const scheme of ['dark', 'light'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto(harness('identities'));
      await expect(status(page)).toHaveText(/^Read complete · 252 recorded fires/);
      const groups = page.getByRole('table', { name: 'Observed rule groups' });
      const timeline = page.getByRole('table', { name: 'Recorded timeline' });
      await expect(groups.locator('.xt-title-cell > span')).toHaveText(
        Array.from({ length: 8 }, (_, index) => `Observed group ${index + 1}`),
      );
      // No raw ID is in the rows' visible text.
      expect(await page.locator('.xt-rulebook').innerText()).not.toMatch(/[abf]{8}-0000/);

      // Pointer: a group's control, named and described by its full IDs.
      const third = groups.getByRole('button', { name: 'ID of observed group 3', exact: true });
      await expect(third).toHaveAccessibleDescription(`Rule ID ${rule(2)}. Rulebook ID ${bookA}`);
      await third.hover();
      await expect(tooltip).toBeVisible();
      await expect(tooltip).toContainText(`Rule ID${rule(2)}Rulebook ID${bookA}`);
      expect(await onTop()).toEqual(expected);
      await page.mouse.move(width - 4, 4);
      await expect(tooltip).toHaveCount(0);

      // The timeline's last row, reached by scrolling its list, then by pointer.
      const list = page.getByRole('region', { name: 'Recorded timeline scroll area' });
      await list.evaluate((element) => (element.scrollTop = element.scrollHeight));
      const last = timeline.getByRole('button', {
        name: 'ID of timeline row 100, observed group 4',
      });
      // The 100th of 252 rows, oldest first numbered 1: fire 0x99, version 2.
      await expect(last).toHaveAccessibleDescription(
        `Rule ID ${rule(3)}. Rulebook ID ${bookA}. Fire ID ${'f'.repeat(8)}-0000-4000-8000-000000000099. Recorded version 2`,
      );
      await last.hover();
      await expect(tooltip).toBeVisible();
      await expect(tooltip).toContainText('Fire ID');
      await expect(tooltip).toContainText('Recorded version');
      expect(await onTop()).toEqual(expected);
      await page.screenshot({ path: info.outputPath(`rulebook-identity-${width}-${scheme}.png`) });
      await page.mouse.move(width - 4, 4);
      await expect(tooltip).toHaveCount(0);

      // Keyboard: focus shows it in place; Escape closes it and keeps focus.
      await last.focus();
      await expect(tooltip).toBeVisible();
      expect(await onTop()).toEqual(expected);
      await page.keyboard.press('Escape');
      await expect(tooltip).toHaveCount(0);
      await expect(last).toBeFocused();
      await assertFits(page);
    }
  }

  // A listed fire whose group is past the returned 100 gets no number.
  await page.goto(harness('pastCap'));
  await expect(status(page)).toHaveText(/^Read complete/);
  const timeline = page.getByRole('table', { name: 'Recorded timeline' });
  await expect(timeline.locator('.xt-title-cell > span').last()).toHaveText('Recorded rule');
  await expect(
    timeline.getByRole('button', { name: 'ID of timeline row 100, recorded rule' }),
  ).toHaveAccessibleDescription(
    'Rule ID synthetic-rule-99. Rulebook ID synthetic-book. Fire ID fire-99. Recorded version 2',
  );
});

test('fits the reference window too', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const [path, title] of routes) {
    await page.goto(path);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(title);
    const overflow = await page.evaluate(() => {
      const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
      return {
        page: document.documentElement.scrollHeight - document.documentElement.clientHeight,
        outlet: outlet.scrollHeight - outlet.clientHeight,
        width: document.documentElement.scrollWidth,
      };
    });
    expect(overflow).toEqual({ page: 0, outlet: 0, width: 1440 });
  }
});

test('keeps direct addresses and Back navigation working', async ({ page }) => {
  await page.goto('/dashboard');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
  await page.getByRole('button', { name: 'Rulebook', exact: true }).click();
  await expect(page).toHaveURL(/\/rulebook$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rulebook');
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');

  await page.goto('/rulebook/fires');
  await expect(page.getByRole('navigation', { name: 'Breadcrumb' })).toContainText('fires');
  await page.getByRole('link', { name: '← Rulebook' }).click();
  await expect(page).toHaveURL(/\/rulebook$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rulebook');
  await page.goBack();
  await expect(page).toHaveURL(/\/rulebook\/fires$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Recorded rule activity');

  await page.goto('/rulebook/not-a-real-rule');
  const crumb = page.getByRole('navigation', { name: 'Breadcrumb' });
  await expect(crumb).toContainText('rule');
  await expect(crumb).not.toContainText('not-a-real-rule');
  await expect(page.locator('.xt-rulebook-address span')).toHaveText('/rulebook/not-a-real-rule');
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Rule detail');
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Recorded rule activity');
});
