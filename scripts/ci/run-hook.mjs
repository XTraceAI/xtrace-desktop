import { spawnSync } from 'node:child_process';
import { lstat, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { CheckError } from './policy.mjs';

const hooks = {
  'plugin-conformance': {
    path: 'scripts/ci/plugin-conformance.sh',
    signals: ['.plugin-pin', 'scripts/ci/assert-no-skipped-conformance.sh'],
  },
  dto: { path: 'scripts/ci/check-dto.sh', signals: ['apps/desktop/ui/src/data/generated'] },
};

async function exists(path) {
  try {
    return await lstat(path);
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw new CheckError('Hook presence could not be checked.');
  }
}

export async function runHook(name, root, { base, run = spawnSync } = {}) {
  const hook = hooks[name];
  if (!hook) throw new CheckError('Unknown integration hook.');
  const file = await exists(resolve(root, hook.path));
  const signals = await Promise.all(hook.signals.map((path) => exists(resolve(root, path))));
  const previous =
    base &&
    (
      await Promise.all([hook.path, ...hook.signals].map((path) => exists(resolve(base, path))))
    ).some(Boolean);
  if (!file) {
    if (previous || signals.some(Boolean))
      throw new CheckError('Declared integration hook is missing.');
    return 'not installed yet';
  }
  if (!file.isFile() || file.isSymbolicLink())
    throw new CheckError('Integration hook must be a regular source file.');
  const first = (await readFile(resolve(root, hook.path), 'utf8')).split('\n')[0];
  if (!['#!/bin/sh', '#!/bin/bash'].includes(first))
    throw new CheckError('Integration hook interpreter is unsupported.');
  const result = run(first.slice(2), [hook.path], {
    cwd: root,
    stdio: 'inherit',
    timeout: 600_000,
    env: { ...process.env, GITHUB_TOKEN: '', GH_TOKEN: '' },
  });
  if (result.error || result.status !== 0)
    throw new CheckError('Installed integration hook failed.');
  return 'passed';
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  runHook(process.argv[2], process.cwd(), { base: process.env.CI_TRUSTED_ROOT })
    .then((result) => {
      console.log(`Integration hook ${result}.`);
    })
    .catch(() => {
      console.error('Integration hook is missing, invalid or failed.');
      process.exitCode = 1;
    });
}
