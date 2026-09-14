import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { scanRepository } from '../security/scan-secrets.mjs';
import test from 'node:test';
import { closingIssues, readPublicContent, requireDisclosure } from './content.mjs';
import { ATTESTATION } from './metadata.mjs';
import { preparePublication, checkPublication } from './recheck.mjs';

export const repository = 'example/project';
export const base = 'a'.repeat(40);
export const head = 'b'.repeat(40);
const timestamp = '2026-01-01T00:00:00Z';
const comments = (items) =>
  structuredClone(items.map((item) => ({ updated_at: timestamp, ...item })));
export function fixture() {
  const state = {
    pr: {
      number: 3,
      commits: 1,
      state: 'open',
      title: 'Synthetic change',
      body: 'Reviewed description.\n\n- [x] ' + ATTESTATION + '\n\nDisclosure snapshot: pending\n',
      base: { sha: base, ref: 'main', repo: { full_name: repository } },
      head: { sha: head, ref: 'feat/synthetic' },
    },
    references: [],
    bodyEdits: [],
    titleEdits: [],
    comments: [],
    reviews: [],
    reviewEdits: {},
    headEvents: [],
    retainedCommits: {},
    reviewComments: [],
    commitComments: [],
    commits: [{ sha: head }],
    queue: [],
    issue: {
      number: 9,
      title: 'Synthetic issue',
      body: 'Public technical acceptance.',
      updated_at: timestamp,
    },
    issueComments: [],
    calls: [],
    writes: [],
  };
  state.api = async (path, body, method) => {
    state.calls.push(path);
    if (path === '/graphql') {
      if (body.query.includes('query HeadRevisions'))
        return {
          data: {
            repository: {
              pullRequest: {
                timelineItems: {
                  nodes: structuredClone(state.headEvents),
                  pageInfo: { hasNextPage: false },
                },
              },
            },
          },
        };
      if (body.query.includes('query RetainedHeadCommits')) {
        const head = body.variables.head;
        const nodes = (state.retainedCommits[head] ?? [head]).map((oid) => ({ oid }));
        return {
          data: {
            repository: {
              object: {
                oid: head,
                history: {
                  totalCount: nodes.length,
                  nodes,
                  pageInfo: { hasNextPage: false },
                },
              },
            },
          },
        };
      }
      if (body.query.includes('query ReviewRevisions')) {
        const nodes = state.reviews.map((review) => {
          const edits = state.reviewEdits[review.id] ?? [];
          return {
            id: 'review-' + review.id,
            fullDatabaseId: String(review.id),
            body: review.body,
            state: review.state,
            updatedAt: review.updated_at ?? timestamp,
            lastEditedAt: edits[0]?.editedAt ?? null,
            userContentEdits: {
              totalCount: edits.length,
              nodes: structuredClone(edits),
              pageInfo: { hasNextPage: false },
            },
          };
        });
        return {
          data: {
            repository: {
              pullRequest: {
                reviews: { totalCount: nodes.length, nodes, pageInfo: { hasNextPage: false } },
              },
            },
          },
        };
      }
      if (body.query.includes('query SourceRevisions')) {
        const field = body.query.includes('userContentEdits(')
          ? 'userContentEdits'
          : 'timelineItems';
        const nodes = field === 'userContentEdits' ? state.bodyEdits : state.titleEdits;
        return {
          data: {
            repository: {
              pullRequest: {
                body: state.pr.body,
                title: state.pr.title,
                lastEditedAt: state.bodyEdits[0]?.editedAt ?? null,
                [field]: {
                  totalCount: nodes.length,
                  nodes: structuredClone(nodes),
                  pageInfo: { hasNextPage: false, endCursor: null },
                },
              },
            },
          },
        };
      }
      if (body.variables.branch)
        return {
          data: {
            repository: {
              mergeQueue: state.queue.length
                ? {
                    entries: {
                      nodes: state.queue,
                      totalCount: state.queue.length,
                      pageInfo: { hasNextPage: false },
                    },
                  }
                : null,
            },
          },
        };
      return {
        data: {
          repository: {
            pullRequest: {
              closingIssuesReferences: {
                nodes: state.references,
                totalCount: state.references.length,
                pageInfo: { hasNextPage: false },
              },
            },
          },
        },
      };
    }
    if (path.includes('/check-runs/') && !body) {
      const id = Number(path.split('/').at(-1));
      return structuredClone(state.writes[id - 1].body);
    }
    if (path.includes('/check-runs')) {
      state.writes.push({ path, body: structuredClone(body), method });
      return { id: state.writes.length };
    }
    const route = path.split('?')[0];
    const root = '/repos/' + repository;
    if (route === root) return { default_branch: 'main' };
    if (route === root + '/commits/main') return { sha: base };
    if (route === root + '/pulls') return [structuredClone(state.pr)];
    if (route === root + '/pulls/3') return structuredClone(state.pr);
    if (route === root + '/issues/3/comments') return comments(state.comments);
    if (route === root + '/pulls/3/reviews')
      return structuredClone(state.reviews.map((item) => ({ submitted_at: timestamp, ...item })));
    if (route === root + '/pulls/3/comments') return comments(state.reviewComments);
    if (route === root + '/pulls/3/commits') return structuredClone(state.commits);
    if (route === root + '/comments') return comments(state.commitComments);
    if (route === root + '/issues/9') return structuredClone(state.issue);
    if (route === root + '/issues/9/comments') return comments(state.issueComments);
    throw new Error('Unexpected synthetic API route');
  };
  return state;
}

