import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

export function prepareUpdaterRelease(args, env = process.env) {
  if (args.length !== 1) throw new Error('Provide the release updater overlay path.');
  const path = resolve(args[0]);
  const overlay = JSON.parse(readFileSync(path, 'utf8'));
  if (overlay.bundle?.createUpdaterArtifacts !== true || !overlay.plugins?.updater)
    throw new Error('The release overlay must configure updater and createUpdaterArtifacts: true.');
  if (!env.TAURI_SIGNING_PRIVATE_KEY?.trim())
    throw new Error('Updater releases require TAURI_SIGNING_PRIVATE_KEY.');
  return { path, env: { ...env, XTRACE_UPDATER_RELEASE: '1' } };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const release = prepareUpdaterRelease(process.argv.slice(2));
    const result = spawnSync('pnpm', ['tauri', 'build', '--config', release.path], {
      env: release.env,
      stdio: 'inherit',
    });
    process.exitCode = result.status ?? 1;
  } catch (error) {
    // Do not print parser contents or configuration/secret values.
    console.error(error instanceof SyntaxError ? 'Invalid release overlay JSON.' : error.message);
    process.exitCode = 1;
  }
}
