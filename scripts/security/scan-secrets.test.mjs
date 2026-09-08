import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { scanFiles } from './gitleaks.mjs';

const cli = fileURLToPath(new URL('./scan-secrets.mjs', import.meta.url));

function git(repo, ...args) {
  const result = spawnSync('git', args, {
    cwd: repo,
    env: { ...process.env, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null' },
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, 'Disposable Git operation must succeed.');
  return result.stdout.trim();
}

async function fixture(t) {
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-scan-test-'));
  t.after(() => rm(repo, { recursive: true, force: true }));
  git(repo, 'init', '-q', '-b', 'main');
  git(repo, 'config', 'user.name', 'Secret scanner test');
  git(repo, 'config', 'user.email', 'scanner-test@example.invalid');
  await writeFile(join(repo, 'README.md'), 'Synthetic scanner regression repository.\n');
  git(repo, 'add', 'README.md');
  git(repo, 'commit', '-qm', 'Initialize synthetic test');
  return repo;
}

// A runtime-generated token-shaped string, never issued by a service and never usable.
function marker() {
  return ['gh', 'p_'].join('') + randomBytes(18).toString('hex');
}

async function tracked(repo, path, content) {
  await mkdir(dirname(join(repo, path)), { recursive: true });
  await writeFile(join(repo, path), content, { mode: 0o600 });
  git(repo, 'add', '--', path);
}

function scan(repo, args = [], extraEnv = {}) {
  const result = spawnSync(process.execPath, [cli, '--repo', repo, ...args], {
    env: { ...process.env, ...extraEnv },
    encoding: 'utf8',
    timeout: 120_000,
  });
  assert.equal(result.error, undefined, 'Scanner subprocess must finish.');
  return { status: result.status, output: result.stdout + result.stderr };
}

function expectDetected(result, secret) {
  assert.equal(result.status, 1, 'A synthetic credential must block publication.');
  assert.match(result.output, /github-pat/);
  assert.ok(!result.output.includes(secret), 'Diagnostics must never contain the marker.');
  assert.ok(
    !result.output.includes('sensitive source text'),
    'Source lines must never be printed.',
  );
}

test('a clean full-history repository passes without a PATH scanner', async (t) => {
  const repo = await fixture(t);
  const fakeBin = join(repo, 'bin');
  await mkdir(fakeBin);
  await writeFile(join(fakeBin, 'gitleaks'), '#!/bin/sh\nexit 99\n', { mode: 0o700 });
  const result = scan(repo, [], { PATH: `${fakeBin}:${process.env.PATH}` });
  assert.equal(result.status, 0);
  assert.match(result.output, /Secret scan passed/);
});

test('current tracked edits are scanned with opaque diagnostics and no source text', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  const path = `${secret}.txt`;
  await tracked(repo, path, 'Initially clean.\n');
  git(repo, 'commit', '-qm', 'Track file');
  await writeFile(join(repo, path), `sensitive source text ${secret}\n`);
  const result = scan(repo);
  expectDetected(result, secret);
  assert.match(result.output, /worktree\[\d+\]/);
});

test('GitLab-shaped filenames and blobs use opaque labels without credential fragments', async (t) => {
  const repo = await fixture(t);
  const base = git(repo, 'rev-parse', 'HEAD');
  const suffix = randomBytes(5).toString('hex').slice(0, 9);
  // Routable tokens have a short dot-separated suffix that must also stay private.
  const secret = ['gl', 'pat-'].join('') + randomBytes(21).toString('base64url') + '.' + suffix;
  const path = `${secret}.txt`;
  await tracked(repo, path, `${secret}\n`);
  git(repo, 'commit', '-qm', 'Synthetic filename and blob marker');
  const result = scan(repo, ['--diff', `${base}..HEAD`]);
  assert.equal(result.status, 1, 'A synthetic GitLab marker must block publication.');
  assert.ok(result.output.includes('gitlab-pat'), 'The pinned GitLab detector must match.');
  for (const privateValue of [secret, suffix, path, repo])
    assert.ok(
      !result.output.includes(privateValue),
      'Diagnostics must omit credential/path fragments.',
    );
  const findings = result.output
    .split('\n')
    .filter((line) => line.startsWith('{'))
    .map(JSON.parse);
  for (const kind of ['git-path', 'git-blob', 'worktree', 'diff'])
    assert.ok(
      findings.some(({ path }) => path.startsWith(`${kind}[`)),
      'Each Git input kind must be tested.',
    );
  assert.ok(
    findings.every(({ path }) => /^(?:git-path|git-blob|worktree|diff)\[\d+\]$/.test(path)),
    'All diagnostic paths must use fixed kinds and opaque sequence numbers.',
  );
});

test('opaque diagnostics preserve path-sensitive detection internally', async (t) => {
  const repo = await fixture(t);
  const secret = randomBytes(12).toString('hex');
  await tracked(
    repo,
    'private-settings/nuget.config',
    `<add key="Password" value="${secret}" />\n`,
  );
  const result = scan(repo);
  assert.equal(result.status, 1);
  assert.ok(
    result.output.includes('nuget-config-password'),
    'The filename-dependent rule must match.',
  );
  assert.ok(!result.output.includes(secret), 'Diagnostics must omit the synthetic password.');
  assert.ok(
    !result.output.includes('private-settings'),
    'Diagnostics must omit the original path.',
  );
});

test('removed historical credentials remain blocking', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  await tracked(repo, 'removed.txt', `${secret}\n`);
  git(repo, 'commit', '-qm', 'Add synthetic historical marker');
  git(repo, 'rm', 'removed.txt');
  git(repo, 'commit', '-qm', 'Remove synthetic marker');
  expectDetected(scan(repo), secret);
});

