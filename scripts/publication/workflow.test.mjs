import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { runInNewContext } from 'node:vm';

const workflow = (name) =>
  readFile(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8');

test('source scanning remains in CI and repository-wide audits require explicit dispatch', async () => {
  await assert.rejects(workflow('publication'), { code: 'ENOENT' });
  await assert.rejects(workflow('publication-review'), { code: 'ENOENT' });
  const source = await workflow('ci');
  assert.equal(source.match(/run: pnpm publication:test/g).length, 1);
  assert.equal(source.match(/scan-secrets\.mjs --diff/g).length, 1);
  const advisory = await workflow('publication-content');
  const events = [
    ...advisory
      .split('\non:\n')[1]
      .split('\npermissions:')[0]
      .matchAll(/^ {2}(\w+):/gm),
  ].map((x) => x[1]);
  assert.deepEqual(events, ['workflow_dispatch']);
  assert.doesNotMatch(advisory, /cron:|actions: read|workflow_run:/);
  const condition = advisory.split('\n  prepare:\n')[1].match(/if: \$\{\{ (.+) \}\}/)[1];
  for (const defaultBranch of ['main', 'develop']) {
    for (const ref of [
      `refs/heads/${defaultBranch}`,
      'refs/heads/feat/example',
      'refs/tags/example',
    ]) {
      const accepted = runInNewContext(condition, {
        github: { ref, event: { repository: { default_branch: defaultBranch } } },
        format: (template, value) => template.replace('{0}', value),
      });
      assert.equal(accepted, ref === `refs/heads/${defaultBranch}`);
    }
  }
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
