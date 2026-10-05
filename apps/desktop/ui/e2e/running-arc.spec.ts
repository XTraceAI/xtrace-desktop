import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

// Synthetic live capability only: the real Vite Dashboard renders the rows,
// glyph assets and CSS, without native IPC or a database. Adapted from the
// live-badge browser probe; two idle rows sit beside the running row.
async function serve(page: Page, host: 'claude' | 'codex') {
  const out = structuredClone(fixture as FixtureExport);
  const ids = Array.from(
    { length: 3 },
    (_, i) => `${host}-00000000-0000-4000-8000-${String(i + 1).padStart(12, '0')}`,
  );
  for (const report of out.dashboards) {
    const base = report.lane_sessions[0];
    report.lanes = ids.map((session_id, i) => ({
      session_id,
      host: i === 1 ? (host === 'claude' ? 'codex' : 'claude') : host,
      start_ms: report.lane_end_ms - (i + 2) * 1000,
      end_ms: report.lane_end_ms - (i + 1) * 1000,
    }));
    report.lane_sessions = ids.map((session_id, i) => ({
      ...base,
      session_id,
      host: i === 1 ? (host === 'claude' ? 'codex' : 'claude') : host,
      parent: null,
      title: `Synthetic chat ${i + 1}`,
    }));
    report.lanes_total = ids.length;
    report.lanes_truncated = false;
  }
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(out)};`,
    }),
  );
  await page.route('**/src/data/FixtureDataSource.ts', async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body: `${await response.text()}
        let arcSerial = 0;
        const arcLeases = new Set();
        FixtureDataSource.prototype.liveSessions = {
          read: async (ids, token) => {
            if (token === null) {
              const view_id = 'arc-fixture-' + (++arcSerial);
              arcLeases.add(view_id);
              return { view_id, states: [] };
            }
            if (!arcLeases.has(token)) throw Error('Expired fixture lease');
            return { view_id: token, states: ids.map(id => ({
              id,
              status: id === ${JSON.stringify(ids[0])} &&
                document.documentElement.dataset.arcIdle !== 'true' ? 'running' : 'idle'
            })) };
          },
          release: async token => { arcLeases.delete(token); }
        };
      `,
    });
  });
}

async function rows(page: Page) {
  return page.getByRole('table', { name: 'Session lanes', exact: true }).evaluate((table) =>
    [...table.querySelectorAll<HTMLElement>('.xt-data-row')].map((row) => {
      const box = row.getBoundingClientRect();
      const glyph = row.querySelector('.xt-host-glyph-mark')!.getBoundingClientRect();
      return {
        top: box.top,
        height: box.height,
        glyphX: glyph.x + glyph.width / 2,
        glyphY: glyph.y + glyph.height / 2 - box.top,
        columns: [...row.querySelectorAll('[role="cell"]')].map((cell) => {
          const rect = cell.getBoundingClientRect();
          return { x: rect.x, width: rect.width };
        }),
      };
    }),
  );
}

async function arc(page: Page) {
  return page.locator('.xt-lane-live-host').evaluate((host) => {
    const box = host.getBoundingClientRect();
    const row = host.closest('.xt-data-row')!.getBoundingClientRect();
    const paint = getComputedStyle(host, '::before');
    const number = (value: string) => parseFloat(value);
    const border = number(paint.borderTopWidth);
    const width = number(paint.width) + (paint.boxSizing === 'border-box' ? 0 : 2 * border);
    const height = number(paint.height) + (paint.boxSizing === 'border-box' ? 0 : 2 * border);
    const cx = box.left + number(paint.left) + width / 2;
    const cy = box.top + number(paint.top) + height / 2;
    const radius = width / 2;
    const image = host.querySelector('img')!;
    const logo = image.getBoundingClientRect();
    const style = getComputedStyle(image);
    const rounding = number(style.borderTopLeftRadius);
    const offset = Math.hypot(logo.x + logo.width / 2 - cx, logo.y + logo.height / 2 - cy);
    // The rotating partial border fits inside this circle at every angle.
    // A transformed square's bounding box would falsely grow at 45 degrees.
    // The rounded logo's farthest point is the corner-circle center's
    // distance plus its radius (and any center displacement).
    const logoRadius =
      offset + Math.hypot(logo.width / 2 - rounding, logo.height / 2 - rounding) + rounding;
    return {
      rowHeight: row.height,
      wrapper: { width: box.width, height: box.height },
      diameter: width,
      circleHeight: height,
      rounding: paint.borderTopLeftRadius,
      rotation: (() => {
        const matrix = new DOMMatrixReadOnly(paint.transform);
        return ((Math.atan2(matrix.b, matrix.a) * 180) / Math.PI + 360) % 360;
      })(),
      transformOrigin: paint.transformOrigin,
      top: cy - radius - row.top,
      bottom: row.bottom - cy - radius,
      logoGap: radius - border - logoRadius,
      logo: { width: logo.width, height: logo.height, rounding },
      image: {
        padding: style.padding,
        border: style.borderTopWidth,
        background: style.backgroundColor,
        path: new URL(image.src).pathname,
        loaded: image.complete && image.naturalWidth > 0,
      },
      arcColor: paint.borderTopColor,
      expectedColor: (() => {
        const probe = document.createElement('span');
        probe.style.color =
          document.documentElement.dataset.theme === 'dark' ? 'var(--secondary)' : 'var(--success)';
        host.append(probe);
        const color = getComputedStyle(probe).color;
        probe.remove();
        return color;
      })(),
      animation: { name: paint.animationName, duration: paint.animationDuration },
    };
  });
}

for (const host of ['claude', 'codex'] as const)
  for (const scheme of ['dark', 'light'] as const)
    for (const dpr of [1, 2])
      test.describe(`${host} ${scheme} DPR ${dpr}`, () => {
        test.use({
          colorScheme: scheme,
          deviceScaleFactor: dpr,
          viewport: { width: 1120, height: 720 },
        });
        test('running arc clears its 22px Dashboard row at eight angles and returns to idle', async ({
          page,
        }, info) => {
          await serve(page, host);
          await page.goto('/dashboard');
          await expect(page.locator('.xt-lane-live-host')).toHaveCount(1);
          const label = host === 'claude' ? 'Claude Code' : 'Codex';
          const runtime = host === 'claude' ? 'Claude Code runtime' : 'Codex desktop runtime';
          await expect(
            page.getByRole('img', { name: `${label} · Running`, exact: true }),
          ).toHaveAttribute('title', `${runtime} · Running`);
          await expect(
            page.getByRole('status', { name: 'Claude Code · Idle', exact: true }),
          ).toHaveCount(1);
          await expect(page.getByRole('status', { name: 'Codex · Idle', exact: true })).toHaveCount(
            1,
          );
          await page.evaluate(async () => {
            await document.fonts.ready;
            await Promise.all([...document.querySelectorAll('img')].map((image) => image.decode()));
          });
          expect(await page.evaluate(() => window.devicePixelRatio)).toBe(dpr);
          expect(await page.locator('html').getAttribute('data-theme')).toBe(scheme);
          const before = await rows(page);
          expect(before).toHaveLength(3);
          for (const row of before) {
            expect.soft(row.height).toBe(22);
            expect.soft(row.glyphX).toBeCloseTo(before[0].glyphX, 5);
            expect.soft(Math.abs(row.glyphY - before[0].glyphY)).toBeLessThanOrEqual(1);
            expect.soft(row.columns).toEqual(before[0].columns);
          }
          const initial = await arc(page);
          expect(initial.animation).toEqual({ name: 'xt-live-logo-spin', duration: '1.4s' });
          expect(initial.arcColor).toBe(initial.expectedColor);
          const measurements = [];
          for (let angle = 0; angle < 360; angle += 45) {
            // Freeze the production animation, rather than substitute a square
            // outline or replace the production arc with test geometry.
            await page.locator('.xt-lane-live-host').evaluate((host, degrees) => {
              const animation = host.getAnimations({ subtree: true })[0];
              animation.pause();
              animation.currentTime = (degrees / 360) * 1400;
            }, angle);
            const fit = await arc(page);
            measurements.push({ angle, ...fit });
            expect.soft(fit.wrapper, `${angle}° wrapper`).toEqual({ width: 18, height: 18 });
            expect.soft(fit.diameter, `${angle}° outer circle`).toBe(18);
            expect.soft(fit.circleHeight).toBe(fit.diameter);
            expect.soft(fit.rounding).toBe('50%');
            expect.soft(fit.transformOrigin).toBe(`${fit.diameter / 2}px ${fit.diameter / 2}px`);
            expect.soft(fit.rotation, `${angle}° frozen animation`).toBeCloseTo(angle, 3);
            expect.soft(fit.top, `${angle}° top clearance`).toBeGreaterThanOrEqual(2);
            expect.soft(fit.bottom, `${angle}° bottom clearance`).toBeGreaterThanOrEqual(2);
            expect.soft(fit.logoGap, `${angle}° rounded logo gap`).toBeGreaterThanOrEqual(1);
            expect.soft(fit.logo).toEqual({ width: 12, height: 12, rounding: 4 });
            expect.soft(fit.image).toEqual({
              padding: '0px',
              border: '0px',
              background: 'rgba(0, 0, 0, 0)',
              path: host === 'claude' ? '/hosts/claude.svg' : '/hosts/codex.webp',
              loaded: true,
            });
          }
          await info.attach('circular-paint-clearance', {
            body: JSON.stringify(measurements, null, 2),
          });
          await page.getByRole('table', { name: 'Session lanes', exact: true }).screenshot({
            path: info.outputPath(`running-${host}-${scheme}-${dpr}.png`),
          });
          await page.emulateMedia({ reducedMotion: 'reduce' });
          expect((await arc(page)).animation.name).toBe('none');
          const reduced = await arc(page);
          expect.soft(reduced.top).toBeGreaterThanOrEqual(2);
          expect.soft(reduced.bottom).toBeGreaterThanOrEqual(2);
          await page.evaluate(() => {
            document.documentElement.dataset.arcIdle = 'true';
          });
          await expect(page.locator('.xt-lane-live-host')).toHaveCount(0);
          const after = await rows(page);
          expect(after.map(({ top, height, columns }) => ({ top, height, columns }))).toEqual(
            before.map(({ top, height, columns }) => ({ top, height, columns })),
          );
          for (const row of after) {
            expect.soft(Math.abs(row.glyphY - after[0].glyphY)).toBeLessThanOrEqual(1);
          }
        });
      });
