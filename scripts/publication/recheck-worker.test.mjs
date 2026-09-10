import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { runBounded } from './recheck-worker.mjs';

test('slow jobs exhaust their own budgets without starving later scheduled PRs', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'publication-scheduling-'));
  try {
    const module = join(directory, 'scan.mjs');
    await writeFile(
      module,
      `
      import { writeFileSync } from 'node:fs';
      writeFileSync(process.env.ATTEMPT, 'attempted');
      if (process.env.BLOCK === '1') Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0);
    `,
    );
    const results = [];
    const started = Date.now();
    // Two available runners; four early scans exceed the old shared budget.
    // The later two still receive their own complete execution budget.
    let next = 0;
    await Promise.all(
      Array.from({ length: 2 }, async () => {
        for (let index; (index = next++) < 6;) {
          results[index] = await runBounded(module, {
            timeoutMs: 400,
            env: {
              ...process.env,
              ATTEMPT: join(directory, String(index)),
              BLOCK: index < 4 ? '1' : '0',
            },
          });
        }
      }),
    );
    assert.ok(Date.now() - started >= 800);
    assert.deepEqual(
      results.map((result) => result.timedOut),
      [true, true, true, true, false, false],
    );
    assert.deepEqual(
      results.map((result) => result.code),
      [1, 1, 1, 1, 0, 0],
    );
    for (let index = 0; index < 6; index++)
      assert.equal(await readFile(join(directory, String(index)), 'utf8'), 'attempted');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('the deadline kills scanner descendants and prevents delayed writes', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'publication-deadline-'));
  try {
    const module = join(directory, 'scan.mjs');
    const descendant = join(directory, 'descendant.mjs');
    const marker = join(directory, 'late-write');
    await writeFile(
      descendant,
      `
      import { writeFileSync } from 'node:fs';
      setTimeout(() => writeFileSync(process.env.MARKER, 'late success'), 800);
    `,
    );
    await writeFile(
      module,
      `
      import { spawnSync } from 'node:child_process';
      spawnSync(process.execPath, [process.env.DESCENDANT], { stdio: 'inherit' });
    `,
    );
    assert.deepEqual(
      await runBounded(module, {
        timeoutMs: 400,
        env: { ...process.env, DESCENDANT: descendant, MARKER: marker },
      }),
      { code: 1, timedOut: true },
    );
    await new Promise((resolve) => setTimeout(resolve, 900));
    await assert.rejects(readFile(marker), { code: 'ENOENT' });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('the workflow schedules isolated bounded jobs without sibling cancellation', async () => {
  const workflow = await readFile(
    new URL('../../.github/workflows/publication-content.yml', import.meta.url),
    'utf8',
  );
  assert.match(workflow, /fail-fast: false/);
  assert.match(workflow, /max-parallel: 4/);
  assert.match(workflow, /matrix: \$\{\{ fromJSON\(needs\.prepare\.outputs\.matrix\) \}\}/);
  assert.match(workflow, /ref: \$\{\{ needs\.prepare\.outputs\.trusted \}\}/);
  assert.match(workflow, /run: node scripts\/publication\/recheck-worker\.mjs/);
  assert.match(workflow, /if: \$\{\{ needs\.prepare\.outputs\.count != '0' \}\}/);
});
