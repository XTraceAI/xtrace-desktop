import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { NativeHostStatus } from '../src/data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../src/data/generated/NativeIndexStatus';

/**
 * The welcome page over the generated F1 export, with test-only index statuses
 * served in its place (not real history). F1 itself reports indexed history
 * at startup, so it opens Dashboard; `serve` clears that fact unless asked.
 */
const host = (name: string, patch: Partial<NativeHostStatus> = {}): NativeHostStatus => ({
  host: name,
  state: 'pending',
  detail: null,
  sessions_imported: 0,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 0,
  records_enriched: 0,
  diagnostics: 0,
  ...patch,
});
const base = {
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/usr/bin/python3' },
  readers: { state: 'verified', commit: 'b'.repeat(40), plugin_version: '0.1.0' },
  reconciles: 1,
  files_scanned: 212,
} as const;
const states: Record<string, NativeIndexStatus> = {
  scanning: {
    ...base,
    phase: { phase: 'scanning' },
    freshness: { freshness: 'unknown' },
    python: { state: 'resolving' },
    hosts: ['claude', 'codex', 'cursor'].map((name) => host(name)),
    reconciles: 0,
    files_scanned: 128,
  },
  partial: {
    ...base,
    phase: { phase: 'ready' },
    freshness: {
      freshness: 'degraded',
      reason: 'the watcher could not be registered for one of the history folders',
    },
    python: { state: 'missing', reason: 'python3 was not found on the login shell path' },
    hosts: [
      host('claude', {
        state: 'incomplete',
        sessions_imported: 12_345,
        sessions_partial: 12,
        sessions_skipped: 3,
        skipped_conversations: [],
        skipped_conversations_omitted: 0,
        records_new: 1_234_567,
        diagnostics: 4,
        detail: 'some transcript lines could not be parsed and were skipped',
      }),
      host('codex', { state: 'missing_runtime', detail: 'python3 was not found' }),
      host('cursor', {
        state: 'reader_failed',
        detail:
          'the bundled reader exited with status 1 before it reported any session from this history',
      }),
    ],
  },
  missing: {
    ...base,
    phase: { phase: 'ready' },
    hosts: [
      host('claude', { state: 'complete', sessions_imported: 41, records_new: 900 }),
      host('codex', { state: 'missing_source' }),
      host('cursor', { state: 'missing_source' }),
    ],
  },
  disabled: fixture.native_index as NativeIndexStatus,
};