test('credentials only in Git paths block staged, current and removed historical empty files', async (t) => {
  for (const kind of ['staged', 'current', 'historical'])
    await t.test(kind, async (t) => {
      const repo = await fixture(t);
      const secret = marker();
      const path = `tests/fixtures/${secret}/empty.txt`;
      await tracked(repo, path, '');
      if (kind !== 'staged') git(repo, 'commit', '-qm', 'Synthetic named file');
      if (kind === 'historical') {
        git(repo, 'rm', path);
        git(repo, 'commit', '-qm', 'Remove synthetic named file');
      }
      const result = scan(repo);
      expectDetected(result, secret);
      assert.match(result.output, /git-path\[/);
      assert.ok(!result.output.includes(path));
    });
});

test('credentials only in branch and lightweight-tag names are scanned opaquely', async (t) => {
  for (const kind of ['branch', 'tag']) {
    const repo = await fixture(t);
    const secret = marker();
    git(repo, kind, secret);
    const result = scan(repo);
    expectDetected(result, secret);
    assert.ok(result.output.includes('git-ref['), 'Ref names must be scanned as opaque inputs.');
    assert.ok(!result.output.includes('refs/'), 'Diagnostics must not expose original ref names.');
  }
});

function object(repo, type, input) {
  const result = spawnSync('git', ['hash-object', '-w', '-t', type, '--stdin'], {
    cwd: repo,
    input,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, 'Synthetic Git object creation must succeed.');
  return result.stdout.trim();
}

test('direct blob targets of lightweight tags and other refs are scanned opaquely', async (t) => {
  for (const ref of ['refs/tags/synthetic-blob', 'refs/synthetic/blob']) {
    const repo = await fixture(t);
    git(repo, 'update-ref', ref, object(repo, 'blob', 'Clean standalone blob.\n'));
    assert.equal(scan(repo).status, 0, 'A clean standalone blob ref must pass.');
    const secret = marker();
    git(repo, 'update-ref', ref, object(repo, 'blob', secret + '\n'));
    const result = scan(repo);
    expectDetected(result, secret);
    assert.match(result.output, /ref-target\[\d+\]/);
    assert.ok(!result.output.includes(ref), 'Diagnostics must omit the original ref.');
  }
});

test('direct tree refs fail closed without exposing or traversing their names', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  const blob = object(repo, 'blob', secret + '\n');
  const tree = object(
    repo,
    'tree',
    Buffer.concat([Buffer.from('100644 ' + secret + '\0'), Buffer.from(blob, 'hex')]),
  );
  git(repo, 'update-ref', 'refs/tags/synthetic-tree', tree);
  const result = scan(repo);
  assert.equal(result.status, 2, 'Unsupported tree refs must block a clean result.');
  assert.match(result.output, /ref-target-unsupported/);
  assert.ok(!result.output.includes(secret), 'Diagnostics must omit tree names and contents.');
});

test('annotated tag messages, nested tags and tagger metadata are scanned without leaking values', async (t) => {
  for (const kind of ['message', 'nested', 'tagger']) {
    await t.test(kind, async (t) => {
      const repo = await fixture(t);
      const secret = marker();
      if (kind === 'tagger')
        git(
          repo,
          '-c',
          'user.name=' + secret,
          'tag',
          '-a',
          'synthetic',
          '-m',
          'Clean synthetic tag',
        );
      else {
        git(repo, 'tag', '-a', 'synthetic', '-m', 'sensitive source text ' + secret);
        if (kind === 'nested') {
          git(repo, 'tag', '-a', 'outer', 'synthetic', '-m', 'Clean outer tag');
          git(repo, 'tag', '-d', 'synthetic');
        }
      }
      expectDetected(scan(repo), secret);
    });
  }
});

test('fixture paths, inline suppression and repository/environment configurations cannot bypass scanning', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  await tracked(repo, 'tests/fixtures/secret.txt', `${secret} # gitleaks:allow\n`);
  await writeFile(
    join(repo, '.gitleaks.toml'),
    '[extend]\nuseDefault = true\n[allowlist]\nregexes = [".*"]\n',
  );
  const result = scan(repo, [], {
    GITLEAKS_CONFIG: join(repo, '.gitleaks.toml'),
    GITLEAKS_CONFIG_TOML: '[allowlist]\nregexes = [".*"]\n',
  });
  expectDetected(result, secret);
  assert.match(result.output, /git-blob\[\d+\]/);
  assert.ok(!result.output.includes('tests/fixtures/secret.txt'));
});

