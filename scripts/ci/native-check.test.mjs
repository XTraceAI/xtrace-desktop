import assert from 'node:assert/strict';
import { access } from 'node:fs/promises';
import { resolve } from 'node:path';
import test from 'node:test';
import { checkNative } from './native-check.mjs';

const base = 'a'.repeat(40);
const head = 'b'.repeat(40);
function fixture({
  fail,
  dirty = false,
  changed = false,
  notAncestor = false,
  environment = {},
} = {}) {
  const calls = [];
  const invocations = [];
  const buildTargets = [];
  let identities = 0;
  let trusted;
  const run = (command, args, options) => {
    calls.push([command, ...args].join(' '));
    invocations.push({ command, args: [...args], env: { ...options.env } });
    if (command === 'pnpm' && args[0] === 'tauri') buildTargets.push(options.env.CARGO_TARGET_DIR);
    assert.equal(options.env.GH_TOKEN, '');
    assert.equal(options.env.GITHUB_TOKEN, '');
    if (options.env.CI_TRUSTED_ROOT) trusted = options.env.CI_TRUSTED_ROOT;
    let stdout = '';
    if (command === 'git' && args[0] === 'status') stdout = dirty ? ' M source.rs' : '';
    if (command === 'git' && args[0] === 'rev-parse')
      stdout = changed && identities++ > 0 ? base : head;
    if (command === 'sw_vers') stdout = '26.5.2';
    const failed =
      (typeof fail === 'function' ? fail(command, args) : calls.at(-1) === fail) ||
      (notAncestor && args[0] === 'merge-base');
    return { status: failed ? 1 : 0, stdout };
  };
  return {
    calls,
    invocations,
    buildTargets,
    environment,
    run,
    trusted: () => trusted,
    install: async () => [],
  };
}

test('bundle setup builds, verifies and exports before the first workspace test', async () => {
  for (const overrides of [
    {},
    {
      CARGO_TARGET_DIR: 'artifacts/private/test-target',
      AGENT_PLUGINS_DIR: 'custom/plugins/memhub',
    },
    { CARGO_TARGET_DIR: '/tmp/xtrace-test-target', PYTHON: '/tmp/xtrace-test-python' },
  ]) {
    const f = fixture({
      environment: {
        AGENT_PLUGINS_SOURCE: 'bundle',
        XTRACE_CONFORMANCE_BUNDLE_VERIFIER: '/tmp/stale-verifier',
        PYTHONOPTIMIZE: '1',
        PYTHONPATH: '/tmp/shadow-modules',
        PYTHONHOME: '/tmp/other-runtime',
        PYTHONSTARTUP: '/tmp/startup.py',
        ...overrides,
      },
    });
    await checkNative({ base, platform: 'darwin', ...f });
    const build = f.calls.indexOf('cargo build -p xt-ingest --example conformance_bundle --locked');
    const verify = f.invocations.findIndex((call) => call.args[0] === '-B');
    const testRun = f.calls.indexOf('cargo test --workspace --all-features --locked');
    const hook = f.calls.indexOf('node scripts/ci/run-hook.mjs plugin-conformance');
    assert.ok(build >= 0 && build < verify && verify < testRun && testRun < hook);
    const expectedRoot = resolve(
      overrides.AGENT_PLUGINS_DIR || 'vendor/agent-plugins/plugins/memhub',
    );
    const expectedVerifier = resolve(
      overrides.CARGO_TARGET_DIR || 'target',
      'debug/examples/conformance_bundle',
    );
    const probe = f.invocations[verify];
    assert.equal(probe.command, overrides.PYTHON || 'python3');
    assert.match(probe.args[2], /from bundle_source import verify_bundle/);
    assert.match(probe.args[2], /sys\.flags\.optimize/);
    assert.equal(probe.args[3], expectedRoot);
    for (const call of [probe, f.invocations[testRun], f.invocations[hook]]) {
      assert.equal(call.env.AGENT_PLUGINS_SOURCE, 'bundle');
      assert.equal(call.env.AGENT_PLUGINS_DIR, expectedRoot);
      assert.equal(call.env.XTRACE_CONFORMANCE_BUNDLE_VERIFIER, expectedVerifier);
      assert.equal(call.env.PYTHONDONTWRITEBYTECODE, '1');
      for (const key of ['PYTHONOPTIMIZE', 'PYTHONPATH', 'PYTHONHOME', 'PYTHONSTARTUP'])
        assert.ok(!(key in call.env));
    }
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('bundle build or verification failure aborts before workspace tests, hooks and packaging', async () => {
  for (const fail of [
    'cargo build -p xt-ingest --example conformance_bundle --locked',
    (command, args) => command === 'python3' && args[0] === '-B',
  ]) {
    const f = fixture({ fail, environment: { AGENT_PLUGINS_SOURCE: 'bundle' } });
    await assert.rejects(checkNative({ base, platform: 'darwin', ...f }), /stopped/);
    assert.ok(!f.calls.includes('cargo test --workspace --all-features --locked'));
    assert.ok(
      !f.calls.some((call) => call.includes('run-hook.mjs') || call.includes('tauri build')),
    );
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('default and explicit checkout flows do not add bundle setup or alter interpreter settings', async () => {
  for (const mode of [undefined, 'checkout']) {
    const environment = { PYTHONOPTIMIZE: '1' };
    if (mode) environment.AGENT_PLUGINS_SOURCE = mode;
    const f = fixture({ environment });
    await checkNative({ base, platform: 'darwin', ...f });
    assert.ok(!f.calls.some((call) => call.includes('conformance_bundle')));
    const workspace = f.invocations.find(
      (call) => call.command === 'cargo' && call.args[0] === 'test',
    );
    assert.equal(workspace.env.PYTHONOPTIMIZE, '1');
    assert.ok(!('XTRACE_CONFORMANCE_BUNDLE_VERIFIER' in workspace.env));
    assert.ok(!('AGENT_PLUGINS_DIR' in workspace.env));
  }
});

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

test('native evidence binds source, base and actual OS; release mode also launches an isolated release build', async () => {
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
    const builds = f.calls.filter((call) => call.startsWith('pnpm tauri build'));
    assert.ok(builds.every((call) => call.includes('--config ')));
    const ids = builds.map((call) => JSON.parse(call.slice(call.indexOf('{'))).identifier);
    assert.ok(ids.every((id) => /^ai\.xtrace\.app\.test\.[a-f0-9-]+$/.test(id)));
    for (const launch of f.calls.filter((call) =>
      call.startsWith('node scripts/ci/debug-bundle.mjs'),
    )) {
      assert.ok(launch.endsWith(ids[0]));
      assert.ok(launch.includes(f.trusted()));
    }
    assert.ok(f.buildTargets.every((target) => target === f.trusted() + '/smoke-target'));
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('a source advance cannot produce successful native evidence', async () => {
  const f = fixture({ changed: true });
  await assert.rejects(checkNative({ base, platform: 'darwin', ...f }), /Source changed/);
  await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
});
