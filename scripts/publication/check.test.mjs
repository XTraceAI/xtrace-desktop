import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { readPublicContent } from './content.mjs';
import { ATTESTATION } from './metadata.mjs';

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

test('real local CLI removes fetched refs after success, fetch failure, mismatch and scan rejection', async (t) => {
  const realGit = spawnSync('which', ['git'], { encoding: 'utf8' }).stdout.trim();
  for (const outcome of ['success', 'fetch-failure', 'mismatch', 'secret']) {
    await t.test(outcome, async (t) => {
      const temporary = await mkdtemp(join(tmpdir(), 'publication-local-'));
      t.after(() => rm(temporary, { recursive: true, force: true }));
      const repo = join(temporary, 'repo');
      const bin = join(temporary, 'bin');
      await mkdir(repo);
      await mkdir(bin);
      const git = (...args) => {
        const result = spawnSync(realGit, ['-c', 'core.hooksPath=/dev/null', ...args], {
          cwd: repo,
          encoding: 'utf8',
        });
        assert.equal(result.status, 0, 'Synthetic Git operation must succeed.');
        return result.stdout.trim();
      };
      git('init', '-q', '-b', 'main');
      git('config', 'user.name', 'Synthetic Contributor');
      git('config', 'user.email', 'contributor@example.invalid');
      await writeFile(join(repo, 'README.md'), 'Clean baseline.\n');
      git('add', '.');
      git('commit', '-qm', 'Synthetic baseline');
      const base = git('rev-parse', 'HEAD');
      const secret = ['gh', 'p_'].join('') + randomBytes(18).toString('hex');
      await writeFile(join(repo, 'README.md'), outcome === 'secret' ? secret : 'Clean candidate.');
      git('add', '.');
      git('commit', '-qm', 'Synthetic candidate');
      const head = git('rev-parse', 'HEAD');
      const repository = 'example/project';
      const pr = {
        number: 3,
        commits: 1,
        state: 'open',
        title: 'Synthetic PR',
        body: '- [x] ' + ATTESTATION + '\n\nDisclosure snapshot: pending\n',
        base: { sha: base, ref: 'main', repo: { full_name: repository } },
        head: { sha: head, ref: 'feat/synthetic' },
      };
      const responses = {
        '/graphql': {
          data: {
            repository: {
              pullRequest: {
                closingIssuesReferences: {
                  nodes: [],
                  totalCount: 0,
                  pageInfo: { hasNextPage: false },
                },
              },
            },
          },
        },
        '/repos/example/project/pulls/3': pr,
        '/repos/example/project/pulls/3/commits': [{ sha: head }],
        '/repos/example/project/pulls/3/comments': [],
        '/repos/example/project/pulls/3/reviews': [],
        '/repos/example/project/issues/3/comments': [],
        '/repos/example/project/comments': [],
      };
      const review = await readPublicContent(
        async (path) => structuredClone(responses[path.split('?')[0]]),
        repository,
        3,
      );
      pr.body = pr.body.replace('pending', review.digest);
      const data = join(temporary, 'responses.json');
      await writeFile(data, JSON.stringify(responses), { mode: 0o600 });
      const preload = join(temporary, 'transport.mjs');
      await writeFile(
        preload,
        `
import { readFileSync } from 'node:fs';
const original = globalThis.fetch;
globalThis.fetch = async (url, options) => {
  if (new URL(url).hostname !== 'api.github.com') return original(url, options);
  const responses = JSON.parse(readFileSync(${JSON.stringify(data)}, 'utf8'));
  const response = responses[new URL(url).pathname];
  if (!response) throw new Error('Unexpected synthetic API route');
  return new Response(JSON.stringify(response), { status: 200 });
};
`,
        { mode: 0o600 },
      );
      const observed = join(temporary, 'fetched-ref.txt');
      await writeFile(
        join(bin, 'git'),
        `#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const { writeFileSync } = require('node:fs');
const args = process.argv.slice(2);
if (args.includes('fetch')) {
  const ref = args.at(-1).split(':')[1];
  writeFileSync(${JSON.stringify(observed)}, ref, { mode: 0o600 });
  const result = spawnSync(${JSON.stringify(realGit)}, ['update-ref', ref, ${JSON.stringify(outcome === 'mismatch' ? base : head)}]);
  process.exit(${outcome === 'fetch-failure' ? 128 : 'result.status'});
}
const result = spawnSync(${JSON.stringify(realGit)}, args, { stdio: 'inherit' });
process.exit(result.status);
`,
        { mode: 0o700 },
      );
      const result = spawnSync(
        process.execPath,
        [command, '--repo', repo, '--repository', repository, '--pr', '3'],
        {
          encoding: 'utf8',
          timeout: 120_000,
          env: {
            ...process.env,
            GITHUB_ACTIONS: '',
            GITHUB_TOKEN: 'synthetic-read-token',
            GH_TOKEN: '',
            PATH: bin + ':' + process.env.PATH,
            NODE_OPTIONS: '--import=' + preload,
          },
        },
      );
      assert.equal(
        result.status,
        outcome === 'success' ? 0 : 1,
        'The real CLI must complete the selected outcome.',
      );
      assert.match(await readFile(observed, 'utf8'), /^refs\/publication-local\/[a-f0-9-]+$/);
      assert.equal(git('for-each-ref', '--format=%(refname)', 'refs/publication-local/'), '');
      assert.equal(git('rev-parse', 'HEAD'), head);
      assert.ok(
        !(result.stdout + result.stderr).includes(secret),
        'CLI output must not expose the marker.',
      );
    });
  }
});
