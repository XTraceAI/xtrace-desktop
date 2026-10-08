import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Draws the background of the disk image window that asks people to drag the
// app into Applications. Sizes and icon centers must match bundle.macOS.dmg in
// apps/desktop/src-tauri/tauri.conf.json: icons at y=170, and a window 32pt taller
// than this picture because its size includes the Finder title bar.
const root = fileURLToPath(new URL('../', import.meta.url));
const { chromium } = createRequire(join(root, 'apps/desktop/ui/package.json'))('@playwright/test');
// setContent pages cannot load file:// fonts, so embed the app's own Manrope.
const font = async (weight) =>
  (await readFile(join(root, `apps/desktop/ui/public/fonts/manrope-${weight}.ttf`))).toString(
    'base64',
  );
const [medium, semibold] = await Promise.all([font(500), font(600)]);
const output = join(root, 'apps/desktop/src-tauri/dmg/background.tiff');

const html = `<!doctype html><html><head><style>
@font-face { font-family: Manrope; font-weight: 600; src: url(data:font/ttf;base64,${semibold}); }
@font-face { font-family: Manrope; font-weight: 500; src: url(data:font/ttf;base64,${medium}); }
html, body { margin: 0; width: 660px; height: 400px; overflow: hidden; }
body {
  position: relative; font-family: Manrope, sans-serif; color: #1a1a1a;
  background:
    radial-gradient(420px 260px at 18% 0%, rgba(123, 144, 252, 0.20), transparent 70%),
    radial-gradient(420px 260px at 92% 100%, rgba(168, 82, 255, 0.16), transparent 70%),
    #f7f7fa;
}
svg { position: absolute; left: 0; top: 0; }
p { position: absolute; left: 0; right: 0; margin: 0; text-align: center; }
.title { top: 316px; font-size: 15px; font-weight: 600; letter-spacing: -0.01em; }
.hint { top: 340px; font-size: 12px; font-weight: 500; color: #6b6b80; }
</style></head><body>
<svg width="660" height="400" viewBox="0 0 660 400">
  <defs>
    <linearGradient id="brand" gradientUnits="userSpaceOnUse" x1="268" x2="394">
      <stop offset="0" stop-color="#7b90fc"/><stop offset="1" stop-color="#a852ff"/>
    </linearGradient>
  </defs>
  <path d="M268 170 H380" stroke="url(#brand)" stroke-width="5" stroke-linecap="round"
        stroke-dasharray="1 13" fill="none"/>
  <path d="M376 156 L394 170 L376 184" stroke="#a852ff" stroke-width="5"
        stroke-linecap="round" stroke-linejoin="round" fill="none"/>
</svg>
<p class="title">Drag XTrace Desktop into Applications</p>
<p class="hint">Then open it from your Applications folder.</p>
</body></html>`;

const temporary = await mkdtemp(join(tmpdir(), 'xtrace-dmg-'));
const browser = await chromium.launch();
try {
  const images = [];
  for (const scale of [1, 2]) {
    const page = await browser.newPage({
      viewport: { width: 660, height: 400 },
      deviceScaleFactor: scale,
    });
    await page.setContent(html, { waitUntil: 'load' });
    await page.evaluate(() => document.fonts.ready);
    const path = join(temporary, scale === 1 ? 'background.png' : 'background@2x.png');
    await page.screenshot({ path });
    images.push(path);
    await page.close();
  }
  // One TIFF with both sizes lets Finder draw the sharp copy on Retina screens.
  const result = spawnSync('tiffutil', ['-cathidpicheck', ...images, '-out', output], {
    stdio: 'inherit',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error('tiffutil could not combine the backgrounds.');
} finally {
  await browser.close();
  await rm(temporary, { recursive: true, force: true });
}
