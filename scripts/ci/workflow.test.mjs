import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { runInNewContext } from 'node:vm';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
const metadata = await readFile(
  new URL('../../.github/workflows/pr-metadata.yml', import.meta.url),
  'utf8',
);
const jobsOf = (workflow) =>
  Object.fromEntries(
    [
      ...workflow
        .split('\njobs:\n')[1]
        .matchAll(/^ {2}([a-z0-9-]+):\n([\s\S]*?)(?=^ {2}[a-z0-9-]+:\n|$(?![\s\S]))/gm),
    ].map(([, name, body]) => [name, body]),
  );
const jobs = jobsOf(source);
const editedJobs = jobsOf(metadata);

test('candidate CI jobs have no metadata credentials or grants', () => {
  assert.equal(Object.keys(jobs).length, 7);
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
  assert.doesNotMatch(jobs.security, /publication:check/);
  assert.match(
    jobs.security,
    /scan-secrets\.mjs --diff "\$PUBLICATION_BASE\.\.\$PUBLICATION_HEAD"/,
  );
});

test('portable checks use Linux while native and macOS dependency checks retain their platform', () => {
  for (const name of ['policy', 'ui', 'security', 'ci-ok'])
    assert.match(jobs[name], /runs-on: ubuntu-24\.04/, name);
  assert.match(editedJobs.policy, /runs-on: ubuntu-24\.04/);
  for (const name of ['rust', 'supply-chain', 'debug-bundle'])
    assert.match(jobs[name], /runs-on: macos-14/, name);
  assert.match(jobs['debug-bundle'], /test "\$\(uname -m\)" = arm64/);
  assert.match(jobs.ui, /path: ~\/\.cache\/ms-playwright/);
  assert.match(jobs.ui, /key: playwright-ubuntu-24\.04-x64-/);
  // Browser binaries may be cached; Linux system libraries must still be installed.
  assert.match(jobs.ui, /playwright install --with-deps webkit chromium/);
});

test('credentialed contribution checks run only reviewed source', () => {
  for (const job of [jobs.policy, editedJobs.policy]) {
    assert.equal(job.match(/actions\/checkout@/g).length, 1);
    assert.match(
      job,
      /ref: \$\{\{ vars\.CI_POLICY_SHA \|\| github\.event\.repository\.default_branch \}\}/,
    );
    assert.match(job, /\^\[0-9a-f\]\{40\}\$/);
    assert.match(job, /persist-credentials: false/);
    assert.doesNotMatch(job, /echo.*CI_POLICY_SHA=|issues: read/);
    assert.doesNotMatch(
      job,
      /pull_request\.head|github\.sha|download-artifact|cache:|test:ci|test:supply-chain/,
    );
    assert.match(job, /pnpm install --frozen-lockfile --ignore-scripts/);
    assert.match(job, /GITHUB_TOKEN: \$\{\{ github\.token \}\}/);
  }
  assert.equal(
    jobs.policy.split('    steps:\n')[1].trimEnd(),
    editedJobs.policy.split('    steps:\n')[1].trimEnd(),
  );
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

test('text edits run metadata only; target-branch edits call the full CI and cannot cancel it', () => {
  assert.match(source, /on:\n {2}workflow_call:\n/);
  assert.match(source, /types: \[opened, reopened, synchronize, ready_for_review\]/);
  assert.doesNotMatch(source, /\bedited\b/);
  assert.match(metadata, /pull_request:\n {4}types: \[edited\]/);
  assert.match(editedJobs['base-ci'], /uses: \.\/\.github\/workflows\/ci\.yml/);
  assert.deepEqual(Object.keys(editedJobs).sort(), ['base-ci', 'policy']);
  assert.doesNotMatch(metadata, /name: ci-ok|aggregate\.mjs|needs:/);
  const condition = (job, changes) => {
    const expression = job.match(/if: \$\{\{ (.+) \}\}/)[1];
    return Boolean(runInNewContext(expression, { github: { event: { changes } } }));
  };
  for (const changes of [
    {},
    { body: { from: 'previous text' } },
    { title: { from: 'old title' } },
  ]) {
    assert.equal(condition(editedJobs['base-ci'], changes), false);
    assert.equal(condition(editedJobs.policy, changes), true);
  }
  for (const changes of [{ base: { ref: { from: 'previous-base' } } }, { base: {}, body: {} }]) {
    assert.equal(condition(editedJobs['base-ci'], changes), true);
    assert.equal(condition(editedJobs.policy, changes), false);
  }
  // The base-triggered reusable CI and text-only workflow have distinct groups.
  assert.match(metadata, /changes\.base && 'base' \|\| 'text'/);
  assert.match(metadata, /group: pr-metadata-/);
  assert.match(source, /group: ci-/);
});

test('consolidated jobs retain each real command and do not blanket-accept failures', () => {
  for (const command of [
    'pnpm check',
    'playwright install --with-deps webkit chromium',
    'pnpm e2e',
  ])
    assert.ok(jobs.ui.includes(command), command);
  for (const command of [
    'cargo fmt',
    'cargo clippy',
    'cargo test',
    'run-hook.mjs plugin-conformance',
    'run-hook.mjs dto',
  ])
    assert.ok(jobs.rust.includes(command), command);
  assert.match(jobs.rust, /CI_TRUSTED_ROOT: \$\{\{ github\.workspace \}\}\/\.ci-trusted/);
  for (const command of [
    'pnpm test:ci',
    'pnpm test:supply-chain',
    'pnpm publication:test',
    'scan-secrets.mjs --diff',
  ])
    assert.ok(jobs.security.includes(command), command);
  for (const command of ['pnpm sbom', 'pnpm notices:check', '--content artifacts/sbom.cdx.json'])
    assert.ok(jobs['supply-chain'].includes(command), command);
  assert.match(jobs['debug-bundle'], /tauri build --debug/);
  assert.match(jobs['debug-bundle'], /scripts\/ci\/debug-bundle\.mjs/);
  assert.doesNotMatch(source + metadata, /continue-on-error|\|\| true/);
});
