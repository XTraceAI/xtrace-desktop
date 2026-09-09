import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const workflow = (name) =>
  readFile(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8');

test('one candidate source workflow owns scanning and all current metadata signals remain wired', async () => {
  await assert.rejects(workflow('publication'), { code: 'ENOENT' });
  const source = await workflow('ci');
  assert.equal(source.match(/run: pnpm publication:test/g).length, 1);
  assert.equal(source.match(/scan-secrets\.mjs --diff/g).length, 1);
  const advisory = await workflow('publication-content');
  assert.match(advisory, /workflows: \[CI, PR metadata, Publication review signal\]/);
  assert.match(
    advisory,
    /pull_request_target:\n {4}types: \[opened, reopened, synchronize, edited, ready_for_review\]/,
  );
  assert.match(advisory, /issue_comment:/);
  assert.match(advisory, /schedule:/);
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
