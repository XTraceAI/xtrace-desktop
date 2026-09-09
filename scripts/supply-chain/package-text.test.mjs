import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { packageText } from './package-text.mjs';

test('notice sources stay inside their package even through symlinks', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'synthetic-notice-boundary-'));
  try {
    const root = join(temporary, 'package');
    await mkdir(root);
    await writeFile(join(root, 'LICENSE'), 'Synthetic package attribution.');
    await writeFile(join(temporary, 'outside'), 'Synthetic outside-package text.');
    await symlink('LICENSE', join(root, 'COPYING'));
    await symlink('../outside', join(root, 'NOTICE'));
    assert.equal(await packageText(root, 'COPYING'), 'Synthetic package attribution.');
    for (const file of ['NOTICE', '../outside', join(temporary, 'outside')])
      await assert.rejects(packageText(root, file), /outside its package/);
    await assert.rejects(packageText(root, 'missing'));
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});
