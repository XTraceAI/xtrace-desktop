import assert from 'node:assert/strict';
import test from 'node:test';
import { spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const inventory = ['conformance_alpha', 'conformance_beta'];
const ok = (name) => `test ${name} ... ok\n`;
const summary = (passed) =>
  `\ntest result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n`;

async function validate(log, names = inventory) {
  const root = await mkdtemp(join(tmpdir(), 'xtrace-conformance-log-'));
  try {
    await writeFile(join(root, 'log'), log);
    await writeFile(join(root, 'inventory'), names.map((name) => `${name}\n`).join(''));
    const result = spawnSync(
      'sh',
      ['scripts/ci/assert-no-skipped-conformance.sh', join(root, 'log'), join(root, 'inventory')],
      { encoding: 'utf8' },
    );
    return result;
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test('every inventoried test must run and pass; the executed names are listed', async () => {
  const result = await validate(ok('conformance_alpha') + ok('conformance_beta') + summary(2));
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /executed conformance_alpha/);
  assert.match(result.stdout, /executed 2 of 2 required conformance tests/);
});

test('self-skips, ignored tests, failures, missing names and empty logs are red', async () => {
  const cases = [
    [
      'SKIP conformance_alpha: set AGENT_PLUGINS_DIR\n' +
        ok('conformance_alpha') +
        ok('conformance_beta') +
        summary(2),
      /skipped itself instead of running: conformance_alpha/,
    ],
    ['test conformance_alpha ... ignored\n' + ok('conformance_beta') + summary(1), /ignored/],
    [
      'test conformance_alpha ... FAILED\n' +
        ok('conformance_beta') +
        '\ntest result: FAILED. 1 passed; 1 failed\n',
      /failed/,
    ],
    [ok('conformance_alpha') + summary(1), /did not run and pass: conformance_beta/],
    [
      ok('conformance_alpha') + ok('conformance_beta_extended') + summary(2),
      /did not run and pass: conformance_beta/,
    ],
    ['', /missing or empty/],
  ];
  for (const [log, reason] of cases) {
    const result = await validate(log);
    assert.equal(result.status, 1, log);
    assert.match(result.stderr, reason, log);
  }
});

test('a skipped test is not counted as executed', async () => {
  const result = await validate(
    'SKIP conformance_alpha: Python 3 is unavailable\n' +
      ok('conformance_alpha') +
      ok('conformance_beta') +
      summary(2),
  );
  assert.equal(result.status, 1);
  assert.match(result.stdout, /executed 1 of 2 required conformance tests/);
  assert.doesNotMatch(result.stdout, /executed conformance_alpha/);
});

test('the committed inventory names the required workspace conformance tests', async () => {
  const { readFile } = await import('node:fs/promises');
  const names = (await readFile('scripts/ci/conformance-inventory.txt', 'utf8')).trim().split('\n');
  assert.deepEqual(names, [
    'conformance_flush_turn',
    'conformance_plugin_transport',
    'conformance_native_reader_stream',
    'conformance_native_import',
    'conformance_bundled_readers',
    'conformance_exact_detail',
  ]);
  const result = await validate(names.map(ok).join('') + summary(names.length), names);
  assert.equal(result.status, 0, result.stderr);
});
