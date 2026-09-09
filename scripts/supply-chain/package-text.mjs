import { readFile, realpath } from 'node:fs/promises';
import { isAbsolute, relative, resolve } from 'node:path';

export async function packageText(directory, file) {
  const root = await realpath(directory);
  const path = await realpath(resolve(root, file));
  const rel = relative(root, path);
  if (rel === '..' || rel.startsWith('../') || isAbsolute(rel))
    throw new Error('License source is outside its package.');
  return readFile(path, 'utf8');
}
