import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ATTESTATION,
  linkedIssues,
  queueMembers,
  readQueue,
  requireAttestation,
  resolveEvent,
  validatePullRequest,
} from './metadata.mjs';

const commit = (character) => character.repeat(40);
const base = commit('a');
const first = commit('b');
const head = commit('c');
const sourceOne = commit('d');
const sourceTwo = commit('e');
const repository = 'XTraceAI/xtrace-desktop';
const checked = `- [x] ${ATTESTATION}`;
const entry = (number, before, after, source) => ({
  baseCommit: { oid: before },
  headCommit: { oid: after },
  pullRequest: { number, headRefOid: source },
});

test('only a single visible checked attestation passes', () => {
  requireAttestation(`A reviewed description.\n\n${checked}`);
  for (const prefix of [
    '<img src="app.png" alt="App">',
    '<br>',
    '<https://github.com/XTraceAI/xtrace-desktop/issues/1>',
  ])
    requireAttestation(`${prefix}\n\n${checked}`);
  for (const body of [
    `- [ ] ${ATTESTATION}`,
    `\`\`\`markdown\n${checked}\n\`\`\``,
    `~~~\n${checked}\n~~~`,
    `\`\`\`\n\`\`\`still code\n${checked}\n\`\`\``,
    `<pre>\n${checked}\n</pre>`,
    `<div>\n${checked}`,
    `<table>\n${checked}`,
    `<details>\n\n${checked}`,
    `<details><summary>Review</summary>\n\n${checked}\n\n</details>`,
    `<?hide\n${checked}\n?>`,
    `<![CDATA[\n${checked}\n]]>`,
    `<!DOCTYPE\n${checked}\n>`,
    `- [<!-- spacer -->x] ${ATTESTATION}`,
    `<!--\n${checked}\n-->`,
    `> ${checked}`,
    `    ${checked}`,
    `${checked}\n- [ ] ${ATTESTATION}`,
    `${checked}\n${checked}`,
    'No disclosure review.',
  ])
    assert.throws(() => requireAttestation(body), /attestation/);
  requireAttestation(`<details>\n\nA collapsed example.\n\n</details>\n\n${checked}`);
});

test('code and HTML comments cannot invent issue references', () => {
  assert.deepEqual(
    linkedIssues(
      'Actual #7. `#123456`\n\n```css\ncolor: #987654\n```\n\n<!-- #654321 -->\n\n    #111111\n\n`other/project#123` and <!-- https://github.com/other/project/issues/4 -->',
      repository,
    ),
    [{ repository, number: 7 }],
  );
  assert.deepEqual(
    linkedIssues(
      'Actual [issue](https://github.com/XTraceAI/xtrace-desktop/issues/7).',
      repository,
    ),
    [{ repository, number: 7 }],
  );
});

test('rendered HTML issue anchors are included without reading comments or code examples', () => {
  const url = 'https://github.com/XTraceAI/xtrace-desktop/issues/7';
  for (const body of [
    `<a href="${url}">Issue</a>`,
    `<div><a href="${url}">Issue</a></div>`,
    '<a href="/XTraceAI/xtrace-desktop/issues/7">Issue</a>',
    '<a href="//github.com/XTraceAI/xtrace-desktop/%69ssues/7">Issue</a>',
    `<a href="${url.replace('issues', '&#105;ssues')}">Issue</a>`,
  ])
    assert.deepEqual(linkedIssues(body, repository), [{ repository, number: 7 }]);
  for (const body of [
    `<!-- <a href="${url}">Issue</a> -->`,
    `\`<a href="${url}">Issue</a>\``,
    `\`\`\`html\n<a href="${url}">Issue</a>\n\`\`\``,
    `    <a href="${url}">Issue</a>`,
    `<pre><a href="${url}">#7</a></pre>`,
    `<code><a href="${url}">#7</a></code>`,
    `<textarea><a href="${url}">#7</a></textarea>`,
    `<script><a href="${url}">#7</a></script>`,
    `<span title="${url}">Description</span>`,
    '<a href="#7">Section</a>',
    '[Section](#7)',
    '<a href="https://example.invalid/other/project#7">External page</a>',
    '<a href="https://github.com.evil.invalid/other/project/issues/7">External page</a>',
    '<a href="javascript:alert(7)">Example</a>',
    '<a href="http://[">Invalid destination</a>',
  ])
    assert.deepEqual(linkedIssues(body, repository), []);
});

test('queue resolution includes every constituent original head and excludes later entries', () => {
  const entries = [
    entry(3, head, commit('f'), commit('1')),
    entry(2, first, head, sourceTwo),
    entry(1, base, first, sourceOne),
  ];
  assert.deepEqual(queueMembers({ base_sha: base, head_sha: head }, entries), [
    { number: 1, head: sourceOne },
    { number: 2, head: sourceTwo },
  ]);
});

test('missing, ambiguous, cyclic, or stale queue metadata cannot become a green empty scan', () => {
  const group = { base_sha: base, head_sha: head };
  for (const entries of [
    [],
    [entry(2, first, head, sourceTwo)],
    [entry(1, base, head, sourceOne), entry(2, base, head, sourceTwo)],
    [entry(1, first, head, sourceOne), entry(2, head, first, sourceTwo)],
    [entry(1, base, head, null)],
    [entry(1, base, first, sourceOne), entry(1, first, head, sourceTwo)],
  ])
    assert.throws(() => queueMembers(group, entries));
});