export async function seal(state) {
  const review = await readPublicContent(state.api, repository, 3);
  state.pr.body = state.pr.body.replace(
    /Disclosure snapshot: (?:pending|[a-f0-9]{64})/,
    'Disclosure snapshot: ' + review.digest,
  );
  return review.digest;
}

test('manual Development relationships are included without a body reference', async () => {
  const state = fixture();
  state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
  await seal(state);
  const review = await readPublicContent(state.api, repository, 3);
  requireDisclosure(review);
  assert.equal(review.content.linked.length, 1);
  assert.ok(review.texts.includes(state.issue.body));
  state.issue.body += ' Edited.';
  assert.throws(() => requireDisclosure({ ...review, digest: 'c'.repeat(64) }), /stale/);
  await assert.rejects(
    async () => requireDisclosure(await readPublicContent(state.api, repository, 3)),
    /stale/,
  );
});

test('linked issue and PR disclosure controls are hashed verbatim', async () => {
  for (const kind of ['issue', 'pull-request']) {
    for (const change of ['checkbox', 'snapshot']) {
      const state = fixture();
      state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
      state.issue.body = '- [ ] ' + ATTESTATION + '\n\nDisclosure snapshot: pending\n';
      if (kind === 'pull-request') {
        state.issue.pull_request = {};
        const api = state.api;
        state.api = async (...args) => {
          const route = args[0].split('?')[0];
          const pull = '/repos/' + repository + '/pulls/9';
          if (route === pull) return { commits: 1 };
          if (route === pull + '/commits') return [{ sha: base }];
          if ([pull + '/reviews', pull + '/comments'].includes(route)) return [];
          return api(...args);
        };
      }
      await seal(state);
      state.issue.body =
        change === 'checkbox'
          ? state.issue.body.replace('- [ ]', '- [x]')
          : state.issue.body.replace('pending', 'c'.repeat(64));
      const current = await readPublicContent(state.api, repository, 3);
      assert.equal(current.content.linked[0].body, state.issue.body);
      assert.throws(() => requireDisclosure(current), /stale/);
      assert.equal((await reconcile(state, async () => {}, 'workflow_dispatch')).failures, 0);
    }
  }
});

test('comment additions, edits, deletions, attachments and review changes stale the unchanged checkbox', async () => {
  for (const mutate of [
    (s) => s.comments.push({ id: 1, body: 'New comment.' }),
    (s) => {
      s.comments[0].body = 'Edited comment.';
    },
    (s) => {
      s.comments = [];
    },
    (s) =>
      s.reviewComments.push({
        id: 2,
        body: '![Synthetic image](https://example.invalid/image.png)',
        diff_hunk: '@@ -1 +1 @@\n+Synthetic context.',
      }),
    (s) => s.reviews.push({ id: 3, body: 'Review text.', state: 'COMMENTED' }),
    (s) => {
      s.pr.head.sha = 'c'.repeat(40);
    },
    (s) => {
      s.references = [{ number: 9, repository: { nameWithOwner: repository } }];
    },
  ]) {
    const state = fixture();
    state.comments = [{ id: 8, body: 'Existing comment.' }];
    await seal(state);
    mutate(state);
    await assert.rejects(
      async () => requireDisclosure(await readPublicContent(state.api, repository, 3)),
      /stale/,
    );
  }
});

