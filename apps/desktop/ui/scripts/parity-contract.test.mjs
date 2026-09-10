import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  currentInputs,
  digest,
  imageRecords,
  validateManifest,
  validateMap,
  writeJson,
} from './parity-contract.mjs';

test('coverage includes 8 Sidebar, 12 TopBar and 20 StatTile comparisons', () => {
  assert.equal(currentInputs().comparisons.length, 40);
});

test('a removed mapping, duplicate theme or wrong size cannot silently reduce coverage', () => {
  const { comparisons } = currentInputs();
  const inventory = comparisons
    .filter((entry) => entry.theme === 'dark')
    .map((entry) => ({ id: entry.story, size: entry.size }));
  assert.throws(() => validateMap(comparisons.slice(1), inventory), /40 entries/);
  const duplicate = structuredClone(comparisons);
  duplicate[1] = duplicate[0];
  assert.throws(() => validateMap(duplicate, inventory), /Duplicate/);
  const resized = structuredClone(comparisons);
  resized[0].size[0]++;
  assert.throws(() => validateMap(resized, inventory), /size/);
});

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'xtrace-parity-test-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const inputs = {
    comparisons: [{ id: 'example-dark', size: [1, 1] }],
    inventory: 'example',
    fonts: { example: 'hash' },
  };
  const environment = { macOS: 'example', webkit: 'pinned' };
  fs.mkdirSync(path.join(directory, 'images'));
  // Only the header is needed to test dimensions and byte integrity; real
  // browser screenshots exercise PNG decoding in the visual comparison suite.
  const png = Buffer.alloc(32);
  Buffer.from('89504e470d0a1a0a', 'hex').copy(png);
  png.writeUInt32BE(2, 16);
  png.writeUInt32BE(2, 20);
  const image = path.join(directory, 'images/example-dark.png');
  fs.writeFileSync(image, png);
  const manifest = {
    version: 1,
    sourceCommit: 'a'.repeat(40),
    inputs,
    environment,
    images: imageRecords(directory, inputs.comparisons),
    review: null,
  };
  manifest.review = { digest: digest(manifest) };
  writeJson(path.join(directory, 'manifest.json'), manifest);
  return { directory, inputs, environment, manifest, image };
}

test('a missing expected image fails without recreating it', (t) => {
  const f = fixture(t);
  fs.unlinkSync(f.image);
  assert.throws(() => validateManifest(f.directory, f.inputs, f.environment), /ENOENT/);
  assert.equal(fs.existsSync(f.image), false);
});

test('changed image bytes and wrong pixel dimensions fail', (t) => {
  const f = fixture(t);
  const bytes = fs.readFileSync(f.image);
  bytes[31] = 1;
  fs.writeFileSync(f.image, bytes);
  assert.throws(() => validateManifest(f.directory, f.inputs, f.environment), /changed baseline/);
  bytes.writeUInt32BE(3, 16);
  fs.writeFileSync(f.image, bytes);
  assert.throws(() => validateManifest(f.directory, f.inputs, f.environment), /pixel width/);
});

test('changed inventory, fonts, runner or mappings invalidate baseline provenance', (t) => {
  const f = fixture(t);
  for (const inputs of [
    { ...f.inputs, inventory: 'changed' },
    { ...f.inputs, fonts: {} },
    { ...f.inputs, comparisons: [] },
  ])
    assert.throws(
      () => validateManifest(f.directory, inputs, f.environment),
      /inventory or fonts changed/,
    );
  assert.throws(
    () => validateManifest(f.directory, f.inputs, { ...f.environment, webkit: 'changed' }),
    /environment differs/,
  );
});

test('candidate generation and comparison do not constitute approval', (t) => {
  const f = fixture(t);
  f.manifest.review = null;
  writeJson(path.join(f.directory, 'manifest.json'), f.manifest);
  assert.throws(() => validateManifest(f.directory, f.inputs, f.environment), /review is missing/);
  assert.equal(validateManifest(f.directory, f.inputs, f.environment, false).review, null);
  f.manifest.review = { digest: digest(f.manifest) };
  f.manifest.sourceCommit = 'b'.repeat(40);
  writeJson(path.join(f.directory, 'manifest.json'), f.manifest);
  assert.throws(
    () => validateManifest(f.directory, f.inputs, f.environment),
    /review is missing or stale/,
  );
});
