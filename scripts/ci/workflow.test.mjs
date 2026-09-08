import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
const jobs = Object.fromEntries(
  [
    ...source
      .split('\njobs:\n')[1]
      .matchAll(/^ {2}([a-z0-9-]+):\n([\s\S]*?)(?=^ {2}[a-z0-9-]+:\n|$(?![\s\S]))/gm),
  ].map(([, name, body]) => [name, body]),
);

test('candidate CI jobs have no metadata credentials or grants', () => {
  assert.equal(Object.keys(jobs).length, 10);
  assert.match(source, /^permissions:\n {2}contents: read\n/m);
  assert.doesNotMatch(
    source.split('\njobs:\n')[0],
    /\b(?:token|secrets)\b|^\s+(?:pull-requests|issues|checks):/im,
  );
  for (const [name, job] of Object.entries(jobs)) {
    if (name === 'policy') continue;
    assert.doesNotMatch(job, /\b(?:GITHUB_TOKEN|GH_TOKEN)\b/i, name);
    assert.doesNotMatch(job, /\$\{\{[^}]*\b(?:token|secrets)\b/i, name);
    assert.doesNotMatch(job, /^\s+(?:permissions|issues|pull-requests|checks|actions):/m, name);
    for (const checkout of job.split(/- uses: actions\/checkout@/).slice(1)) {
      assert.match(checkout.split(/\n\s+- (?:uses|name|run):/)[0], /persist-credentials: false/);
    }
  }
  assert.doesNotMatch(jobs.publication, /publication:check/);
  assert.match(
    jobs.publication,
    /scan-secrets\.mjs --diff "\$PUBLICATION_BASE\.\.\$PUBLICATION_HEAD"/,
  );
});

test('credentialed policy runs only reviewed source with its matching stage map', () => {
  const job = jobs.policy;
  assert.equal(job.match(/actions\/checkout@/g).length, 1);
  assert.match(
    job,
    /ref: \$\{\{ vars\.CI_POLICY_SHA \|\| github\.event\.repository\.default_branch \}\}/,
  );
  assert.match(job, /\^\[0-9a-f\]\{40\}\$/);
  assert.match(job, /persist-credentials: false/);
  assert.match(job, /CI_POLICY_SHA=\$\(git rev-parse HEAD\)/);
  assert.doesNotMatch(
    job,
    /pull_request\.head|github\.sha|download-artifact|cache:|test:ci|test:supply-chain/,
  );
  assert.match(job, /pnpm install --frozen-lockfile --ignore-scripts/);
  assert.match(job, /GITHUB_TOKEN: \$\{\{ github\.token \}\}/);
});

test('the actual aggregate rejects every missing, skipped, cancelled or failed workflow dependency', () => {
  const required = Object.keys(jobs).filter((name) => name !== 'ci-ok');
  const declared = jobs['ci-ok'].match(/needs: \[([^\]]+)\]/)[1].split(', ');
  assert.deepEqual(declared.sort(), [...required].sort());
  const good = Object.fromEntries(required.map((name) => [name, { result: 'success' }]));
  const aggregate = (needs) =>
    spawnSync(process.execPath, [new URL('./aggregate.mjs', import.meta.url).pathname], {
      env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
      encoding: 'utf8',
    });
  assert.equal(aggregate(good).status, 0);
  for (const name of required) {
    for (const result of ['skipped', 'cancelled', 'failure', undefined]) {
      assert.equal(aggregate({ ...good, [name]: { result } }).status, 1, `${name}: ${result}`);
    }
  }
});