test('default filename exclusions cannot hide lockfile credentials', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  await tracked(repo, 'pnpm-lock.yaml', `synthetic: ${secret}\n`);
  expectDetected(scan(repo), secret);
});

test('a supplied PR diff scans a source head that is not in reachable refs', async (t) => {
  const repo = await fixture(t);
  const base = git(repo, 'rev-parse', 'HEAD');
  git(repo, 'checkout', '--detach', '-q');
  const secret = marker();
  await tracked(repo, 'pr-only.txt', `${secret}\n`);
  git(repo, 'commit', '-qm', 'Synthetic detached PR change');
  const head = git(repo, 'rev-parse', 'HEAD');
  git(repo, 'checkout', '-q', 'main');
  assert.equal(scan(repo).status, 0, 'The detached source head is absent from all refs.');
  expectDetected(scan(repo, ['--diff', `${base}..${head}`]), secret);
});

test('outbound PR/release text is scanned without disclosing its local path', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  const outbound = join(repo, 'outbound-private.md');
  await writeFile(outbound, `${secret}\n`, { mode: 0o600 });
  const result = scan(repo, ['--content', outbound]);
  expectDetected(result, secret);
  assert.match(result.output, /outbound\[\d+\]/);
  assert.ok(!result.output.includes(outbound));
});

test('shallow history and invalid diff refs fail closed', async (t) => {
  const repo = await fixture(t);
  const parent = await mkdtemp(join(tmpdir(), 'xtrace-shallow-test-'));
  t.after(() => rm(parent, { recursive: true, force: true }));
  const shallow = join(parent, 'checkout');
  git(parent, 'clone', '-q', '--depth', '1', `file://${repo}`, shallow);
  const result = scan(shallow);
  assert.equal(result.status, 2);
  assert.match(result.output, /shallow-history/);
  const secret = marker();
  const invalid = scan(repo, ['--diff', `HEAD..${secret}`]);
  assert.equal(invalid.status, 2);
  assert.ok(!invalid.output.includes(secret));
});

