import { spawn } from 'node:child_process';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// The supervisor remains responsive even while the trusted scanner uses sync
// Git/native calls. Each Actions matrix job has its own checkout and process group.
export function runBounded(module, { timeoutMs = 240_000, env = process.env } = {}) {
  return new Promise((resolveResult) => {
    const child = spawn(process.execPath, [module, 'check'], {
      env,
      detached: true,
      stdio: ['ignore', 'inherit', 'inherit'],
    });
    let timedOut = false;
    let cancelled = false;
    const kill = () => {
      try {
        process.kill(-child.pid, 'SIGKILL');
      } catch {
        // A child already exiting still cannot turn a timeout into success.
      }
    };
    const cancel = () => {
      cancelled = true;
      kill();
    };
    process.once('SIGTERM', cancel);
    process.once('SIGINT', cancel);
    const deadline = setTimeout(() => {
      timedOut = true;
      kill();
    }, timeoutMs);
    const cleanup = () => {
      clearTimeout(deadline);
      process.removeListener('SIGTERM', cancel);
      process.removeListener('SIGINT', cancel);
    };
    child.once('error', () => {
      cleanup();
      resolveResult({ code: 1, timedOut });
    });
    child.once('close', (code) => {
      cleanup();
      resolveResult({ code: timedOut || cancelled ? 1 : (code ?? 1), timedOut });
    });
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = await runBounded(fileURLToPath(new URL('./recheck.mjs', import.meta.url)));
  if (result.timedOut)
    console.error('Content scan exceeded its four-minute budget; its check remains failed.');
  process.exitCode = result.code;
}
