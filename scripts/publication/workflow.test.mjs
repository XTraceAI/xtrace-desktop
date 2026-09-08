import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const workflow = (name) =>
  readFile(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8');

test('candidate publication jobs have no metadata grants or credentials for runtime code', async () => {
  const source = await workflow('publication');
  assert.doesNotMatch(source, /\b(?:GITHUB_TOKEN|GH_TOKEN)\b/i);
  assert.doesNotMatch(source, /\$\{\{[^}]*\b(?:token|secrets)\b/i);
  assert.doesNotMatch(source, /^\s+(?:issues|pull-requests|checks|actions):/m);
  assert.match(source, /^permissions:\n {2}contents: read\n/m);
  assert.deepEqual(source.match(/^ *permissions:.*$/gm), ['permissions:', '    permissions: {}']);
  const checkouts = source.split(/- uses: actions\/checkout@/).slice(1);
  assert.equal(checkouts.length, 2);
  for (const checkout of checkouts) {
    const settings = checkout.split(/\n\s+- (?:uses|name):/)[0];
    assert.match(settings, /persist-credentials: false/);
  }
  assert.doesNotMatch(source, /run:.*scripts\/publication\/(?:check|recheck)/);
  assert.match(
    source,
    /PUBLICATION_BASE: \$\{\{ github\.event\.pull_request\.base\.sha \|\| github\.event\.merge_group\.base_sha \}\}/,
  );
  assert.match(source, /PUBLICATION_HEAD: \$\{\{ github\.sha \}\}/);
  assert.match(
    source,
    /run: node scripts\/security\/scan-secrets\.mjs --diff "\$PUBLICATION_BASE\.\.\$PUBLICATION_HEAD"/,
  );
});

test('metadata credentials remain confined to default-branch publication jobs', async () => {
  const source = await workflow('publication-content');
  const prepare = source.split('\n  prepare:\n')[1].split('\n  scan:\n')[0];
  const scan = source.split('\n  scan:\n')[1];
  assert.match(prepare, /ref: \$\{\{ github\.event\.repository\.default_branch \}\}/);
  assert.match(scan, /ref: \$\{\{ needs\.prepare\.outputs\.trusted \}\}/);
  for (const job of [prepare, scan]) {
    assert.match(job, /persist-credentials: false/);
    assert.match(job, /GITHUB_TOKEN: \$\{\{ github\.token \}\}/);
    assert.doesNotMatch(job, /actions\/download-artifact|pull_request\.head|refs\/pull/);
  }
  assert.match(prepare, /run: node scripts\/publication\/recheck\.mjs prepare/);
  assert.match(scan, /run: node scripts\/publication\/recheck-worker\.mjs/);
});
