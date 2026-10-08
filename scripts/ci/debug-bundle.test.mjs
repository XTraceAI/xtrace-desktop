import assert from 'node:assert/strict';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { test } from 'node:test';
import { smokeEnvironment } from './debug-bundle.mjs';

test('native smoke overrides live paths and clears inherited fixture selection', () => {
  const inherited = {
    XTRACE_DATA_DIR: '/existing-data',
    XTRACE_FIXTURE: 'F1',
    XTRACE_PYTHON: '/some/python',
    PATH: '/tools',
  };
  const env = smokeEnvironment('/owned-test-directory', inherited);
  assert.equal(env.XTRACE_DATA_DIR, join('/owned-test-directory', 'data'));
  assert.equal(env.XTRACE_NATIVE_HOME, join('/owned-test-directory', 'home'));
  assert.equal('XTRACE_FIXTURE' in env, false);
  assert.equal('XTRACE_PYTHON' in env, false);
  assert.equal(env.PATH, inherited.PATH);
  assert.equal(env.GH_TOKEN, '');
  assert.equal(env.GITHUB_TOKEN, '');
  assert.equal(inherited.XTRACE_DATA_DIR, '/existing-data');
  assert.equal(inherited.XTRACE_FIXTURE, 'F1');
});

import { mkdtemp, mkdir, readFile, rm, writeFile, symlink, lstat, rename } from 'node:fs/promises';
import { readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import {
  isSmokeIdentifier,
  inspectorArguments,
  launchSmokeBundle,
  prepareSmokeBundle,
} from './debug-bundle.mjs';

async function bundleFixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'smoke-id-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'source', 'XTrace Desktop.app');
  const directory = join(root, 'copy');
  await mkdir(join(source, 'Contents/MacOS'), { recursive: true });
  await mkdir(directory);
  const plist = join(source, 'Contents/Info.plist');
  const identifier = `ai.xtrace.app.test.${randomUUID()}`;
  await writeFile(plist, identifier);
  await writeFile(join(source, 'Contents/MacOS/xtrace-desktop'), 'original executable');
  const calls = [];
  const run = (command, args) => {
    calls.push([command, ...args]);
    let stdout = '';
    if (args[1]?.startsWith('Set :CFBundleIdentifier '))
      writeFileSync(args[2], args[1].split(' ').at(-1));
    if (args[1] === 'Print :CFBundleIdentifier') stdout = readFileSync(args[2], 'utf8');
    if (args[1] === 'Print :CFBundleExecutable') stdout = 'xtrace-desktop';
    return { status: 0, stdout };
  };
  return { source, directory, plist, calls, run, identifier };
}

test('smoke copies, assigns a unique test ID, signs and verifies without changing the built app', async (t) => {
  const f = await bundleFixture(t);
  const prepared = await prepareSmokeBundle(f.source, f.directory, {
    run: f.run,
    identifier: f.identifier,
  });
  assert.notEqual(prepared.bundle, f.source);
  assert.equal(isSmokeIdentifier(prepared.identifier), true);
  assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  assert.equal(
    await readFile(join(prepared.bundle, 'Contents/Info.plist'), 'utf8'),
    prepared.identifier,
  );
  assert.equal(
    await readFile(join(prepared.bundle, 'Contents/MacOS/xtrace-desktop'), 'utf8'),
    'original executable',
  );
  assert.deepEqual(f.calls.slice(1, 3), [
    ['codesign', '--force', '--deep', '--sign', '-', prepared.bundle],
    ['codesign', '--verify', '--deep', '--strict', prepared.bundle],
  ]);
  const second = join(f.directory, 'second');
  await mkdir(second);
  const next = await prepareSmokeBundle(f.source, second, { run: f.run, identifier: f.identifier });
  assert.equal(prepared.identifier, next.identifier); // Both copies match their compiled source.
  assert.deepEqual(inspectorArguments(42, prepared), ['42', prepared.bundle, prepared.identifier]);
});

