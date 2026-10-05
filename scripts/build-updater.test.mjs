import { test } from 'node:test';
import assert from 'node:assert/strict';
import { prepareUpdaterRelease } from './build-updater.mjs';
import { fileURLToPath } from 'node:url';

const template = fileURLToPath(
  new URL('../apps/desktop/src-tauri/tauri.updater.example.json', import.meta.url),
);
test('updater release requires an explicit overlay', () => {
  assert.throws(() => prepareUpdaterRelease([], {}), /overlay path/);
});
test('updater release refuses missing signing configuration before build', () => {
  assert.throws(() => prepareUpdaterRelease([template], {}), /TAURI_SIGNING_PRIVATE_KEY/);
});
