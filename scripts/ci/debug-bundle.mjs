import { spawn, spawnSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function smokeEnvironment(directory, inherited = process.env) {
  const env = { ...inherited, GITHUB_TOKEN: '', GH_TOKEN: '', XTRACE_TELEMETRY: '0' };
  delete env.XTRACE_FIXTURE;
  delete env.XTRACE_PYTHON;
  env.XTRACE_DATA_DIR = join(directory, 'data');
  // The smoke launch indexes an empty synthetic home, never the machine's own history.
  env.XTRACE_NATIVE_HOME = join(directory, 'home');
  return env;
}

/** Test identities are per-run and never use the installed app's identity. */
export function isSmokeIdentifier(value) {
  return /^ai\.xtrace\.app\.test\.[a-f0-9-]+$/.test(value);
}

export function inspectorArguments(pid, prepared) {
  if (!isSmokeIdentifier(prepared.identifier)) throw new Error('Invalid smoke identity');
  return [String(pid), prepared.bundle, prepared.identifier];
}

/** Copy first; all plist and signing writes belong to that disposable copy. */
export async function prepareSmokeBundle(source, directory, { identifier, run = spawnSync } = {}) {
  if (!isSmokeIdentifier(identifier)) throw new Error('Invalid smoke identity');
  const checked = (command, args) => {
    const result = run(command, args, { encoding: 'utf8', stdio: 'pipe', timeout: 120_000 });
    if (result.error || result.signal || result.status !== 0)
      throw new Error('Smoke preparation failed');
    return result.stdout?.trim();
  };
  // A plist copy cannot change Tauri's compiled single-instance identity.
  // The caller must build with this exact test ID before asking for a launch.
  if (
    checked('/usr/libexec/PlistBuddy', [
      '-c',
      'Print :CFBundleIdentifier',
      join(source, 'Contents/Info.plist'),
    ]) !== identifier
  )
    throw new Error('Source was not built with the expected test identity');
  const bundle = join(directory, basename(source));
  if (resolve(bundle) === resolve(source)) throw new Error('Smoke copy must be separate');
  // Dereference trusted build links so executable and signing writes cannot
  // follow a copied link back into the original build.
  await cp(source, bundle, {
    recursive: true,
    dereference: true,
    errorOnExist: true,
    force: false,
  });
  const plist = join(bundle, 'Contents/Info.plist');
  checked('codesign', ['--force', '--deep', '--sign', '-', bundle]);
  checked('codesign', ['--verify', '--deep', '--strict', bundle]);
  // Read after signing; never launch a copy whose identity was not established.
  if (checked('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleIdentifier', plist]) !== identifier)
    throw new Error('Smoke identity mismatch');
  const executable = checked('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleExecutable', plist]);
  if (!executable || !/^[A-Za-z0-9_. -]+$/.test(executable))
    throw new Error('Invalid smoke executable');
  return { bundle, identifier, executable };
}

export async function launchSmokeBundle(
  source,
  directory,
  { spawnProcess = spawn, ...options } = {},
) {
  const prepared = await prepareSmokeBundle(source, directory, options);
  const child = spawnProcess(join(prepared.bundle, 'Contents/MacOS', prepared.executable), [], {
    stdio: 'ignore',
    env: smokeEnvironment(directory),
  });
  return { prepared, child };
}

async function main() {
  if (process.platform !== 'darwin' || process.argv.length !== 4) throw new Error();
  const identifier = process.argv[3];
  if (!isSmokeIdentifier(identifier)) throw new Error();
  const bundle = resolve(process.argv[2]);
  if (!(await stat(bundle)).isDirectory() || !bundle.endsWith('.app')) throw new Error();
  const temporary = await mkdtemp(join(tmpdir(), 'xtrace-native-smoke-'));
  const script = join(dirname(fileURLToPath(import.meta.url)), 'debug-bundle.swift');
  const inspector = join(temporary, 'inspect');
  let child;
  try {
    const compile = spawnSync('swiftc', [script, '-o', inspector], {
      stdio: 'pipe',
      timeout: 120_000,
    });
    if (compile.error || compile.status !== 0) throw new Error();
    const env = smokeEnvironment(temporary);
    await mkdir(env.XTRACE_NATIVE_HOME);
    const launched = await launchSmokeBundle(bundle, temporary, { identifier });
    child = launched.child;
    await new Promise((resolve, reject) => {
      child.once('spawn', resolve);
      child.once('error', reject);
    });
    const inspect = spawnSync(inspector, inspectorArguments(child.pid, launched.prepared), {
      stdio: 'pipe',
      timeout: 35_000,
    });
    if (inspect.error || inspect.status !== 0 || child.exitCode !== null) throw new Error();
    console.log('Debug app bundle launched and created its native main window.');
  } finally {
    // Only terminate the exact child created here; never kill by app name.
    if (child && child.exitCode === null) {
      child.kill('SIGTERM');
      await new Promise((resolve) => {
        const timeout = setTimeout(() => {
          child.kill('SIGKILL');
          resolve();
        }, 3000);
        child.once('exit', () => {
          clearTimeout(timeout);
          resolve();
        });
      });
    }
    await rm(temporary, { recursive: true, force: true });
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(() => {
    console.error('Debug bundle smoke failed; native launch evidence is unavailable.');
    process.exitCode = 1;
  });
}
