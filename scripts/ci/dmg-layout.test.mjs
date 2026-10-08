import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  expectedLayout,
  inspectMountedLayout,
  layoutProblems,
  readIconPositions,
} from './dmg-layout.mjs';
import { blob, dsStore, iloc, record, u32 } from './ds-store-fixture.mjs';

const expected = {
  app: 'XTrace Desktop.app',
  background: 'background.tiff',
  icons: { 'XTrace Desktop.app': { x: 180, y: 170 }, Applications: { x: 480, y: 170 } },
};
const finderRecords = [
  record('.', 'bwsp', 'blob', blob(Buffer.from('window settings'))),
  record('.', 'vSrn', 'long', u32(1)),
  iloc('Applications', 480, 170),
  record('XTrace Desktop.app', 'ICVO', 'bool', Buffer.from([1])),
  iloc('XTrace Desktop.app', 180, 170),
];

test('the expected layout comes from the disk image settings in tauri.conf.json', async () => {
  const layout = await expectedLayout();
  assert.deepEqual(layout, expected);
});

test('reads saved icon positions from one leaf or a split record tree', () => {
  for (const split of [false, true]) {
    const positions = readIconPositions(dsStore(finderRecords, { split }));
    assert.deepEqual(Object.fromEntries(positions), expected.icons, `split: ${split}`);
    assert.deepEqual(layoutProblems(positions, expected), []);
  }
});

test('reports the misplaced app icon seen in a local build and a missing icon', () => {
  const moved = readIconPositions(
    dsStore([iloc('Applications', 480, 170), iloc('XTrace Desktop.app', 208, 151)]),
  );
  assert.deepEqual(layoutProblems(moved, expected), [
    'XTrace Desktop.app is at 208,151; expected 180,170.',
  ]);
  const missing = readIconPositions(dsStore([iloc('XTrace Desktop.app', 180, 170)]));
  assert.deepEqual(layoutProblems(missing, expected), ['Applications has no saved icon position.']);
});

test('rejects files that are not a readable .DS_Store', () => {
  assert.throws(() => readIconPositions(Buffer.alloc(64)), /Not a \.DS_Store file/);
  const truncated = dsStore(finderRecords).subarray(0, 4096 * 3);
  assert.throws(() => readIconPositions(truncated), /Truncated|Missing/);
});

test('a mounted image needs the app, the Applications link, the background and saved positions', async () => {
  const volume = await mkdtemp(join(tmpdir(), 'xtrace-dmg-layout-test-'));
  try {
    assert.deepEqual((await inspectMountedLayout(volume, expected)).problems, [
      'XTrace Desktop.app is missing.',
      'The Applications shortcut is missing.',
      'The window background picture is missing.',
      'Finder saved no window layout (.DS_Store is missing).',
    ]);
    await mkdir(join(volume, 'XTrace Desktop.app'));
    await symlink('/Applications', join(volume, 'Applications'));
    await mkdir(join(volume, '.background'));
    await writeFile(join(volume, '.background/background.tiff'), '');
    await writeFile(join(volume, '.DS_Store'), dsStore(finderRecords, { split: true }));
    assert.deepEqual((await inspectMountedLayout(volume, expected)).problems, []);
    await rm(join(volume, 'Applications'));
    await symlink('/tmp', join(volume, 'Applications'));
    assert.deepEqual((await inspectMountedLayout(volume, expected)).problems, [
      'The Applications shortcut does not point to /Applications.',
    ]);
  } finally {
    await rm(volume, { recursive: true, force: true });
  }
});
