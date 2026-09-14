import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checkSourcePolicy, validateDescription, sourceCommits } from './check-pr.mjs';

const repository = 'example/project';
const base = 'a'.repeat(40);
const synthetic = 'b'.repeat(40);
const body = () =>
  `## Change\n\nAdd a synthetic capability.\n\n## Verification\n\nRun the synthetic case; expect success.\n`;

function fixture(commits) {
  const head = commits.at(-1).sha;
  const state = {
    reads: 0,
    pr: {
      number: 3,
      commits: commits.length,
      state: 'open',
      title: 'Synthetic change',
      body: body(),
      base: { sha: base, ref: 'main', repo: { full_name: repository } },
      head: { sha: head, ref: 'feat/desktop-ci' },
    },
    commits,
  };
  state.api = async (path) => {
    const root = '/repos/' + repository;
    if (path === root) return { default_branch: 'main' };
    if (path === root + '/commits/main') return { sha: base };
    if (path === root + '/pulls/3') {
      state.reads++;
      return structuredClone(state.pr);
    }
    if (path === root + '/pulls/3/commits?per_page=100&page=1')
      return structuredClone(state.commits);
    if (path === '/graphql')
      return {
        data: {
          repository: {
            mergeQueue: {
              entries: {
                totalCount: 1,
                pageInfo: { hasNextPage: false, endCursor: null },
                nodes: [
                  {
                    baseCommit: { oid: base },
                    headCommit: { oid: synthetic },
                    pullRequest: { number: 3, headRefOid: state.pr.head.sha },
                  },
                ],
              },
            },
          },
        },
      };
    throw new Error('Unexpected synthetic API route');
  };
  return state;
}

const execute = (state, eventName = 'pull_request', extra = {}) =>
  checkSourcePolicy({
    api: state.api,
    repository,
    eventName,
    event: {
      repository: { full_name: repository },
      action: 'checks_requested',
      pull_request: state.pr,
      merge_group: { base_ref: 'refs/heads/main', base_sha: base, head_sha: synthetic },
    },
    ...extra,
  });

test('public PR descriptions require substantive change and verification sections', () => {
  assert.doesNotThrow(() => validateDescription(body()));
  for (const verification of [
    '- Run `pnpm check`; expect all checks to pass.\n- Actual result: passed.',
    '| Case | Result |\n| --- | --- |\n| Synthetic check | Passed |',
  ])
    assert.doesNotThrow(() =>
      validateDescription(body().replace('Run the synthetic case; expect success.', verification)),
    );
  for (const text of [
    '',
    body().replace('## Verification', '## Missing'),
    body().replace('Run the synthetic case; expect success.', '- Describe each acceptance case.'),
    body().replace('Add a synthetic capability.', 'Describe the problem and resulting behavior.'),
    '```md\n' + body() + '\n```',
    body().replace('Run the synthetic case; expect success.', '<!-- Test results -->'),
  ])
    assert.throws(() => validateDescription(text), /Change and Verification/);
});