test('preparing a candidate does not change public content and snapshot controls do not hash themselves', async () => {
  const state = fixture();
  state.pr.body = state.pr.body.replace('- [x]', '- [ ]');
  const digest = await seal(state);
  state.pr.body = state.pr.body.replace('- [ ]', '- [x]');
  assert.equal((await readPublicContent(state.api, repository, 3)).digest, digest);
  const original = state.pr.body;
  await assert.rejects(
    readPublicContent(state.api, repository, 3, original.replace('description', 'candidate')),
    /Publish the reviewed PR prose/,
  );
  assert.equal(
    (await readPublicContent(state.api, repository, 3, original.replace('- [x]', '- [ ]'))).digest,
    digest,
  );
  assert.equal(state.pr.body, original);
  assert.equal(state.writes.length, 0);
});

test('linked API errors, incomplete pagination and unsupported external issue references fail closed', async () => {
  await assert.rejects(
    closingIssues(
      async () => ({ errors: [{ message: 'synthetic-private-error' }] }),
      repository,
      3,
    ),
    /could not resolve/,
  );
  await assert.rejects(
    closingIssues(
      async () => ({
        data: {
          repository: {
            pullRequest: {
              closingIssuesReferences: {
                nodes: [],
                totalCount: 1,
                pageInfo: { hasNextPage: false },
              },
            },
          },
        },
      }),
      repository,
      3,
    ),
    /pagination/,
  );
  const state = fixture();
  state.pr.body += '\nother/project#9';
  await assert.rejects(readPublicContent(state.api, repository, 3), /Cross-repository/);
});

const event = { repository: { full_name: repository } };
const reconcile = async (
  state,
  scan = async () => {},
  eventName = 'workflow_dispatch',
  payload = event,
) => {
  const options = {
    api: state.api,
    repository,
    repo: 'unused-synthetic-repo',
    runId: 1,
    trustedHead: base,
  };
  const plan = await preparePublication({ ...options, eventName, event: payload });
  const results = await Promise.all(
    plan.pending.map((member) => checkPublication({ ...options, member, scan })),
  );
  return {
    checked: results.length + plan.queue.length,
    failures: results.reduce((sum, result) => sum + result.failures, plan.queue.length),
  };
};

test('automated disclosure output is advisory and cannot claim merge authorization', async () => {
  const state = fixture();
  await seal(state);
  assert.equal((await reconcile(state)).failures, 0);
  assert.equal(state.writes[0].body.name, 'publication-content-advisory');
  assert.match(state.writes.at(-1).body.output.title, /Advisory/);
  assert.match(state.writes.at(-1).body.output.summary, /not merge authorization/);
  assert.match(state.writes.at(-1).body.output.summary, /review before publication/);
});

test('source commit comments and attachments invalidate snapshots on creation, edit and deletion', async () => {
  for (const change of ['create', 'edit', 'delete']) {
    const state = fixture();
    state.commitComments = [{ id: 80, commit_id: head, body: 'Original synthetic comment.' }];
    await seal(state);
    if (change === 'create')
      state.commitComments.push({
        id: 81,
        commit_id: head,
        body: '![Attachment](https://example.invalid/synthetic.png)',
      });
    if (change === 'edit') state.commitComments[0].body = 'Edited synthetic comment.';
    if (change === 'delete') state.commitComments = [];
    const review = await readPublicContent(state.api, repository, 3);
    assert.throws(() => requireDisclosure(review), /stale/);
    assert.equal(
      (
        await reconcile(
          state,
          async (_, texts) => {
            for (const comment of state.commitComments) assert.ok(texts.includes(comment.body));
          },
          'workflow_dispatch',
          event,
        )
      ).failures,
      0,
    );
  }
});

test('commit pagination fails closed and unrelated commit comments do not stale a PR', async () => {
  const state = fixture();
  const digest = await seal(state);
  state.commitComments.push({
    id: 90,
    commit_id: 'd'.repeat(40),
    body: 'Unrelated synthetic comment.',
  });
  assert.equal((await readPublicContent(state.api, repository, 3)).digest, digest);
  state.pr.commits = 251;
  await assert.rejects(readPublicContent(state.api, repository, 3), /commit pagination/);
  state.pr.commits = 1;
  state.commits.push(state.commits[0]);
  await assert.rejects(readPublicContent(state.api, repository, 3), /commit pagination/);
});

test('the advisory scans the current linked issue text without a prior snapshot', async () => {
  const state = fixture();
  state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
  await seal(state);
  state.issue.body += ' Changed acceptance.';
  let scans = 0;
  assert.equal(
    (
      await reconcile(
        state,
        async () => {
          scans++;
        },
        'workflow_dispatch',
      )
    ).failures,
    0,
  );
  assert.equal(scans, 1);
  assert.equal(state.writes[0].body.status, 'completed');
  assert.equal(state.writes[1].body.conclusion, 'success');
  assert.ok(state.writes.every((write) => write.path.includes('/check-runs')));
});

