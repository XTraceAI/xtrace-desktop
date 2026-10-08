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
const release = await readFile(
  new URL('../../.github/workflows/release-native.yml', import.meta.url),
  'utf8',
);
const nativeSource = await readFile(new URL('./native-check.mjs', import.meta.url), 'utf8');
const jobs = jobsOf(source);
const releaseJobs = jobsOf(release);
const editedJobs = jobsOf(metadata);

test('candidate CI jobs have no metadata credentials or grants', () => {
  assert.equal(Object.keys(jobs).length, 4);
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

test('routine CI is Linux-only and native work is explicit release preparation', () => {
  assert.deepEqual(Object.keys(jobs).sort(), ['ci-ok', 'policy', 'security', 'ui']);
  for (const [name, job] of Object.entries(jobs)) assert.match(job, /runs-on: ubuntu-24\.04/, name);
  assert.match(editedJobs.policy, /runs-on: ubuntu-24\.04/);
  assert.deepEqual(
    [
      ...release
        .split('\non:\n')[1]
        .split('\npermissions:')[0]
        .matchAll(/^ {2}(\w+):/gm),
    ].map((x) => x[1]),
    ['workflow_dispatch'],
  );
  assert.match(release, /runs-on: macos-14/);
  assert.match(release, /test "\$\(uname -m\)" = arm64/);
  assert.match(release, /sw_vers -productVersion/);
  assert.match(release, /CANDIDATE_SHA.*\^\[0-9a-f\]\{40\}\$/);
  assert.match(release, /test "\$WORKFLOW_REF" = "refs\/heads\/\$DEFAULT_BRANCH"/);
  assert.match(release, /git merge-base --is-ancestor "\$CANDIDATE_SHA"/);
  assert.match(release, /ref: \$\{\{ inputs\.candidate_sha \}\}/);
  assert.match(release, /REVIEWED_BASE_SHA="\$\(git rev-parse "\$CANDIDATE_SHA\^1"\)"/);
  assert.match(release, /pnpm check:native --base "\$REVIEWED_BASE_SHA" --release/);
  assert.doesNotMatch(
    release,
    /GITHUB_TOKEN|GH_TOKEN|contents: write|gh release|permissions:\n(?! {2}contents: read\n)/,
  );
  assert.doesNotMatch(releaseJobs.native, /secrets\.|environment:/);
  assert.match(release, /persist-credentials: false/);
  assert.match(jobs.ui, /path: ~\/\.cache\/ms-playwright/);
  assert.match(jobs.ui, /playwright install --with-deps webkit chromium/);
});

test('release validation provisions the producer required by native conformance', async () => {
  const pin = JSON.parse(
    await readFile(new URL('../../.plugin-pin', import.meta.url), 'utf8'),
  ).commit;
  assert.match(pin, /^[a-f0-9]{40}$/);
  const steps = release.split(/\n {6}- /);
  const producer = steps.findIndex((step) => step.includes('repository: XTraceAI/agent-plugins'));
  const validation = steps.findIndex((step) => step.includes('pnpm check:native'));
  assert.ok(producer > 0 && validation > producer, 'producer must exist before the mandatory gate');
  assert.equal(steps[producer].match(/\n {10}ref: ([a-f0-9]{40})(?: #[^\n]*)?\n/)[1], pin);
  assert.match(steps[producer], /persist-credentials: false/);
  const path = steps[producer].match(/\n {10}path: (.+)\n/)[1];
  assert.ok(
    steps[validation].includes(
      'AGENT_PLUGINS_DIR: ${{ github.workspace }}/' + path + '/plugins/memhub',
    ),
  );
  const ignored = await readFile(new URL('../../.gitignore', import.meta.url), 'utf8');
  assert.ok(
    ignored
      .split('\n')
      .some((line) => line.startsWith('/') && line.endsWith('/') && path.startsWith(line.slice(1))),
    'nested producer must not dirty the candidate checkout',
  );
});

test('only the signing step of the packaging job sees Apple values, after validation passes', () => {
  assert.deepEqual(Object.keys(releaseJobs), ['native', 'package']);
  const job = releaseJobs.package;
  assert.match(job, /^ {4}needs: native$/m);
  assert.match(job, /^ {4}environment: macos-signing$/m);
  assert.match(job, /runs-on: macos-14/);
  assert.doesNotMatch(job, /permissions:|continue-on-error|\|\| true/);
  for (const check of [
    /CANDIDATE_SHA.*\^\[0-9a-f\]\{40\}\$/,
    /test "\$WORKFLOW_REF" = "refs\/heads\/\$DEFAULT_BRANCH"/,
    /test "\$\(git rev-parse HEAD\)" = "\$CANDIDATE_SHA"/,
    /git merge-base --is-ancestor "\$CANDIDATE_SHA"/,
    /test "\$\(uname -m\)" = arm64/,
  ])
    assert.match(job, check);
  for (const checkout of job.split(/- uses: actions\/checkout@/).slice(1))
    assert.match(checkout.split(/\n\s+- (?:uses|name|run):/)[0], /persist-credentials: false/);
  const steps = job.split(/\n {6}- /);
  const build = steps.findIndex((step) => step.includes('pnpm tauri build --no-bundle'));
  const sign = steps.findIndex((step) => step.includes('node scripts/ci/release-dmg.mjs'));
  const upload = steps.findIndex((step) => step.includes('actions/upload-artifact@'));
  assert.ok(
    build > 0 && sign === build + 1 && upload === sign + 1,
    'compile, then sign, then upload',
  );
  assert.match(steps[build], /pnpm install --frozen-lockfile --ignore-scripts/);
  steps.forEach((step, index) => {
    if (index !== sign) assert.doesNotMatch(step, /secrets\.|APPLE_/, step.split('\n')[0]);
  });
  assert.deepEqual(
    [...steps[sign].matchAll(/secrets\.(\w+)/g)].map((match) => match[1]),
    [
      'APPLE_CERTIFICATE',
      'APPLE_CERTIFICATE_PASSWORD',
      'APPLE_API_KEY',
      'APPLE_API_ISSUER',
      'APPLE_API_PRIVATE_KEY',
    ],
  );
  assert.match(
    steps[sign],
    /APPLE_SIGNING_IDENTITY: 'Developer ID Application: .+ \([A-Z0-9]{10}\)'/,
  );
  assert.match(steps[upload], /path: artifacts\/release\/\n/);
  assert.match(steps[upload], /if-no-files-found: error/);
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
    'pnpm test:ci',
    'pnpm test:supply-chain',
    'pnpm publication:test',
    'scan-secrets.mjs --diff',
  ])
    assert.ok(jobs.security.includes(command), command);
  for (const command of [
    "['fmt', '--all'",
    "['clippy', '--workspace'",
    "['test', '--workspace'",
    "['sbom']",
    "['notices:check']",
    "'--debug'",
    "'plugin-conformance', 'dto'",
  ])
    assert.ok(nativeSource.replace(/\s/g, '').includes(command.replace(/\s/g, '')), command);
  assert.match(nativeSource, /CI_TRUSTED_ROOT = trusted/);
  assert.doesNotMatch(source + metadata, /continue-on-error|\|\| true/);
});
