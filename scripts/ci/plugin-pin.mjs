import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { CheckError } from './policy.mjs';

// The pin names one reviewed producer revision. Conformance runs against that
// exact checkout, and the reader sources it lists are verified by Git object
// identity, so a checkout at the right commit with edited files cannot pass.
const REPOSITORY = 'https://github.com/XTraceAI/agent-plugins.git';
const OBJECT_ID = /^[0-9a-f]{40}$/;
const VERSION = /^\d+\.\d+\.\d+$/;
const RELATIVE = /^[A-Za-z0-9_][A-Za-z0-9_./-]*$/;

export function parsePin(text) {
  let pin;
  try {
    pin = JSON.parse(text);
  } catch {
    throw new CheckError('.plugin-pin must be a JSON object.');
  }
  if (!pin || typeof pin !== 'object' || Array.isArray(pin))
    throw new CheckError('.plugin-pin must be a JSON object.');
  const keys = Object.keys(pin).sort().join(',');
  if (keys !== 'commit,plugin_root,plugin_version,reader_sources,repository')
    throw new CheckError('.plugin-pin must declare exactly its known keys.');
  if (pin.repository !== REPOSITORY)
    throw new CheckError('.plugin-pin repository is not the reviewed public producer.');
  if (!OBJECT_ID.test(pin.commit)) throw new CheckError('.plugin-pin commit must be a full SHA.');
  if (!RELATIVE.test(pin.plugin_root) || pin.plugin_root.split('/').includes('..'))
    throw new CheckError('.plugin-pin plugin_root must be a relative path.');
  if (!VERSION.test(pin.plugin_version))
    throw new CheckError('.plugin-pin plugin_version must be a release version.');
  const sources = pin.reader_sources;
  if (!sources || typeof sources !== 'object' || Array.isArray(sources))
    throw new CheckError('.plugin-pin reader_sources must map paths to object IDs.');
  const entries = Object.entries(sources);
  if (entries.length === 0) throw new CheckError('.plugin-pin must list reader sources.');
  for (const [path, id] of entries) {
    if (!RELATIVE.test(path) || path.split('/').includes('..') || !OBJECT_ID.test(id))
      throw new CheckError('.plugin-pin reader_sources must map relative paths to object IDs.');
  }
  return pin;
}

export async function readPin(root) {
  let text;
  try {
    text = await readFile(resolve(root, '.plugin-pin'), 'utf8');
  } catch {
    throw new CheckError('.plugin-pin is missing.');
  }
  return parsePin(text);
}

// Resolves the pin against a checkout: HEAD and every listed reader source
// must be the exact pinned objects. `git` runs the command and returns stdout.
export function verifyCheckout(pin, git) {
  const head = git(['rev-parse', 'HEAD']).trim();
  if (head !== pin.commit) throw new CheckError('Plugin checkout is not at the pinned commit.');
  for (const [path, id] of Object.entries(pin.reader_sources)) {
    let actual;
    try {
      actual = git(['rev-parse', '--verify', `HEAD:${path}`]).trim();
    } catch {
      throw new CheckError(`Pinned reader source is absent: ${path}`);
    }
    if (actual !== id) throw new CheckError(`Pinned reader source differs: ${path}`);
  }
  if (git(['status', '--porcelain']).trim())
    throw new CheckError('Plugin checkout has local modifications.');
  return head;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const field = process.argv[2];
  readPin(process.cwd())
    .then((pin) => {
      if (!['repository', 'commit', 'plugin_root', 'plugin_version'].includes(field))
        throw new CheckError(
          'Usage: node scripts/ci/plugin-pin.mjs <repository|commit|plugin_root|plugin_version>',
        );
      console.log(pin[field]);
    })
    .catch((error) => {
      console.error(error instanceof CheckError ? error.message : 'Pin could not be read.');
      process.exitCode = 1;
    });
}