test('routine advisory scanning does not require per-PR attestation controls', async () => {
  const state = fixture();
  state.pr.body = '## Change\n\nSynthetic change.\n\n## Verification\n\nPassed.';
  assert.equal((await reconcile(state)).failures, 0);
  assert.equal(state.writes.at(-1).body.conclusion, 'success');
});

test('content and source base/head races cannot overwrite a pending check with success', async () => {
  for (const mutate of [
    (s) => {
      s.pr.head.sha = 'c'.repeat(40);
    },
    (s) => {
      s.pr.base.sha = 'c'.repeat(40);
    },
    (s) => {
      s.comments.push({ id: 12, body: 'Changed during scan.' });
    },
  ]) {
    const state = fixture();
    await seal(state);
    const result = await reconcile(state, async () => mutate(state));
    assert.equal(result.failures, 1);
    assert.equal(state.writes.at(-1).body.conclusion, 'failure');
  }
  const state = fixture();
  await seal(state);
  const api = state.api;
  let reads = 0;
  state.api = async (...args) => {
    if (args[0] === '/repos/' + repository + '/pulls/3' && ++reads === 3)
      state.pr.head.sha = 'c'.repeat(40);
    return api(...args);
  };
  assert.equal((await reconcile(state)).failures, 1);
  assert.equal(state.writes.at(-1).body.conclusion, 'failure');
});

test('a base advance immediately before certification remains blocking', async () => {
  const state = fixture();
  await seal(state);
  const api = state.api;
  let reads = 0;
  state.api = async (...args) => {
    if (args[0] === '/repos/' + repository + '/pulls/3' && ++reads === 5)
      state.pr.base.sha = 'c'.repeat(40);
    return api(...args);
  };
  assert.equal((await reconcile(state)).failures, 1);
  assert.equal(reads, 6);
  assert.equal(state.writes.at(-1).body.conclusion, 'failure');
});

test('final certification refreshes all disclosure fields after trusted-code lookup', async (t) => {
  const mutations = {
    title: (s) => (s.pr.title += ' Updated.'),
    body: (s) => (s.pr.body += '\nUpdated prose.'),
    comment: (s) => s.comments.push({ id: 20, body: 'Late conversation.' }),
    review: (s) => s.reviews.push({ id: 21, body: 'Late review.', state: 'COMMENTED' }),
    'inline comment': (s) =>
      s.reviewComments.push({ id: 22, body: 'Late inline.', diff_hunk: '@@ Synthetic context' }),
    'commit comment': (s) =>
      s.commitComments.push({ id: 23, body: 'Late commit comment.', commit_id: head }),
    'linked title': (s) => (s.issue.title += ' Updated.'),
    'linked body': (s) => (s.issue.body += ' Updated.'),
    'linked comment': (s) => s.issueComments.push({ id: 24, body: 'Late linked comment.' }),
    relationship: (s) => (s.references = []),
    'title revision': (s) =>
      s.titleEdits.push({
        id: 'late-title',
        createdAt: timestamp,
        previousTitle: 'Retained earlier title',
        currentTitle: s.pr.title,
      }),
    'source head': (s) => (s.pr.head.sha = 'c'.repeat(40)),
    'source base': (s) => (s.pr.base.sha = 'c'.repeat(40)),
    closed: (s) => (s.pr.state = 'closed'),
  };
  for (const [name, mutate] of Object.entries(mutations)) {
    await t.test(name, async () => {
      const state = fixture();
      state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
      await seal(state);
      const api = state.api;
      let mutated = false;
      state.api = async (...args) => {
        const response = await api(...args);
        if (args[0] === '/repos/' + repository + '/commits/main') {
          mutate(state);
          // Even a newly matching attestation cannot certify unscanned content.
          if (name === 'comment') await seal(state);
          mutated = true;
        }
        return response;
      };
      assert.equal((await reconcile(state)).failures, 1);
      assert.equal(mutated, true);
      assert.equal(state.writes.at(-1).body.conclusion, 'failure');
    });
  }
});

test('raw API or scanner errors cannot become public diagnostics or successful checks', async () => {
  const state = fixture();
  await seal(state);
  assert.equal(
    (
      await reconcile(state, async () => {
        throw new Error('synthetic-private-diagnostic');
      })
    ).failures,
    1,
  );
  assert.doesNotMatch(JSON.stringify(state.writes), /synthetic-private-diagnostic/);
  assert.equal(state.writes.at(-1).body.conclusion, 'failure');
  const api = state.api;
  state.api = async (...args) => {
    if (args[0].includes('/pulls?state=open')) return Array.from({ length: 100 }, () => state.pr);
    return api(...args);
  };
  await assert.rejects(reconcile(state), /pagination/);
});

