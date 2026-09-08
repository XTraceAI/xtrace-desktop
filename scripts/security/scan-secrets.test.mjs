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

test('current tracked edits are scanned and diagnostic paths/source are redacted', async (t) => {
  const repo = await fixture(t);
  const secret = marker();
  const path = `${secret}.txt`;
  await tracked(repo, path, 'Initially clean.\n');
  git(repo, 'commit', '-qm', 'Track file');
  await writeFile(join(repo, path), `sensitive source text ${secret}\n`);
  const result = scan(repo);
  expectDetected(result, secret);
  assert.match(result.output, /redacted/);
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
  assert.match(result.output, /tests\/fixtures\/secret.txt/);
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
  assert.match(result.output, /outbound\[1\]/);
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
