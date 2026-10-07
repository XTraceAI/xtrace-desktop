import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { NativeHostState } from '../src/data/generated/NativeHostState';
import type { NativeIndexStatus } from '../src/data/generated/NativeIndexStatus';

/**
 * Settings → Native index over a test-only synthetic status served in place of
 * the F1 export's (not real history): the note that says what a host's scan
 * state covers sits under the host rows inside the card at 1120×720 in both
 * schemes, in the card's meta ink, adds no tab stop, and leaves every reported
 * row as it was.
 */
const scan = (host: string, state: NativeHostState, imported: number) => ({
  host,
  state,
  needs_attention: state !== 'complete',
  detail: null,
  sessions_imported: imported,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 3,
  records_enriched: 0,
  diagnostics: 0,
});
const status: NativeIndexStatus = {
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/synthetic/python3' },
  readers: { state: 'verified', commit: '0'.repeat(40), plugin_version: '0.0.0' },
  hosts: [
    scan('claude', 'complete', 2),
    scan('codex', 'incomplete', 1),
    scan('cursor', 'complete', 4),
  ],
  needs_attention: true,
  reconciles: 2,
  files_scanned: 3,
};

async function open(page: Page, indexStatus = status) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify({ ...(fixture as FixtureExport), native_index: indexStatus })};`,
    }),
  );
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto('/settings');
  await page.evaluate(() => document.fonts.ready);
}

for (const scheme of ['dark', 'light'] as const)
  test(`skipped details open by keyboard and fit the host row (${scheme})`, async ({
    page,
  }, info) => {
    const id = '00000000-0000-4000-8000-000000000001';
    await page.emulateMedia({ colorScheme: scheme });
    await open(page, {
      ...status,
      hosts: [
        {
          ...status.hosts[0],
          state: 'incomplete',
          sessions_skipped: 6,
          skipped_conversations: [
            { conversation_id: id, reason: 'invalid_transcript' },
            { conversation_id: null, reason: 'unknown' },
          ],
          skipped_conversations_omitted: 4,
        },
      ],
    });
    const summary = page.getByText('Skipped conversation details (claude)');
    const details = page.locator('.xt-skipped-conversations');
    const row = details.getByText(`${id} — Conversation data has an invalid format.`);
    await expect(row).toBeHidden();
    await summary.focus();
    await summary.press('Enter');
    await expect(row).toBeVisible();
    await expect(details.getByText('ID unavailable — Reason unavailable.')).toBeVisible();
    await expect(
      details.getByText('Showing 2 of 6 skipped conversations. 4 more omitted.'),
    ).toBeVisible();
    expect(await details.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
      true,
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1120);
    await details.screenshot({ path: info.outputPath(`skipped-details-expanded-${scheme}.png`) });
    await summary.press('Space');
    await expect(row).toBeHidden();
  });

for (const scheme of ['dark', 'light'] as const)
  test(`the host scan scope note fits under the host rows at 1120×720 (${scheme})`, async ({
    page,
  }, info) => {
    await page.emulateMedia({ colorScheme: scheme });
    await open(page);
    const list = page.getByTestId('native-index');
    const note = page.getByTestId('native-index-scope');
    await expect(note).toHaveText(
      "“Read” and “Read with gaps” describe each host's last scan of the sources this index reads, not all of that host's history. Conversations kept only in the Cursor IDE's database are not read, so they are not in Cursor's counts.",
    );
    // The reported rows are unchanged.
    expect(await list.locator('dd').allTextContents()).toEqual([
      'Ready (2 reconciliations)',
      'Live: changes are reconciled as they happen',
      '/synthetic/python3',
      `Bundled memhub 0.0.0 at ${'0'.repeat(12)}`,
      'Read · 2 imported, 0 partial, 0 skipped, 3 new records, 0 enriched',
      'Read with gaps · 1 imported, 0 partial, 0 skipped, 3 new records, 0 enriched',
      'Read · 4 imported, 0 partial, 0 skipped, 3 new records, 0 enriched',
    ]);
    await note.scrollIntoViewIfNeeded();
    await expect(note).toBeInViewport();
    const card = page.locator('.xt-section-card').filter({ has: list });
    const [cardBox, listBox, noteBox] = await Promise.all(
      [card, list, note].map(async (locator) => (await locator.boundingBox())!),
    );
    // Below the last host row, inside the card, never wider than the window.
    expect(noteBox.y).toBeGreaterThanOrEqual(listBox.y + listBox.height);
    expect(noteBox.x).toBeGreaterThanOrEqual(cardBox.x);
    expect(noteBox.x + noteBox.width).toBeLessThanOrEqual(cardBox.x + cardBox.width);
    expect(noteBox.y + noteBox.height).toBeLessThanOrEqual(cardBox.y + cardBox.height);
    expect(await note.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1120);
    // The same quiet ink as the page's other notes.
    const meta = await page.evaluate(() => {
      const probe = document.createElement('span');
      probe.style.color = 'var(--meta)';
      document.querySelector('.xt-settings')!.append(probe);
      const { color } = getComputedStyle(probe);
      probe.remove();
      return color;
    });
    await expect(note).toHaveCSS('color', meta);
    await expect(note).toHaveCSS('font-size', '12px');
    // Keyboard: the note is plain text and adds no tab stop.
    expect(await note.locator('a, button, input, select, [tabindex]').count()).toBe(0);
    await page.screenshot({ path: info.outputPath(`settings-index-scope-${scheme}.png`) });
  });
