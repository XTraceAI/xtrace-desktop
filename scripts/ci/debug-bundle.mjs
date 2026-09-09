import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

async function main() {
  if (process.platform !== 'darwin' || process.argv.length !== 3) throw new Error();
  const bundle = resolve(process.argv[2]);
  if (!(await stat(bundle)).isDirectory() || !bundle.endsWith('.app')) throw new Error();
  const plist = spawnSync(
    '/usr/libexec/PlistBuddy',
    ['-c', 'Print :CFBundleExecutable', join(bundle, 'Contents/Info.plist')],
    { encoding: 'utf8' },
  );
  const executable = plist.stdout?.trim();
  if (plist.status !== 0 || !executable || !/^[A-Za-z0-9_. -]+$/.test(executable))
    throw new Error();
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
    child = spawn(join(bundle, 'Contents/MacOS', executable), [], {
      stdio: 'ignore',
      env: { ...process.env, GITHUB_TOKEN: '', GH_TOKEN: '', XTRACE_TELEMETRY: '0' },
    });
    await new Promise((resolve, reject) => {
      child.once('spawn', resolve);
      child.once('error', reject);
    });
    const inspect = spawnSync(inspector, [String(child.pid), bundle], {
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
main().catch(() => {
  console.error('Debug bundle smoke failed; native launch evidence is unavailable.');
  process.exitCode = 1;
});
