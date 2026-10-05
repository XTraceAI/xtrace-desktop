import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { runInNewContext } from 'node:vm';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
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
  assert.doesNotMatch(release, /GITHUB_TOKEN|GH_TOKEN|secrets\.|contents: write|gh release/);
  assert.match(release, /persist-credentials: false/);
  assert.deepEqual(Object.keys(jobsOf(release)), ['native']);
  assert.match(release, /^permissions:\n {2}contents: read\n\n/m);
  assert.doesNotMatch(jobsOf(release).native, /^\s+permissions:/m);
  for (const action of release.matchAll(/uses: (\S+)/g)) assert.match(action[1], /@[a-f0-9]{40}$/);
  for (const checkout of release.split(/uses: actions\/checkout@/).slice(1))
    assert.match(checkout.split(/\n {6}- /)[0], /persist-credentials: false/);
  assert.match(jobs.ui, /path: ~\/\.cache\/ms-playwright/);
  assert.match(jobs.ui, /playwright install --with-deps webkit chromium/);
});

test('actual release shell guards accept only default history or matching reviewed tags', async (t) => {
  const steps = release.split(/\n {6}- /);
  const guardSteps = [
    steps.find((step) => step.startsWith('name: Require an explicit candidate')),
    steps.find((step) => step.startsWith('name: Verify candidate identity')),
  ];
  for (const step of guardSteps) {
    for (const [name, expression] of Object.entries({
      CANDIDATE_SHA: 'inputs.candidate_sha',
      WORKFLOW_REF: 'github.ref',
      WORKFLOW_SHA: 'github.workflow_sha',
      SOURCE_SHA: 'github.sha',
      DEFAULT_BRANCH: 'github.event.repository.default_branch',
    }))
      assert.ok(step.includes(`${name}: \${{ ${expression} }}`), name);
  }
  assert.ok(guardSteps[0].includes('WORKFLOW_REF_TYPE: ${{ github.ref_type }}'));
  const guards = guardSteps.map((step) =>
    step.split('        run: |\n')[1].replace(/^ {10}/gm, ''),
  );
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-release-guard-'));
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 'Synthetic CI',
    GIT_AUTHOR_EMAIL: 'ci@example.invalid',
    GIT_COMMITTER_NAME: 'Synthetic CI',
    GIT_COMMITTER_EMAIL: 'ci@example.invalid',
  };
  const git = (...args) => {
    const result = spawnSync('git', args, { cwd: repo, env, input: '', encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  // Only the OS probes are controlled: both scripts and all Git checks run as written.
  const runGuard = (index, values) =>
    spawnSync(
      'bash',
      [
        '--noprofile',
        '--norc',
        '-e',
        '-o',
        'pipefail',
        '-c',
        'uname() { echo "${TEST_MACHINE:-arm64}"; }; sw_vers() { echo "${TEST_MACOS:-14.7}"; };\n' +
          guards[index],
      ],
      { cwd: repo, env: { ...env, ...values }, encoding: 'utf8' },
    );
  try {
    git('init', '--template=', '--initial-branch=main');
    const tree = git('mktree');
    const base = git('commit-tree', tree, '-m', 'Synthetic baseline');
    const candidate = git('commit-tree', tree, '-p', base, '-m', 'Synthetic candidate');
    const descendant = git('commit-tree', tree, '-p', candidate, '-m', 'Synthetic later commit');
    git('update-ref', 'HEAD', candidate);
    git('update-ref', 'refs/remotes/origin/main', descendant);
    git('tag', 'v0.1.1', candidate);
    git('tag', '-a', 'v0.1.1-beta.1+qa.14', candidate, '-m', 'Synthetic annotated beta');
    const defaults = {
      CANDIDATE_SHA: candidate,
      WORKFLOW_REF: 'refs/heads/main',
      WORKFLOW_REF_TYPE: 'branch',
      WORKFLOW_SHA: descendant,
      SOURCE_SHA: descendant,
      DEFAULT_BRANCH: 'main',
    };
    const tagged = {
      ...defaults,
      WORKFLOW_REF: 'refs/tags/v0.1.1',
      WORKFLOW_REF_TYPE: 'tag',
      WORKFLOW_SHA: candidate,
      SOURCE_SHA: candidate,
    };
    const check = async (name, values, accepted, rejectedAt = 0) => {
      await t.test(name, () => {
        const results = [runGuard(0, values)];
        if (results[0].status === 0) results.push(runGuard(1, values));
        assert.equal(
          results.every((result) => result.status === 0),
          accepted,
          results.map((result) => result.stderr).join('\n'),
        );
        if (!accepted) assert.notEqual(results[rejectedAt]?.status, 0);
        if (!accepted && rejectedAt === 1) assert.equal(results[0].status, 0);
      });
    };
    await check(
      'default branch accepts an ancestor, without requiring workflow SHA equality',
      defaults,
      true,
    );
    git('update-ref', 'refs/remotes/origin/main', base);
    await check('default branch rejects a candidate outside main history', defaults, false, 1);
    await check('tag accepts identical commits outside main history', tagged, true);
    await check(
      'annotated SemVer beta resolves to the candidate commit',
      { ...tagged, WORKFLOW_REF: 'refs/tags/v0.1.1-beta.1+qa.14' },
      true,
    );
    for (const [field, value] of [
      ['WORKFLOW_REF', 'refs/heads/feature'],
      ['WORKFLOW_REF', 'refs/heads/v0.1.1'],
      ['WORKFLOW_REF', 'refs/pull/1/head'],
      ['WORKFLOW_REF', 'v0.1.1'],
      ['WORKFLOW_REF_TYPE', 'branch'],
      ['WORKFLOW_REF_TYPE', ''],
      ['WORKFLOW_SHA', base],
      ['WORKFLOW_SHA', ''],
      ['SOURCE_SHA', base],
      ['SOURCE_SHA', 'malformed'],
      ['CANDIDATE_SHA', base],
      ['CANDIDATE_SHA', candidate.slice(0, 7)],
      ['CANDIDATE_SHA', 'A'.repeat(40)],
      ['CANDIDATE_SHA', ''],
      ['CANDIDATE_SHA', `${candidate}\n`],
      ['CANDIDATE_SHA', `${candidate}; echo unexpected`],
    ])
      await check(`tag rejects ${field}=${value}`, { ...tagged, [field]: value }, false);
    for (const version of [
      '0.1',
      '01.1.1',
      '0.01.1',
      '0.1.01',
      '0.1.1-01',
      '0.1.1-',
      '0.1.1+',
      '0.1.1-beta..1',
      '0.1.1+qa..14',
      '0.1.1/extra',
      '0.1.1^{commit}',
      '0.1.1; echo unexpected',
      '0.1.1\n',
    ])
      await check(
        `tag rejects malformed v${version}`,
        { ...tagged, WORKFLOW_REF: `refs/tags/v${version}` },
        false,
      );
    await check(
      'default branch rejects a tag ref type',
      { ...defaults, WORKFLOW_REF_TYPE: 'tag' },
      false,
    );
    await check(
      'missing tag rejects even with matching event identities',
      { ...tagged, WORKFLOW_REF: 'refs/tags/v9.9.9' },
      false,
      1,
    );
    git('tag', 'v0.1.2', tree);
    await check(
      'a tag pointing to a tree rejects',
      { ...tagged, WORKFLOW_REF: 'refs/tags/v0.1.2' },
      false,
      1,
    );
    await check(
      'macOS 15 rejects despite identical source commits',
      { ...tagged, TEST_MACOS: '15.7' },
      false,
      1,
    );
    await check(
      'x86_64 rejects despite identical source commits',
      { ...tagged, TEST_MACHINE: 'x86_64' },
      false,
      1,
    );
    git('update-ref', 'HEAD', base);
    await check('tag rejects a different checked-out HEAD', tagged, false, 1);
    await check(
      'tag rejects a different resolved commit with all other identities matching',
      { ...tagged, CANDIDATE_SHA: base, WORKFLOW_SHA: base, SOURCE_SHA: base },
      false,
      1,
    );
    git('update-ref', 'HEAD', candidate);
    await t.test('tag moved after the first guard rejects at the Git identity guard', () => {
      assert.equal(runGuard(0, tagged).status, 0);
      git('update-ref', 'refs/tags/v0.1.1', base);
      assert.notEqual(runGuard(1, tagged).status, 0);
    });
  } finally {
    await rm(repo, { recursive: true, force: true });
  }
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
