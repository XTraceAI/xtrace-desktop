import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { ATTESTATION } from '../publication/metadata.mjs';
import { checkSourcePolicy, planSlot, sourceCommits } from './check-pr.mjs';

const repository = 'example/project';
const base = 'a'.repeat(40);
const synthetic = 'b'.repeat(40);
const policy = {
  version: 1,
  slots: { 'FND-02': { stage: 1 }, 'FND-12': { stage: 3 } },
  maintenance: { 'publication-checks': { stage: 1, branch: 'feat/publication-checks' } },
};
const body = (control = 'Plan slot: FND-02') =>
  `${control}\n\n## Change\n\nAdd a synthetic capability.\n\n## Verification\n\nRun the synthetic case; expect success.\n\n- [x] ${ATTESTATION}\n`;

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
      head: { sha: head, ref: 'feat/fnd-02-ci' },
    },
    commits,
  };
  state.api = async (path) => {
    const root = '/repos/' + repository;
    if (path === root) return { default_branch: 'main' };
    if (path === root + '/commits/main') return { sha: base };
    if (path === root + '/contents/scripts/ci/stages.json?ref=' + base)
      return {
        type: 'file',
        encoding: 'base64',
        size: 500,
        content: Buffer.from(JSON.stringify(policy)).toString('base64'),
      };
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

test('classification is an explicit real paragraph bound to a curated branch, with acceptance sections', () => {
  assert.equal(planSlot(body(), 'feat/fnd-02-ci', policy).stage, 1);
  assert.equal(
    planSlot(body('Maintenance: publication-checks'), 'feat/publication-checks', policy).stage,
    1,
  );
  for (const verification of [
    '- Run `pnpm check`; expect all checks to pass.\n- Actual result: passed.',
    '| Case | Result |\n| --- | --- |\n| Synthetic check | Passed |',
  ])
    assert.equal(
      planSlot(
        body().replace('Run the synthetic case; expect success.', verification),
        'feat/fnd-02-ci',
        policy,
      ).stage,
      1,
    );
  for (const text of [
    body('Plan slot: UNKNOWN'),
    body('`Plan slot: FND-02`'),
    body('```\nPlan slot: FND-02\n```'),
    body('<!-- Plan slot: FND-02 -->'),
    body('Plan slot: FND-02\n\nPlan slot: FND-12'),
    body().replace('## Verification', '## Missing'),
    body().replace('Run the synthetic case; expect success.', '- Describe each acceptance case.'),
  ])
    assert.throws(() => planSlot(text, 'feat/fnd-02-ci', policy));
  assert.throws(() => planSlot(body(), 'feat/fnd-12-other', policy));
  assert.throws(() =>
    planSlot(body('Maintenance: publication-checks'), 'feat/fnd-12-other', policy),
  );
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

test('dependent stage fails without approval while producer foundations pass without configured checkpoints', async () => {
  const source = {
    sha: 'c'.repeat(40),
    commit: {
      message: 'Change\n\nSigned-off-by: Test Contributor <contributor@example.com>',
      author: { name: 'Test Contributor', email: 'contributor@example.com' },
    },
  };
  const state = fixture([source]);
  assert.equal(await execute(state), 1);
  state.pr.body = body('Plan slot: FND-12');
  state.pr.head.ref = 'feat/fnd-12-next';
  for (const event of ['pull_request', 'merge_group'])
    await assert.rejects(execute(state, event), /approvers/);
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

test('PR and queue require linked current checkpoint approval and reread its identity', async () => {
  const signed = {
    sha: 'c'.repeat(40),
    commit: {
      message: 'Change\n\nSigned-off-by: Test Contributor <contributor@example.com>',
      author: { name: 'Test Contributor', email: 'contributor@example.com' },
    },
  };
  const state = fixture([signed]);
  state.pr.body = body('Plan slot: FND-12') + '\nCheckpoint evidence: #17\n';
  state.pr.head.ref = 'feat/fnd-12-next';
  const evidence = { checkpoint: 'CP1', source: base, artifacts: [], decisions: 'f'.repeat(64) };
  const issue = { state: 'open', body: JSON.stringify(evidence) };
  const comments = [
    {
      id: 20,
      user: { type: 'User', login: 'maintainer' },
      body: JSON.stringify({ ...evidence, status: 'approved' }),
    },
  ];
  const original = state.api;
  let issueReads = 0;
  let race = false;
  state.api = async (path, ...args) => {
    if (path === '/repos/' + repository + '/issues/3/comments?per_page=100&page=1') return [];
    if (path === '/repos/' + repository + '/issues/17') {
      issueReads++;
      return {
        ...issue,
        body:
          race && issueReads > 1
            ? JSON.stringify({ ...evidence, source: 'd'.repeat(40) })
            : issue.body,
      };
    }
    if (path === '/repos/' + repository + '/issues/17/comments?per_page=100&page=1')
      return structuredClone(comments);
    if (path === '/repos/' + repository + '/compare/' + base + '...' + base)
      return { status: 'identical' };
    return original(path, ...args);
  };
  const config = { checkpoints: { approvers: ['maintainer'], issues: { CP1: 17 } } };
  for (const event of ['pull_request', 'merge_group'])
    assert.equal(await execute(state, event, config), 1);
  state.pr.body = body('Plan slot: FND-12');
  await assert.rejects(execute(state, 'pull_request', config), /must link/);
  state.pr.body += '\nCheckpoint evidence: #17\n';
  comments[0].body = JSON.stringify({ ...evidence, status: 'revoked' });
  await assert.rejects(execute(state, 'merge_group', config), /missing or stale/);
  comments[0].body = JSON.stringify({ ...evidence, status: 'approved' });
  race = true;
  issueReads = 0;
  await assert.rejects(execute(state, 'merge_group', config), /evidence changed/);
});
