import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const uiRoot = fileURLToPath(new URL('../', import.meta.url));
export const repoRoot = path.resolve(uiRoot, '../../..');
export const approvedRoot = path.join(uiRoot, 'e2e/parity/baselines');
export const captureOptions = {
  viewport: { width: 2880, height: 1120 },
  deviceScaleFactor: 2,
  locale: 'en-US',
  timezoneId: 'UTC',
  colorScheme: 'light',
  reducedMotion: 'reduce',
};
export const diffOptions = { maxDiffPixelRatio: 0.002, threshold: 0.1 };
export const sha256 = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
export const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));
export const writeJson = (file, value) =>
  fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n');

export function validateMap(entries, inventory) {
  assert.equal(entries.length, 40, 'The initial comparison contract requires 40 entries');
  assert.equal(new Set(entries.map((entry) => entry.id)).size, 40, 'Duplicate comparison IDs');
  const families = { sidebar: 0, topbar: 0, stattile: 0 };
  for (const entry of entries) {
    assert.match(entry.id, /^[a-z0-9-]+$/);
    assert.ok(['dark', 'light'].includes(entry.theme), 'Invalid comparison theme');
    assert.equal(entry.id, `${entry.story.replace('/', '-')}-${entry.theme}`);
    const story = inventory.find((story) => story.id === entry.story);
    assert.ok(story, 'Comparison story is missing from the gallery');
    assert.deepEqual(entry.size, story.size, 'Comparison size does not match the gallery');
    const family = entry.story.split('/')[0];
    assert.ok(Object.hasOwn(families, family), 'Unexpected comparison family');
    families[family]++;
    assert.equal(
      entries.filter((other) => other.story === entry.story).length,
      2,
      'Each story needs both themes',
    );
  }
  assert.deepEqual(families, { sidebar: 8, topbar: 12, stattile: 20 });
  return entries;
}

export function currentInputs() {
  const inventory = fs.readFileSync(path.join(uiRoot, 'src/gallery/inventory.json'));
  const comparisons = validateMap(
    readJson(path.join(uiRoot, 'e2e/parity/map.json')),
    JSON.parse(inventory),
  );
  const fontsRoot = path.join(uiRoot, 'public/fonts');
  const fonts = Object.fromEntries(
    fs
      .readdirSync(fontsRoot)
      .filter((name) => /\.(woff2|ttf)$/.test(name))
      .sort()
      .map((name) => [name, sha256(fs.readFileSync(path.join(fontsRoot, name)))]),
  );
  return { comparisons, inventory: sha256(inventory), fonts };
}

export function imageRecords(directory, comparisons) {
  return Object.fromEntries(
    comparisons.map((entry) => {
      const name = `${entry.id}.png`;
      const bytes = fs.readFileSync(path.join(directory, 'images', name));
      assert.equal(
        bytes.subarray(0, 8).toString('hex'),
        '89504e470d0a1a0a',
        'Expected PNG baseline',
      );
      assert.equal(bytes.readUInt32BE(16), entry.size[0] * 2, 'Wrong baseline pixel width');
      assert.equal(bytes.readUInt32BE(20), entry.size[1] * 2, 'Wrong baseline pixel height');
      return [name, `sha256:${sha256(bytes)}`];
    }),
  );
}

export function digest(manifest) {
  const content = { ...manifest };
  delete content.review;
  return sha256(JSON.stringify(content));
}

export function validateManifest(directory, inputs, environment, requireApproval = true) {
  const manifest = readJson(path.join(directory, 'manifest.json'));
  assert.equal(manifest.version, 1, 'Unsupported baseline manifest');
  assert.match(manifest.sourceCommit, /^[a-f0-9]{40}$/);
  assert.deepEqual(
    manifest.inputs,
    inputs,
    'Baseline mapping, gallery inventory or fonts changed; review new candidates',
  );
  assert.deepEqual(
    manifest.environment,
    environment,
    'Baseline macOS/WebKit environment differs; use the recorded runner',
  );
  assert.deepEqual(
    manifest.images,
    imageRecords(directory, inputs.comparisons),
    'Missing or changed baseline PNG',
  );
  if (requireApproval)
    assert.equal(manifest.review?.digest, digest(manifest), 'Baseline review is missing or stale');
  return manifest;
}

export function reviewPage(directory, manifest) {
  const rows = manifest.inputs.comparisons
    .filter((entry) => entry.theme === 'dark')
    .map((entry) => {
      const id = entry.id.slice(0, -5);
      return `<section><h2>${entry.story}</h2><div class="pair">${['dark', 'light']
        .map(
          (theme) =>
            `<figure><figcaption>${theme}${fs.existsSync(path.join(directory, 'previous/images', `${id}-${theme}.png`)) ? ` · <a href="previous/images/${id}-${theme}.png">Previous</a>` : ''}</figcaption><a href="images/${id}-${theme}.png"><img width="${entry.size[0]}" height="${entry.size[1]}" src="images/${id}-${theme}.png" alt="${entry.story} ${theme}"></a></figure>`,
        )
        .join('')}</div></section>`;
    })
    .join('');
  fs.writeFileSync(
    path.join(directory, 'index.html'),
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>XTrace visual baseline review</title><style>body{margin:0;padding:24px;font:15px system-ui;background:#ededf1;color:#222}header{max-width:900px}code{overflow-wrap:anywhere}section{margin:32px 0;padding:20px;background:white;border-radius:12px}h2{font-size:16px}.pair{display:flex;gap:24px;overflow:auto}figure{margin:0}figcaption{margin-bottom:10px}img{display:block;max-width:none}a{color:inherit}</style><header><h1>Visual baseline candidates</h1><p>40 screenshots of authored sample states. These are proposed regression references, not proof that finished product screens match an independent design.</p><p>Review both themes for each component. Images are shown at CSS size; click to inspect their full 2× resolution.</p><p>Source: <code>${manifest.sourceCommit}</code></p><p>Review identity: <code>${digest(manifest)}</code></p></header>${rows}</html>`,
  );
}