test('scanner execution errors are fail-closed and discard raw stderr', async (t) => {
  const scratch = await mkdtemp(join(tmpdir(), 'xtrace-scanner-failure-'));
  t.after(() => rm(scratch, { recursive: true, force: true }));
  const targets = join(scratch, 'sources');
  await mkdir(targets);
  const binary = join(scratch, 'failing-scanner');
  const secret = marker();
  await writeFile(binary, `#!/bin/sh\nprintf '%s' '${secret}' >&2\nexit 2\n`, { mode: 0o700 });
  await assert.rejects(scanFiles(binary, targets, scratch), (error) => {
    assert.equal(error.message, 'scanner-execution-failed');
    assert.ok(!String(error.stack).includes(secret));
    assert.equal(error.cause, undefined);
    return true;
  });
});

test('a substituted tracked parent directory cannot make the scan read outside the repository', async (t) => {
  const repo = await fixture(t);
  const external = await mkdtemp(join(tmpdir(), 'xtrace-external-scan-test-'));
  t.after(() => rm(external, { recursive: true, force: true }));
  await tracked(repo, 'config/settings.txt', 'Clean tracked settings.\n');
  git(repo, 'commit', '-qm', 'Track synthetic settings');
  await writeFile(join(external, 'settings.txt'), `${marker()}\n`, { mode: 0o600 });
  await rm(join(repo, 'config'), { recursive: true });
  await symlink(external, join(repo, 'config'));
  const result = scan(repo);
  assert.equal(result.status, 2);
  assert.match(result.output, /tracked-file-outside-repository/);
  assert.ok(!result.output.includes(external));
});

test('a scanner success exit without a valid report cannot pass', async (t) => {
  const scratch = await mkdtemp(join(tmpdir(), 'xtrace-scanner-report-'));
  t.after(() => rm(scratch, { recursive: true, force: true }));
  const targets = join(scratch, 'sources');
  await mkdir(targets);
  const binary = join(scratch, 'incomplete-scanner');
  await writeFile(binary, '#!/bin/sh\nexit 0\n', { mode: 0o700 });
  await assert.rejects(scanFiles(binary, targets, scratch), { message: 'scanner-report-invalid' });
});

test('scanner error diagnostics cannot be hidden behind a clean report and success exit', async (t) => {
  const scratch = await mkdtemp(join(tmpdir(), 'xtrace-scanner-error-log-'));
  t.after(() => rm(scratch, { recursive: true, force: true }));
  const targets = join(scratch, 'sources');
  await mkdir(targets);
  const binary = join(scratch, 'error-logging-scanner');
  const secret = marker();
  await writeFile(
    binary,
    `#!/bin/sh
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--report-path" ]; then shift; printf '[]' > "$1"; fi
  shift
done
printf '%s' '${secret}' >&2
exit 0
`,
    { mode: 0o700 },
  );
  await assert.rejects(scanFiles(binary, targets, scratch), {
    message: 'scanner-execution-failed',
  });
});

test('Git LFS pointers fail closed in current, historical and direct-ref blobs', async (t) => {
  for (const kind of ['current', 'historical', 'direct-ref', 'worktree']) {
    await t.test(kind, async (t) => {
      const repo = await fixture(t);
      const pointer =
        'version https://git-lfs.github.com/spec/v1\n' +
        'oid sha256:' +
        'a'.repeat(64) +
        '\nsize 100\n';
      if (kind === 'worktree') {
        await writeFile(join(repo, 'README.md'), pointer);
      } else {
        await tracked(repo, 'asset.dat', pointer);
        git(repo, 'commit', '-qm', 'Synthetic LFS pointer');
        if (kind === 'historical') {
          git(repo, 'rm', 'asset.dat');
          git(repo, 'commit', '-qm', 'Remove current pointer');
        }
        if (kind === 'direct-ref') {
          const oid = git(repo, 'rev-parse', 'HEAD:asset.dat');
          git(repo, 'reset', '--hard', 'HEAD~1');
          git(repo, 'update-ref', 'refs/tags/synthetic-lfs', oid);
        }
      }
      const result = scan(repo);
      assert.equal(result.status, 2);
      assert.match(result.output, /git-lfs-object-unsupported/);
      assert.doesNotMatch(result.output, /Secret scan passed/);
    });
  }
});