test('queue pages must be complete and API errors must propagate', async () => {
  await assert.rejects(
    readQueue(async () => ({ errors: [{ message: 'private detail' }] }), repository, 'main'),
    /could not resolve/,
  );
  await assert.rejects(
    readQueue(
      async () => ({
        data: {
          repository: {
            mergeQueue: { entries: { totalCount: 2, nodes: [], pageInfo: { hasNextPage: false } } },
          },
        },
      }),
      repository,
      'main',
    ),
    /changed during pagination/,
  );
  const pages = [
    {
      totalCount: 2,
      nodes: [entry(1, base, first, sourceOne)],
      pageInfo: { hasNextPage: true, endCursor: 'cursor-1' },
    },
    {
      totalCount: 2,
      nodes: [entry(2, first, head, sourceTwo)],
      pageInfo: { hasNextPage: false, endCursor: null },
    },
  ];
  const result = await readQueue(
    async (_path, request) => {
      assert.equal(request.variables.after, pages.length === 2 ? null : 'cursor-1');
      return { data: { repository: { mergeQueue: { entries: pages.shift() } } } };
    },
    repository,
    'main',
  );
  assert.equal(result.length, 2);
});

test('PR checks use the actual current source head and attestation', () => {
  const pr = {
    number: 1,
    state: 'open',
    title: 'Scaffold',
    body: checked,
    base: { sha: base, ref: 'main', repo: { full_name: repository, private: false } },
    head: { sha: sourceOne, ref: 'feat/scaffold' },
  };
  assert.equal(validatePullRequest(pr, repository, sourceOne), pr);
  assert.throws(() => validatePullRequest(pr, repository, sourceTwo), /changed during this run/);
  assert.throws(
    () => validatePullRequest({ ...pr, body: '- [ ] ' + ATTESTATION }, repository, sourceOne),
    /attestation/,
  );
  assert.equal(
    validatePullRequest(
      { ...pr, base: { ...pr.base, repo: { full_name: repository, private: true } } },
      repository,
      sourceOne,
    ).number,
    1,
  );
  assert.throws(
    () =>
      validatePullRequest(
        { ...pr, base: { ...pr.base, repo: { full_name: 'other/repo' } } },
        repository,
        sourceOne,
      ),
    /this repository/,
  );
});

test('unsupported or mismatched event shapes fail before scanning', async () => {
  const event = {
    repository: { full_name: repository },
    pull_request: { number: 2, head: { sha: sourceOne }, base: { sha: base } },
  };
  assert.deepEqual(
    await resolveEvent('pull_request', event, repository, () => {
      throw new Error('Not needed');
    }),
    { base, head: sourceOne, members: [{ number: 2, head: sourceOne }] },
  );
  await assert.rejects(resolveEvent('push', event, repository), /Unsupported/);
  await assert.rejects(resolveEvent('pull_request', event, 'other/repo'), /does not match/);
  await assert.rejects(
    resolveEvent('pull_request', { ...event, pull_request: {} }, repository),
    /Missing/,
  );
});

test('explicit GitHub issue references are deduplicated without fetching arbitrary URLs', () => {
  const result = linkedIssues(
    'Closes #3, also (other/project#4). Details https://github.com/XTraceAI/xtrace-desktop/issues/3 and https://github.com/other/project/pull/4. External https://example.com/private.',
    repository,
  );
  assert.deepEqual(
    result.sort((left, right) => left.number - right.number),
    [
      { repository, number: 3 },
      { repository: 'other/project', number: 4 },
    ],
  );
  assert.throws(() => linkedIssues('#0', repository), /invalid pull request/);
  assert.deepEqual(
    linkedIssues(
      '/XTraceAI/xtrace-desktop/issues/3 http://github.com/XTraceAI/xtrace-desktop/issues/3 https://GitHub.com/XTraceAI/xtrace-desktop/issues/3',
      repository,
    ),
    [{ repository, number: 3 }],
  );
});

test('GitHub GH-number and hash autolinks include case variants and punctuation without URL/code lookalikes', () => {
  for (const body of [
    'GH-7',
    'gh-7',
    'Gh-7',
    '[GH-7]',
    '{GH-7}',
    'x-GH-7',
    '/GH-7',
    ':GH-7',
    'GH-07',
    '[#7]',
    '{#7}',
    'x-#7',
  ]) {
    assert.deepEqual(linkedIssues(body, repository), [{ repository, number: 7 }]);
  }
  assert.deepEqual(linkedIssues('GH-7 #7 gh-07', repository), [{ repository, number: 7 }]);
  for (const body of [
    'G`ignored`H-7',
    'G[ignored](https://example.invalid)H-7',
    'GH-`ignored`7',
    'xGH-7',
    'x_GH-7',
    'GH-7word',
    'GH-7_suffix',
    'x#7',
    'https://example.invalid/GH-7',
    'https://example.invalid/#7',
    'www.example.invalid/GH-7',
    '[GH-7](https://example.invalid)',
    '<a href="https://example.invalid">GH-7</a>',
    '`GH-7`',
    '```text\nGH-7\n```',
    '<!-- GH-7 -->',
    '<pre>GH-7</pre>',
  ])
    assert.deepEqual(linkedIssues(body, repository), []);
});
