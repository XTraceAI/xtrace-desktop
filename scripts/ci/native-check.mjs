import { spawnSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { installAbout, installSyft } from '../supply-chain/install-tools.mjs';
import { readPin } from './plugin-pin.mjs';

export async function checkNative({
  base,
  release = false,
  repo = process.cwd(),
  platform = process.platform,
  run = spawnSync,
  environment = process.env,
  install = async () => [await installSyft(), await installAbout()],
} = {}) {
  if (platform !== 'darwin' || !/^[a-f0-9]{40}$/.test(base ?? ''))
    throw new Error('Native checks require macOS and --base with a full reviewed commit SHA.');
  const env = { ...environment, GH_TOKEN: '', GITHUB_TOKEN: '' };
  const execute = (command, args, options = {}) => {
    const result = run(command, args, { cwd: repo, env, stdio: 'inherit', ...options });
    if (result.error || result.signal || result.status !== 0)
      throw new Error(`Native validation stopped: ${command} failed.`);
    return result.stdout;
  };
  const git = (...args) => execute('git', args, { stdio: 'pipe', encoding: 'utf8' }).trim();
  const requireClean = () => {
    if (git('status', '--porcelain'))
      throw new Error('Native validation requires a clean checkout, including untracked files.');
  };
  requireClean();
  const head = git('rev-parse', 'HEAD');
  git('cat-file', '-e', `${base}^{commit}`);
  // Validate the combined result; a stale feature branch is not merge evidence.
  git('merge-base', '--is-ancestor', base, head);
  const version = execute('sw_vers', ['-productVersion'], {
    stdio: 'pipe',
    encoding: 'utf8',
  }).trim();
  const trusted = await mkdtemp(join(tmpdir(), 'xtrace-native-base-'));
  try {
    const archive = execute('git', ['archive', base], {
      stdio: 'pipe',
      maxBuffer: 64 * 1024 * 1024,
    });
    execute('tar', ['-xf', '-', '-C', trusted], { input: archive, stdio: 'pipe' });
    env.CI_TRUSTED_ROOT = trusted;
    execute('pnpm', ['install', '--frozen-lockfile', '--ignore-scripts']);
    execute('cargo', ['fmt', '--all', '--', '--check']);
    execute('cargo', [
      'clippy',
      '--workspace',
      '--all-targets',
      '--all-features',
      '--locked',
      '--',
      '-D',
      'warnings',
    ]);
    if (env.AGENT_PLUGINS_SOURCE === 'bundle') {
      // The first workspace test already runs the Python conformance harnesses;
      // they need the same verifier and clean interpreter environment as the hook.
      const pin = await readPin(repo);
      env.AGENT_PLUGINS_DIR = resolve(
        repo,
        env.AGENT_PLUGINS_DIR || join('vendor/agent-plugins', pin.plugin_root),
      );
      env.XTRACE_CONFORMANCE_BUNDLE_VERIFIER = resolve(
        repo,
        env.CARGO_TARGET_DIR || 'target',
        'debug/examples/conformance_bundle',
      );
      env.PYTHONDONTWRITEBYTECODE = '1';
      for (const key of ['PYTHONOPTIMIZE', 'PYTHONPATH', 'PYTHONHOME', 'PYTHONSTARTUP'])
        delete env[key];
      execute('cargo', ['build', '-p', 'xt-ingest', '--example', 'conformance_bundle', '--locked']);
      execute(env.PYTHON || 'python3', [
        '-B',
        '-c',
        [
          'import sys; from pathlib import Path',
          'if sys.version_info < (3, 10) or sys.flags.optimize: raise RuntimeError("Conformance requires Python 3.10+ with assertions active")',
          'sys.path.insert(0, "scripts/conformance")',
          'from bundle_source import verify_bundle',
          'print("pinned producer bundle " + verify_bundle(Path(sys.argv[1])))',
        ].join('\n'),
        env.AGENT_PLUGINS_DIR,
      ]);
    }
    execute('cargo', ['test', '--workspace', '--all-features', '--locked']);
    for (const hook of ['plugin-conformance', 'dto'])
      execute('node', ['scripts/ci/run-hook.mjs', hook]);
    env.PATH = [...(await install()), env.PATH ?? ''].join(delimiter);
    execute('pnpm', ['sbom']);
    execute('pnpm', ['notices:check']);
    execute('node', ['scripts/security/scan-secrets.mjs', '--content', 'artifacts/sbom.cdx.json']);
    // A test identity must be compiled into Tauri, including its singleton
    // socket. A plist-only change cannot isolate it from the installed app.
    const testIdentifier = `ai.xtrace.app.test.${randomUUID()}`;
    const testTarget = join(trusted, 'smoke-target');
    // APFS clones reuse our completed build cache without writable links back
    // to it. All test build outputs are disposable and separate from source.
    execute('cp', ['-c', '-R', join(repo, 'target'), testTarget]);
    env.CARGO_TARGET_DIR = testTarget;
    const testConfig = JSON.stringify({ identifier: testIdentifier });
    execute('pnpm', ['tauri', 'build', '--debug', '--bundles', 'app', '--config', testConfig]);
    execute('node', [
      'scripts/ci/debug-bundle.mjs',
      join(testTarget, 'debug/bundle/macos/XTrace Desktop.app'),
      testIdentifier,
    ]);
    if (release) {
      execute('pnpm', ['tauri', 'build', '--bundles', 'app', '--config', testConfig]);
      execute('node', [
        'scripts/ci/debug-bundle.mjs',
        join(testTarget, 'release/bundle/macos/XTrace Desktop.app'),
        testIdentifier,
      ]);
    }
    if (git('rev-parse', 'HEAD') !== head)
      throw new Error('Source changed during native validation.');
    requireClean();
    return { head, base, macOS: version, release };
  } finally {
    await rm(trusted, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const base = args[0] === '--base' ? args[1] : undefined;
  if (!base || args.length > 3 || (args[2] && args[2] !== '--release')) {
    console.error('Usage: pnpm check:native --base FULL_COMMIT_SHA [--release]');
    process.exitCode = 1;
  } else {
    checkNative({ base, release: args[2] === '--release' })
      .then((result) => console.log('Native validation passed: ' + JSON.stringify(result)))
      .catch((error) => {
        console.error(error.message);
        process.exitCode = 1;
      });
  }
}
