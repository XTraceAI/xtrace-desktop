import assert from 'node:assert/strict';
import { access } from 'node:fs/promises';
import test from 'node:test';
import { checkNative } from './native-check.mjs';

const base = 'a'.repeat(40);
const head = 'b'.repeat(40);
function fixture({ fail, dirty = false, changed = false, notAncestor = false } = {}) {
  const calls = [];
  let identities = 0;
  let trusted;
  const run = (command, args, options) => {
    calls.push([command, ...args].join(' '));
    assert.equal(options.env.GH_TOKEN, '');
    assert.equal(options.env.GITHUB_TOKEN, '');
    if (options.env.CI_TRUSTED_ROOT) trusted = options.env.CI_TRUSTED_ROOT;
    let stdout = '';
    if (command === 'git' && args[0] === 'status') stdout = dirty ? ' M source.rs' : '';
    if (command === 'git' && args[0] === 'rev-parse')
      stdout = changed && identities++ > 0 ? base : head;
    if (command === 'sw_vers') stdout = '26.5.2';
    const failed = calls.at(-1) === fail || (notAncestor && args[0] === 'merge-base');
    return { status: failed ? 1 : 0, stdout };
  };
  return { calls, run, trusted: () => trusted, install: async () => [] };
}

test('native validation stops before work on an unsupported platform, dirty checkout or stale base', async () => {
  for (const options of [{ platform: 'linux' }, { base: 'main' }]) {
    await assert.rejects(
      checkNative({ base, platform: 'darwin', run: () => assert.fail(), ...options }),
    );
  }
  for (const options of [{ dirty: true }, { notAncestor: true }]) {
    const f = fixture(options);
    await assert.rejects(checkNative({ base, platform: 'darwin', ...f }));
    assert.ok(!f.calls.some((x) => x.startsWith('pnpm')));
  }
});

test('native failures stop packaging and clean the temporary hook baseline', async () => {
  for (const fail of [
    'cargo test --workspace --all-features --locked',
    'node scripts/ci/run-hook.mjs dto',
    'pnpm notices:check',
  ]) {
    const f = fixture({ fail });
    await assert.rejects(checkNative({ base, platform: 'darwin', ...f }), /stopped/);
    assert.ok(!f.calls.some((x) => x.includes('tauri build')));
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('native evidence binds source, base and actual OS; release mode also launches the production build', async () => {
  for (const release of [false, true]) {
    const f = fixture();
    assert.deepEqual(await checkNative({ base, release, platform: 'darwin', ...f }), {
      head,
      base,
      macOS: '26.5.2',
      release,
    });
    assert.ok(f.calls.includes('node scripts/ci/run-hook.mjs plugin-conformance'));
    assert.ok(f.calls.includes('node scripts/ci/run-hook.mjs dto'));
    assert.ok(
      f.calls.includes('node scripts/security/scan-secrets.mjs --content artifacts/sbom.cdx.json'),
    );
    assert.ok(f.calls.some((x) => x.includes('target/debug/bundle/macos/')));
    assert.equal(
      f.calls.some((x) => x.includes('target/release/bundle/macos/')),
      release,
    );
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('a source advance cannot produce successful native evidence', async () => {
  const f = fixture({ changed: true });
  await assert.rejects(checkNative({ base, platform: 'darwin', ...f }), /Source changed/);
  await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
});