test('manual audits reject automatic events and another repository before API access', async () => {
  for (const eventName of [
    'issues',
    'issue_comment',
    'schedule',
    'workflow_run',
    'pull_request_review',
    'pull_request_target',
  ]) {
    const state = fixture();
    state.api = async () => assert.fail('Automatic events must not read metadata');
    await assert.rejects(reconcile(state, undefined, eventName), /explicit workflow dispatch/);
    assert.equal(state.writes.length, 0);
  }
  await assert.rejects(
    reconcile(fixture(), undefined, 'workflow_dispatch', {
      repository: { full_name: 'other/project' },
    }),
    /explicit workflow dispatch/,
  );
});

test('queue certification remains failed until trusted combined-tree scanning is activated', async () => {
  const state = fixture();
  state.queue = [
    {
      baseCommit: { oid: base },
      headCommit: { oid: 'c'.repeat(40) },
      pullRequest: { number: 3, headRefOid: head },
    },
  ];
  await seal(state);
  const result = await reconcile(state);
  assert.equal(result.failures, 1);
  assert.equal(state.writes[0].body.conclusion, 'failure');
  assert.equal(state.writes.filter((write) => write.path.endsWith('/check-runs/1')).length, 0);
});

test('queue heads have terminal failures before scans and remain blocked when later reads fail', async () => {
  for (const failAt of ['scan', 'pull-list']) {
    const state = fixture();
    const queueHead = 'c'.repeat(40);
    state.queue = [
      {
        baseCommit: { oid: base },
        headCommit: { oid: queueHead },
        pullRequest: { number: 3, headRefOid: head },
      },
    ];
    await seal(state);
    const assertQueuePending = () => {
      assert.equal(state.writes[0].body.head_sha, queueHead);
      assert.equal(state.writes[0].body.status, 'completed');
      assert.ok(!state.writes.some(({ body }) => body.conclusion === 'success'));
    };
    if (failAt === 'pull-list') {
      const api = state.api;
      state.api = async (...args) => {
        if (args[0].includes('/pulls?state=open')) {
          assertQueuePending();
          throw new Error('Synthetic unavailable PR metadata');
        }
        return api(...args);
      };
      await assert.rejects(reconcile(state), /unavailable PR metadata/);
      assert.equal(state.writes.length, 1, 'The queue must retain its blocking pending check.');
    } else {
      const result = await reconcile(state, async () => {
        assertQueuePending();
        throw new Error('Synthetic scan failure');
      });
      assert.equal(result.failures, 2);
      assert.equal(state.writes[0].body.conclusion, 'failure');
      assert.equal(state.writes.at(-1).body.conclusion, 'failure');
    }
  }
});

test('source and linked PR review diff context is required, scanned and included in snapshots', async () => {
  for (const linked of [false, true]) {
    const state = fixture();
    const comment = {
      id: 90,
      body: 'Reviewed context.',
      diff_hunk: '@@ -1 +1 @@\n+Original context.',
    };
    if (linked) {
      state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
      state.issue.pull_request = {};
      const api = state.api;
      state.api = async (...args) => {
        const route = args[0].split('?')[0];
        const pull = '/repos/' + repository + '/pulls/9';
        if (route === pull) return { commits: 1 };
        if (route === pull + '/commits') return [{ sha: base }];
        if (route === pull + '/reviews') return [];
        if (route === pull + '/comments') return comments([comment]);
        return api(...args);
      };
    } else state.reviewComments = [comment];
    await seal(state);
    let review = await readPublicContent(state.api, repository, 3);
    assert.ok(review.texts.includes(comment.diff_hunk));
    comment.diff_hunk += '\n+Changed context only.';
    review = await readPublicContent(state.api, repository, 3);
    assert.throws(() => requireDisclosure(review), /stale/);
    comment.diff_hunk = '';
    await seal(state);
    requireDisclosure(await readPublicContent(state.api, repository, 3));
    for (const invalid of [null, 0, {}, []]) {
      comment.diff_hunk = invalid;
      await assert.rejects(
        readPublicContent(state.api, repository, 3),
        /diff context is unavailable/,
      );
    }
    delete comment.diff_hunk;
    await assert.rejects(
      readPublicContent(state.api, repository, 3),
      /diff context is unavailable/,
    );
  }
});