async function serve(
  page: Page,
  native: NativeIndexStatus,
  history = false,
  counts?: FixtureExport['db_counts'],
) {
  const exported = structuredClone(fixture as FixtureExport);
  exported.app_info.had_indexed_history_at_startup = history;
  exported.native_index = native;
  if (counts) exported.db_counts = counts;
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(exported)};`,
    }),
  );
}
const heading = (page: Page) => page.getByRole('heading', { level: 1 });

test('a fresh startup opens welcome with the real counts; a deep link is kept', async ({
  page,
}) => {
  await serve(page, states.scanning);
  await page.goto('/');
  await expect(page).toHaveURL(/\/first-launch$/);
  await expect(heading(page)).toHaveText('Welcome to XTrace');
  const counts = page.getByRole('group', { name: 'In the local database now' });
  await expect(counts).toContainText(String(fixture.db_counts.sessions));
  await expect(counts).toContainText(String(fixture.db_counts.records));
  await expect(page.getByTestId('welcome-files')).toHaveText('128');
  await page.goto('/sessions');
  await expect(heading(page)).toHaveText('Sessions');
  await expect(page).toHaveURL(/\/sessions$/);
});

test('existing indexed history opens Dashboard', async ({ page }) => {
  await serve(page, states.missing, true);
  await page.goto('/');
  await expect(page).toHaveURL(/\/dashboard$/);
  await expect(heading(page)).toHaveText('What your agents did');
});

test('continue by keyboard is remembered across a restart', async ({ page }) => {
  await serve(page, states.missing);
  await page.goto('/');
  await expect(heading(page)).toHaveText('Welcome to XTrace');
  const cta = page.getByRole('button', { name: 'Open Dashboard' });
  for (let step = 0; step < 30; step++) {
    if (await cta.evaluate((element) => element === document.activeElement)) break;
    await page.keyboard.press('Tab');
  }
  await expect(cta).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/dashboard$/);
  await page.goto('/');
  await expect(page).toHaveURL(/\/dashboard$/);
});

test('a storage write failure still leaves for Dashboard, and welcome returns next time', async ({
  page,
}) => {
  await page.addInitScript(() => {
    Storage.prototype.setItem = () => {
      throw new DOMException('quota', 'QuotaExceededError');
    };
  });
  await serve(page, states.missing);
  await page.goto('/');
  await page.getByRole('button', { name: 'Open Dashboard' }).click();
  await expect(page).toHaveURL(/\/dashboard$/);
  await page.goto('/');
  await expect(page).toHaveURL(/\/first-launch$/);
});

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    for (const state of Object.keys(states))
      test(`welcome fits at ${width}x${height} ${scheme} while ${state}`, async ({
        page,
      }, info) => {
        const errors: string[] = [];
        page.on('pageerror', (error) => errors.push(error.message));
        await serve(page, states[state]);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto('/first-launch');
        await expect(heading(page)).toHaveText('Welcome to XTrace');
        await expect(page.getByRole('button', { name: 'Open Dashboard' })).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
        const layout = await page.evaluate(() => {
          const root = document.querySelector('.xt-welcome')!;
          const box = root.getBoundingClientRect();
          const escaped = [...root.querySelectorAll<HTMLElement>('*')]
            .filter((element) => {
              const rect = element.getBoundingClientRect();
              return rect.width > 0 && (rect.left < box.left - 0.5 || rect.right > box.right + 0.5);
            })
            .map((element) => element.className);
          const cards = [...root.querySelectorAll('.xt-section-card')].map((card) => {
            const rect = card.getBoundingClientRect();
            const inner = [...card.querySelectorAll<HTMLElement>('*')].filter((element) => {
              const r = element.getBoundingClientRect();
              return r.width > 0 && (r.left < rect.left - 0.5 || r.right > rect.right + 0.5);
            });
            return inner.length;
          });
          const clipped = [
            ...root.querySelectorAll<HTMLElement>('.xt-welcome-host, .xt-welcome-count'),
          ].filter((element) => element.scrollWidth > element.clientWidth + 1).length;
          return {
            page: document.documentElement.scrollWidth,
            outlet: (() => {
              const outlet = document.querySelector('.xt-shell-outlet')!;
              return outlet.scrollWidth - outlet.clientWidth;
            })(),
            escaped,
            cards,
            clipped,
          };
        });
        expect(layout.page).toBe(width);
        expect(layout.outlet).toBe(0);
        expect(layout.escaped).toEqual([]);
        expect(layout.cards.every((count) => count === 0)).toBe(true);
        expect(layout.clipped).toBe(0);
        await page.screenshot({
          path: info.outputPath(`welcome-${state}-${width}-${scheme}.png`),
        });
        expect(errors).toEqual([]);
      });

/** Test-only counts: plausible indexed-history volumes, and the exact JSON integer limit. */
const largeCounts: Record<string, FixtureExport['db_counts']> = {
  large: { sessions: 12_345, records: 1_234_567, usage: 123_456 },
  limit: {
    sessions: Number.MAX_SAFE_INTEGER,
    records: Number.MAX_SAFE_INTEGER,
    usage: Number.MAX_SAFE_INTEGER,
  },
};
for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    for (const [name, counts] of Object.entries(largeCounts))
      test(`${name} database counts stay whole at ${width}x${height} ${scheme}`, async ({
        page,
      }, info) => {
        await serve(page, states.partial, false, counts);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto('/first-launch');
        const group = page.getByRole('group', { name: 'In the local database now' });
        await expect(group).toContainText(counts.records.toLocaleString('en-US'));
        await page.evaluate(() => document.fonts.ready);
        const cells = await group.evaluate((root) =>
          [...root.querySelectorAll<HTMLElement>('.xt-welcome-count')].map((tile) => {
            const cell = tile.querySelector<HTMLElement>('.xt-metric-cell')!;
            const label = tile.querySelector<HTMLElement>('.xt-welcome-count-label')!;
            const range = document.createRange();
            range.selectNodeContents(cell);
            const text = range.getBoundingClientRect();
            const box = tile.getBoundingClientRect();
            const own = cell.getBoundingClientRect();
            const style = getComputedStyle(cell);
            return {
              text: cell.textContent,
              // The glyphs themselves, not the outer card: inside the span, the span inside its tile.
              overflow: cell.scrollWidth - cell.clientWidth,
              glyphsInside:
                text.left >= own.left - 0.5 &&
                text.right <= own.right + 0.5 &&
                own.left >= box.left - 0.5 &&
                own.right <= box.right + 0.5,
              ellipsis: style.textOverflow === 'ellipsis' && style.overflow !== 'visible',
              overlapsLabel:
                label.getBoundingClientRect().right > own.left + 0.5 &&
                label.getBoundingClientRect().bottom > own.top + 0.5,
            };
          }),
        );
        expect(cells.map((cell) => cell.text)).toEqual(
          [counts.sessions, counts.records, counts.usage].map((value) =>
            value.toLocaleString('en-US'),
          ),
        );
        for (const cell of cells) {
          expect(cell.overflow).toBeLessThanOrEqual(0);
          expect(cell.glyphsInside).toBe(true);
          expect(cell.ellipsis).toBe(false);
          expect(cell.overlapsLabel).toBe(false);
        }
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        await expect(page.getByRole('button', { name: 'Open Dashboard' })).toBeVisible();
        await group.screenshot({ path: info.outputPath(`counts-${name}-${width}-${scheme}.png`) });
      });