test('real Git source commits drive DCO on PR and queue; unsigned bootstrap/synthetic commits are not substituted', async () => {
  const root = await mkdtemp(join(tmpdir(), 'xtrace-dco-test-'));
  const git = (...args) =>
    execFileSync('git', ['-c', 'core.hooksPath=/dev/null', ...args], {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: 'Test Contributor',
        GIT_AUTHOR_EMAIL: 'contributor@example.com',
        GIT_COMMITTER_NAME: 'Test Contributor',
        GIT_COMMITTER_EMAIL: 'contributor@example.com',
      },
    }).trim();
  try {
    git('init', '-q');
    git('commit', '--allow-empty', '-m', 'Unsigned bootstrap');
    git('commit', '--allow-empty', '-s', '-m', 'Signed contribution');
    const object = () => ({
      sha: git('rev-parse', 'HEAD'),
      commit: {
        message: git('show', '-s', '--format=%B', 'HEAD'),
        author: {
          name: git('show', '-s', '--format=%an', 'HEAD'),
          email: git('show', '-s', '--format=%ae', 'HEAD'),
        },
      },
    });
    const signed = object();
    git('commit', '--allow-empty', '-m', 'Unsigned contribution');
    const unsigned = object();
    for (const event of ['pull_request', 'merge_group']) {
      assert.equal(await execute(fixture([signed]), event), 1);
      await assert.rejects(execute(fixture([signed, unsigned]), event), /DCO sign-off/);
    }
    git(
      'commit',
      '--allow-empty',
      '--author=Test Maintainer <maintainer@example.com>',
      '-m',
      'Squashed contribution\n\nSigned-off-by: Test Contributor <contributor@example.com>',
    );
    const squash = object();
    // The same author/trailer mismatch must fail as a contributed PR commit,
    // while the post-merge build must not treat it as a new contribution.
    await assert.rejects(execute(fixture([squash])), /DCO sign-off/);
    assert.equal(
      await execute(fixture([squash]), 'push', {
        event: {
          repository: { full_name: repository, default_branch: 'main' },
          ref: 'refs/heads/main',
          before: signed.sha,
          after: squash.sha,
        },
      }),
      null,
    );
    const state = fixture([signed]);
    state.pr.commits = 251;
    await assert.rejects(sourceCommits(state.api, repository, state.pr), /pagination/);
    state.pr.commits = 2;
    state.commits = [signed, signed];
    await assert.rejects(sourceCommits(state.api, repository, state.pr), /pagination/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('post-merge policy only accepts the expected default-branch push and valid source identities', async () => {
  const event = {
    repository: { full_name: repository, default_branch: 'main' },
    ref: 'refs/heads/main',
    before: base,
    after: synthetic,
  };
  const api = async () => assert.fail('Post-merge policy must not resolve mutable source PRs.');
  const run = (value) => checkSourcePolicy({ api, repository, eventName: 'push', event: value });
  assert.equal(await run(event), null);
  for (const value of [
    { ...event, repository: { ...event.repository, full_name: 'other/project' } },
    { ...event, ref: 'refs/heads/feature' },
    { ...event, repository: { full_name: repository }, ref: 'refs/heads/undefined' },
    { ...event, before: 'invalid' },
    { ...event, after: 'invalid' },
  ])
    await assert.rejects(run(value));
});

test('signed contributions pass on ordinary contributor branches', async () => {
  const source = {
    sha: 'c'.repeat(40),
    commit: {
      message: 'Change\n\nSigned-off-by: Test Contributor <contributor@example.com>',
      author: { name: 'Test Contributor', email: 'contributor@example.com' },
    },
  };
  for (const branch of ['feat/desktop-ci', 'fix/startup', 'contributor/topic']) {
    const state = fixture([source]);
    state.pr.head.ref = branch;
    for (const event of ['pull_request', 'merge_group'])
      assert.equal(await execute(state, event), 1);
  }
});

test('source metadata changes and queue API failure cannot leave a successful policy result', async () => {
  const source = {
    sha: 'c'.repeat(40),
    commit: {
      message: 'Change\n\nSigned-off-by: Test Contributor <contributor@example.com>',
      author: { name: 'Test Contributor', email: 'contributor@example.com' },
    },
  };
  const state = fixture([source]);
  const api = state.api;
  state.api = async (path, ...args) => {
    const result = await api(path, ...args);
    if (path.endsWith('/pulls/3') && state.reads > 1) result.head.sha = 'd'.repeat(40);
    return result;
  };
  await assert.rejects(execute(state), /changed/);
  state.api = async () => {
    throw new Error('Synthetic unavailable API');
  };
  await assert.rejects(execute(state, 'merge_group'));
});

test('a multi-PR merge group validates every constituent source commit', async () => {
  const signed = {
    sha: 'c'.repeat(40),
    commit: {
      message: 'Change\n\nSigned-off-by: Test Contributor <contributor@example.com>',
      author: { name: 'Test Contributor', email: 'contributor@example.com' },
    },
  };
  const state = fixture([signed]);
  const next = fixture([
    { ...signed, sha: 'd'.repeat(40), commit: { ...signed.commit, message: 'Unsigned second PR' } },
  ]);
  next.pr.number = 4;
  const original = state.api;
  state.api = async (path, ...args) => {
    if (path === '/repos/' + repository + '/pulls/4') return structuredClone(next.pr);
    if (path === '/repos/' + repository + '/pulls/4/commits?per_page=100&page=1')
      return structuredClone(next.commits);
    if (path === '/graphql')
      return {
        data: {
          repository: {
            mergeQueue: {
              entries: {
                totalCount: 2,
                pageInfo: { hasNextPage: false, endCursor: null },
                nodes: [
                  {
                    baseCommit: { oid: base },
                    headCommit: { oid: 'e'.repeat(40) },
                    pullRequest: { number: 3, headRefOid: state.pr.head.sha },
                  },
                  {
                    baseCommit: { oid: 'e'.repeat(40) },
                    headCommit: { oid: synthetic },
                    pullRequest: { number: 4, headRefOid: next.pr.head.sha },
                  },
                ],
              },
            },
          },
        },
      };
    return original(path, ...args);
  };
  await assert.rejects(execute(state, 'merge_group'), /DCO sign-off/);
  next.commits[0].commit.message = signed.commit.message;
  assert.equal(await execute(state, 'merge_group'), 2);
});