test('smoke refuses production IDs and preparation failures before launching any process', async (t) => {
  for (const identifier of [
    'ai.xtrace.app',
    'ai.xtrace.desktop',
    'ai.xtrace.app.test',
    'ai.xtrace.app.test.bad/id',
  ]) {
    assert.equal(isSmokeIdentifier(identifier), false);
    assert.throws(() => inspectorArguments(42, { bundle: '/copy/app', identifier }));
    const f = await bundleFixture(t);
    await assert.rejects(
      launchSmokeBundle(f.source, f.directory, {
        identifier,
        run: f.run,
        spawnProcess: () => assert.fail('Must not spawn'),
      }),
    );
    assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  }
  for (const failAt of [0, 1, 2, 3, 4]) {
    const f = await bundleFixture(t);
    let step = 0;
    await assert.rejects(
      launchSmokeBundle(f.source, f.directory, {
        identifier: f.identifier,
        run: (command, args) => (step++ === failAt ? { status: 1 } : f.run(command, args)),
        spawnProcess: () => assert.fail('Must not spawn'),
      }),
    );
    assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  }
});

test('smoke refuses a source whose identity differs before any copy or spawn', async (t) => {
  const f = await bundleFixture(t);
  await assert.rejects(
    launchSmokeBundle(f.source, f.directory, {
      identifier: f.identifier,
      run: (command, args) =>
        args[1] === 'Print :CFBundleIdentifier'
          ? { status: 0, stdout: 'ai.xtrace.app' }
          : f.run(command, args),
      spawnProcess: () => assert.fail('Must not spawn'),
    }),
    /expected test identity/,
  );
});

test('relative plist and executable links become independent files in the smoke copy', async (t) => {
  const f = await bundleFixture(t);
  const realPlist = join(f.source, 'Contents/real.plist');
  const executable = join(f.source, 'Contents/MacOS/xtrace-desktop');
  const realExecutable = join(f.source, 'Contents/MacOS/real-executable');
  await rename(f.plist, realPlist);
  await symlink('real.plist', f.plist);
  await rename(executable, realExecutable);
  await symlink('real-executable', executable);
  const prepared = await prepareSmokeBundle(f.source, f.directory, {
    run: f.run,
    identifier: f.identifier,
  });
  assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  assert.equal(await readFile(realPlist, 'utf8'), f.identifier);
  assert.equal((await lstat(f.plist)).isSymbolicLink(), true);
  assert.equal((await lstat(executable)).isSymbolicLink(), true);
  assert.equal((await lstat(join(prepared.bundle, 'Contents/Info.plist'))).isSymbolicLink(), false);
  assert.equal(
    (await lstat(join(prepared.bundle, 'Contents/MacOS/xtrace-desktop'))).isSymbolicLink(),
    false,
  );
  assert.equal(await readFile(realExecutable, 'utf8'), 'original executable');
});

test('compiled native inspector accepts test IDs and immediately rejects production or malformed IDs', async (t) => {
  if (process.platform !== 'darwin') return t.skip('Native inspector requires macOS');
  const root = await mkdtemp(join(tmpdir(), 'smoke-inspector-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const exe = join(root, 'inspect');
  const compile = spawnSync('swiftc', ['scripts/ci/debug-bundle.swift', '-o', exe], {
    encoding: 'utf8',
  });
  assert.equal(compile.status, 0, compile.stderr);
  for (const identifier of ['ai.xtrace.app', 'ai.xtrace.desktop', 'ai.xtrace.app.test', 'bad']) {
    assert.equal(spawnSync(exe, ['2147483647', '/no/test/app', identifier]).status, 2);
  }
  // An accepted ID enters the inspection loop rather than returning its
  // validation error. No process with this impossible PID is ever launched.
  const accepted = spawnSync(exe, ['2147483647', '/no/test/app', 'ai.xtrace.app.test.abc123'], {
    timeout: 250,
  });
  assert.equal(accepted.status, null);
  assert.equal(accepted.signal, 'SIGTERM');
});

test('a copied identity changed during signing is refused before spawn', async (t) => {
  const f = await bundleFixture(t);
  await assert.rejects(
    launchSmokeBundle(f.source, f.directory, {
      identifier: f.identifier,
      run: (command, args) =>
        args[1] === 'Print :CFBundleIdentifier' && args[2] !== f.plist
          ? { status: 0, stdout: 'ai.xtrace.app' }
          : f.run(command, args),
      spawnProcess: () => assert.fail('Must not spawn'),
    }),
    /identity mismatch/,
  );
});
