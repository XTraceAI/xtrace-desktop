import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { rustNotices } from './rust-notices.mjs';
import { renderNotices } from './licenses.mjs';

test('Rust generic license text never replaces original package copyrights', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'synthetic-license-'));
  try {
    const pkg = {
      name: 'synthetic',
      version: '1.0.0',
      source: 'synthetic-registry',
      manifest_path: join(directory, 'Cargo.toml'),
    };
    const rust = {
      licenses: [{ id: 'MIT', text: 'Generic license text.', used_by: [{ crate: pkg }] }],
      crates: [{ package: pkg, license: 'MIT' }],
    };
    await assert.rejects(rustNotices(rust), /original upstream notice/);
    await writeFile(
      join(directory, 'LICENSE'),
      'Copyright Synthetic Contributor.\nPermission text.',
    );
    const output = renderNotices(await rustNotices(rust));
    assert.match(output, /Copyright Synthetic Contributor/);
    assert.match(output, /Generic license text/);
    await writeFile(join(directory, 'NOTICE'), 'Additional attribution.');
    assert.notEqual(renderNotices(await rustNotices(rust)), output);
    pkg.license_file = '../unowned.txt';
    await assert.rejects(rustNotices(rust));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
