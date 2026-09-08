import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const command = fileURLToPath(new URL('./check.mjs', import.meta.url));

test('real CLI fails without credentials and does not echo unrelated environment values', () => {
  const result = spawnSync(process.execPath, [command, '--repository', 'example/project'], {
    encoding: 'utf8',
    env: {
      ...process.env,
      GITHUB_TOKEN: '',
      GH_TOKEN: '',
      PRIVATE_TEST_VALUE: 'synthetic-private-value',
    },
  });
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  assert.equal(result.stderr.trim(), 'A read-only GITHUB_TOKEN or GH_TOKEN is required.');
  assert.doesNotMatch(result.stderr, /synthetic-private-value/);
});

test('real CLI hides paths and credentials when event-file loading fails', async () => {
  const repo = await mkdtemp(join(tmpdir(), 'publication-cli-test-'));
  try {
    assert.equal(spawnSync('git', ['init', '-q', repo]).status, 0);
    const result = spawnSync(
      process.execPath,
      [command, '--repo', repo, '--repository', 'example/project'],
      {
        encoding: 'utf8',
        env: {
          ...process.env,
          GITHUB_TOKEN: 'synthetic-token-value',
          GH_TOKEN: '',
          GITHUB_EVENT_NAME: 'pull_request',
          GITHUB_EVENT_PATH: join(repo, 'private-missing-event.json'),
        },
      },
    );
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    assert.equal(
      result.stderr.trim(),
      'Publication check failed while reading required metadata or temporary content.',
    );
    assert.doesNotMatch(result.stderr, /private-missing-event|synthetic-token-value|ENOENT/);
  } finally {
    await rm(repo, { recursive: true, force: true });
  }
});