test('a secret only in an outdated review hunk is detected from collected public text', async (t) => {
  const state = fixture();
  const secret = ['gh', 'p_'].join('') + randomBytes(18).toString('hex');
  state.reviewComments = [
    {
      id: 90,
      body: 'Reviewed context.',
      commit_id: 'c'.repeat(40),
      diff_hunk: '@@ -1 +1 @@\n+token = "' + secret + '"',
    },
  ];
  const review = await readPublicContent(state.api, repository, 3);
  assert.ok(!state.commits.some(({ sha }) => sha === state.reviewComments[0].commit_id));
  const temporary = await mkdtemp(join(tmpdir(), 'publication-hunk-'));
  t.after(() => rm(temporary, { recursive: true, force: true }));
  const repo = join(temporary, 'repo');
  await mkdir(repo);
  const git = (...args) =>
    execFileSync('git', ['-c', 'core.hooksPath=/dev/null', ...args], {
      cwd: repo,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  git('init', '-q', '-b', 'main');
  git('config', 'user.name', 'Synthetic Contributor');
  git('config', 'user.email', 'contributor@example.invalid');
  await writeFile(join(repo, 'README.md'), 'Clean current source.\n');
  git('add', '.');
  git('commit', '-qm', 'Synthetic clean source');
  assert.deepEqual(await scanRepository({ repo, diffs: [], content: [] }), []);
  const content = join(temporary, 'public-text.md');
  await writeFile(content, review.texts.join('\n\n'), { mode: 0o600 });
  assert.ok(
    (await scanRepository({ repo, diffs: [], content: [content] })).length > 0,
    'Collected outdated diff context must detect a credential absent from current Git source.',
  );
});

test('linked issue and PR edit-and-revert revisions invalidate the old disclosure', async () => {
  for (const linkedPr of [false, true]) {
    const state = fixture();
    state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
    if (linkedPr) {
      state.issue.pull_request = {};
      const api = state.api;
      state.api = async (...args) => {
        const route = args[0].split('?')[0];
        const pull = '/repos/' + repository + '/pulls/9';
        if (route === pull) return { commits: 1 };
        if (route === pull + '/commits') return [{ sha: base }];
        if ([pull + '/reviews', pull + '/comments'].includes(route)) return [];
        return api(...args);
      };
    }
    await seal(state);
    const original = state.issue.body;
    state.issue.body = 'Unreviewed intermediate text.';
    state.issue.updated_at = '2026-01-01T00:00:01Z';
    state.issue.body = original;
    state.issue.updated_at = '2026-01-01T00:00:02Z';
    // Read the final reverted state, not the intermediate edit.
    await assert.rejects(
      async () => requireDisclosure(await readPublicContent(state.api, repository, 3)),
      /stale/,
    );
    await seal(state);
    requireDisclosure(await readPublicContent(state.api, repository, 3));
    for (const invalid of [undefined, null, '', 0, 'yesterday', '2026-02-30T00:00:00Z']) {
      state.issue.updated_at = invalid;
      await assert.rejects(readPublicContent(state.api, repository, 3), /revision timestamp/);
    }
  }
});

test('discussion revision timestamps are required and detect reverted text when the API advances them', async () => {
  for (const kind of ['comments', 'reviewComments', 'commitComments']) {
    const state = fixture();
    const comment = {
      id: 90,
      body: 'Reviewed text.',
      commit_id: head,
      diff_hunk: '',
      updated_at: timestamp,
    };
    state[kind] = [comment];
    await seal(state);
    comment.body = 'Unreviewed intermediate text.';
    comment.updated_at = '2026-01-01T00:00:01Z';
    comment.body = 'Reviewed text.';
    comment.updated_at = '2026-01-01T00:00:02Z';
    await assert.rejects(
      async () => requireDisclosure(await readPublicContent(state.api, repository, 3)),
      /stale/,
    );
    await seal(state);
    requireDisclosure(await readPublicContent(state.api, repository, 3));
    comment.updated_at = 'invalid';
    await assert.rejects(readPublicContent(state.api, repository, 3), /revision timestamp/);
    comment.updated_at = undefined;
    await assert.rejects(readPublicContent(state.api, repository, 3), /revision timestamp/);
  }
});

test('every scheduled check is already terminal when work is cancelled or metadata fails', async () => {
  const state = fixture();
  await seal(state);
  const plan = await preparePublication({
    api: state.api,
    repository,
    runId: 1,
    trustedHead: base,
    eventName: 'workflow_dispatch',
    event,
  });
  assert.equal(plan.pending.length, 1);
  assert.equal(state.writes[0].body.status, 'completed');
  assert.equal(state.writes[0].body.conclusion, 'failure');
  // Never starting the worker is already a terminal failure. A failing final
  // write must preserve that result rather than leaving an in-progress check.
  const api = async (...args) => {
    if (args[2] === 'PATCH') throw new Error('Synthetic API outage');
    return state.api(...args);
  };
  await assert.rejects(
    checkPublication({
      api,
      repository,
      repo: 'unused',
      runId: 1,
      trustedHead: base,
      member: plan.pending[0],
      scan: async () => {},
    }),
    /API outage/,
  );
  assert.equal(state.writes.length, 1);
});

test('a scan cannot update a check from another run, head or context', async () => {
  for (const change of [
    (check) => {
      check.external_id = 'publication-content-advisory:2';
    },
    (check) => {
      check.head_sha = 'd'.repeat(40);
    },
    (check) => {
      check.name = 'another-context';
    },
    (check) => {
      check.conclusion = 'success';
    },
  ]) {
    const state = fixture();
    await seal(state);
    const plan = await preparePublication({
      api: state.api,
      repository,
      runId: 1,
      trustedHead: base,
      eventName: 'workflow_dispatch',
      event,
    });
    change(state.writes[0].body);
    await assert.rejects(
      checkPublication({
        api: state.api,
        repository,
        repo: 'unused',
        runId: 1,
        trustedHead: base,
        member: plan.pending[0],
        scan: async () => assert.fail('must not scan'),
      }),
      /does not belong/,
    );
    assert.equal(state.writes.length, 1);
  }
});

test('matrix overflow leaves every enumerated PR failed instead of truncating work', async () => {
  const state = fixture();
  const api = async (...args) => {
    if (args[0].includes('/pulls?state=open')) {
      const page = Number(new URL('https://example.invalid' + args[0]).searchParams.get('page'));
      return Array.from({ length: Math.min(100, 257 - (page - 1) * 100) }, (_, index) => ({
        ...state.pr,
        number: (page - 1) * 100 + index + 1,
      }));
    }
    return state.api(...args);
  };
  await assert.rejects(
    preparePublication({ api, repository, runId: 1, eventName: 'workflow_dispatch', event }),
    /256/,
  );
  assert.equal(state.writes.length, 257);
  assert.ok(
    state.writes.every(
      (write) => write.body.status === 'completed' && write.body.conclusion === 'failure',
    ),
  );
});

test('a trusted default-branch advance during scanning cannot certify old policy', async () => {
  const state = fixture();
  await seal(state);
  const api = state.api;
  let advanced = false;
  state.api = async (...args) => {
    if (advanced && args[0] === '/repos/' + repository + '/commits/main')
      return { sha: 'd'.repeat(40) };
    return api(...args);
  };
  const result = await reconcile(state, async () => {
    advanced = true;
  });
  assert.equal(result.failures, 1);
  assert.equal(state.writes.at(-1).body.conclusion, 'failure');
  assert.match(state.writes.at(-1).body.output.summary, /Trusted default-branch code changed/);
});

test('source body edit-and-revert stales the attestation but control-only edits do not', async () => {
  const state = fixture();
  const original = state.pr.body;
  const publish = (body) => {
    state.pr.body = body;
    state.bodyEdits.unshift({
      id: 'body-' + state.bodyEdits.length,
      editedAt: timestamp,
      deletedAt: null,
      diff: body,
    });
  };
  const digest = await seal(state);
  // GitHub materializes the original revision on the first body edit.
  const sealed = state.pr.body;
  publish(original);
  publish(sealed);
  requireDisclosure(await readPublicContent(state.api, repository, 3));
  publish(sealed.replace('Reviewed description.', 'Intervening description.'));
  publish(sealed);
  const changed = await readPublicContent(state.api, repository, 3);
  assert.notEqual(changed.digest, digest);
  assert.throws(() => requireDisclosure(changed), /stale/);
  assert.ok(changed.texts.some((text) => text.includes('Intervening description.')));
  publish(sealed.replace(digest, changed.digest));
  requireDisclosure(await readPublicContent(state.api, repository, 3));
});

test('source title edit-and-revert stales the attestation and includes retained titles', async () => {
  const state = fixture();
  await seal(state);
  const title = state.pr.title;
  state.titleEdits = [
    { id: 'title-1', createdAt: timestamp, previousTitle: title, currentTitle: 'Temporary title' },
    { id: 'title-2', createdAt: timestamp, previousTitle: 'Temporary title', currentTitle: title },
  ];
  const changed = await readPublicContent(state.api, repository, 3);
  assert.throws(() => requireDisclosure(changed), /stale/);
  assert.ok(changed.texts.includes('Temporary title'));
});

test('GH-number references without closing relationships include issue text and invalidate on edits', async () => {
  const state = fixture();
  state.pr.body = 'Related GH-9.\n\n' + state.pr.body;
  await seal(state);
  const review = await readPublicContent(state.api, repository, 3);
  assert.deepEqual(state.references, []);
  assert.equal(review.content.linked[0].key, repository + '#9');
  assert.ok(review.texts.includes(state.issue.body));
  state.issue.body = 'Changed linked issue text.';
  state.issue.updated_at = '2026-01-01T00:00:01Z';
  assert.equal(
    (
      await reconcile(
        state,
        async () => assert.fail('Stale content must not scan'),
        'workflow_dispatch',
      )
    ).failures,
    1,
  );
});

test('source and linked review edit-and-revert retains history despite identical timestamps', async (t) => {
  for (const linked of [false, true]) {
    await t.test(linked ? 'linked review' : 'source review', async () => {
      const state = fixture();
      const review = {
        id: 91,
        body: 'Reviewed text.',
        state: 'COMMENTED',
        submitted_at: timestamp,
      };
      state.reviews = [review];
      if (linked) {
        state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
        state.issue.pull_request = {};
        const api = state.api;
        state.api = async (...args) => {
          const route = args[0].split('?')[0];
          const pull = '/repos/' + repository + '/pulls/9';
          if (route === pull) return { commits: 1 };
          if (route === pull + '/commits') return [{ sha: base }];
          if (route === pull + '/comments') return [];
          if (route === pull + '/reviews') return structuredClone(state.reviews);
          if (route === '/repos/' + repository + '/pulls/3/reviews') return [];
          return api(...args);
        };
      }
      const original = await seal(state);
      state.reviewEdits[91] = [
        { id: 'restored', editedAt: timestamp, deletedAt: null, diff: review.body },
        {
          id: 'intermediate',
          editedAt: timestamp,
          deletedAt: null,
          diff: 'Retained private draft.',
        },
        { id: 'initial', editedAt: timestamp, deletedAt: null, diff: review.body },
      ];
      const changed = await readPublicContent(state.api, repository, 3);
      assert.notEqual(changed.digest, original);
      assert.throws(() => requireDisclosure(changed), /stale/);
      assert.ok(changed.texts.includes('Retained private draft.'));
      const records = linked ? changed.content.linked[0].reviews : changed.content.reviews;
      assert.equal(records[0].revisions.edits.length, 3);
      await seal(state);
      requireDisclosure(await readPublicContent(state.api, repository, 3));
      state.reviewEdits[91][1].deletedAt = timestamp;
      state.reviewEdits[91][1].diff = null;
      const deleted = await readPublicContent(state.api, repository, 3);
      assert.throws(() => requireDisclosure(deleted), /stale/);
      assert.ok(!deleted.texts.includes('Retained private draft.'));
    });
  }
});

test('force-pushed commit comments and attachments remain in source and linked disclosure', async () => {
  for (const linked of [false, true]) {
    const state = fixture();
    const former = 'c'.repeat(40);
    const ancestor = 'd'.repeat(40);
    state.headEvents = [
      {
        id: 'force-1',
        createdAt: timestamp,
        beforeCommit: { oid: former },
        afterCommit: { oid: head },
      },
    ];
    state.retainedCommits[former] = [former, ancestor];
    state.commitComments = [
      {
        id: 97,
        commit_id: ancestor,
        body: '![Retained attachment](https://example.invalid/old.png)',
      },
    ];
    if (linked) {
      state.references = [{ number: 9, repository: { nameWithOwner: repository } }];
      state.issue.pull_request = {};
      const api = state.api;
      state.api = async (...args) => {
        const route = args[0].split('?')[0];
        const pull = '/repos/' + repository + '/pulls/9';
        if (route === pull) return { commits: 1 };
        if (route === pull + '/commits') return [{ sha: base }];
        if ([pull + '/reviews', pull + '/comments'].includes(route)) return [];
        if (
          args[0] === '/graphql' &&
          args[1].query.includes('query HeadRevisions') &&
          args[1].variables.number === 3
        )
          return {
            data: {
              repository: {
                pullRequest: {
                  timelineItems: {
                    nodes: [],
                    pageInfo: { hasNextPage: false },
                  },
                },
              },
            },
          };
        return api(...args);
      };
    }
    await seal(state);
    const review = await readPublicContent(state.api, repository, 3);
    assert.ok(review.texts.includes(state.commitComments[0].body));
    const content = linked ? review.content.linked[0] : review.content;
    assert.ok(content.headRevisions.commits.includes(ancestor));
    assert.equal(content.commitComments[0].id, 97);
    state.commitComments[0].body += ' Edited.';
    const changed = await readPublicContent(state.api, repository, 3);
    assert.throws(() => requireDisclosure(changed), /stale/);
  }
});
