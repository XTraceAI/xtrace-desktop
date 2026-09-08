import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, rm, writeFile, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runHook } from './run-hook.mjs';

test('pre-owner absence is explicit and actual installed failures are fatal', async () => {
  const root = await mkdtemp(join(tmpdir(), 'xtrace-hook-test-'));
  try {
    await mkdir(join(root, 'scripts/ci'), { recursive: true });
    assert.equal(await runHook('plugin-conformance', root), 'not installed yet');
    const file = join(root, 'scripts/ci/plugin-conformance.sh');
    await writeFile(file, '#!/bin/sh\nexit 1\n');
    await assert.rejects(runHook('plugin-conformance', root), /failed/);
    await writeFile(file, '#!/bin/sh\nexit 0\n');
    assert.equal(await runHook('plugin-conformance', root), 'passed');
    await rm(file);
    await writeFile(join(root, '.plugin-pin'), 'synthetic-pin\n');
    await assert.rejects(runHook('plugin-conformance', root), /missing/);
    await symlink('/dev/null', file);
    await assert.rejects(runHook('plugin-conformance', root), /regular/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('default-branch declaration makes deletion fail even if all candidate signals disappear', async () => {
  const root = await mkdtemp(join(tmpdir(), 'xtrace-hook-test-'));
  try {
    const base = join(root, 'base');
    const source = join(root, 'source');
    await mkdir(join(base, 'scripts/ci'), { recursive: true });
    await mkdir(source);
    await writeFile(join(base, 'scripts/ci/check-dto.sh'), '#!/bin/sh\nexit 0\n');
    await assert.rejects(runHook('dto', source, { base }), /missing/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
