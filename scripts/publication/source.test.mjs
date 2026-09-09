import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { access, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { scanRepository } from '../security/scan-secrets.mjs';
import { scanText } from './recheck.mjs';
import { withCandidateSource } from './source.mjs';

const relayPath = '.github/workflows/publication-review.yml';
const relayText = await readFile(new URL('../../' + relayPath, import.meta.url), 'utf8');

const marker = () => ['gh', 'p_'].join('') + randomBytes(18).toString('hex');

function git(repo, ...args) {
  const result = spawnSync('git', ['-c', 'core.hooksPath=/dev/null', ...args], {
    cwd: repo,
    encoding: 'utf8',
    env: { ...process.env, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null' },
  });
  assert.equal(result.status, 0, 'Disposable Git operation must succeed.');
  return result.stdout.trim();
}

async function fixture(t) {
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-trusted-source-'));
  t.after(() => rm(repo, { recursive: true, force: true }));
  git(repo, 'init', '-q', '-b', 'main');
  git(repo, 'config', 'user.name', 'Synthetic Contributor');
  git(repo, 'config', 'user.email', 'contributor@example.invalid');
  await writeFile(join(repo, 'README.md'), 'Synthetic trusted baseline.\n');
  await mkdir(join(repo, '.github/workflows'), { recursive: true });
  await writeFile(join(repo, relayPath), relayText);
  git(repo, 'add', '.');
  git(repo, 'commit', '-qm', 'Synthetic baseline');
  const base = git(repo, 'rev-parse', 'HEAD');
  return { repo, base };
}

function transport(repo, head, observe = () => {}) {
  return (command, args, options) => {
    observe(command, args, options);
    if (args.includes('fetch')) {
      const ref = args.at(-1).split(':')[1];
      git(repo, 'update-ref', ref, head);
      return { status: 0, stdout: '' };
    }
    return spawnSync(command, args, options);
  };
}

test('trusted scanning rejects a candidate that replaces its own scanner with a success stub', async (t) => {
  const { repo, base } = await fixture(t);
  const secret = marker();
  git(repo, 'checkout', '--detach', '-q');
  const path = join(repo, 'scripts/security/scan-secrets.mjs');
  await mkdir(dirname(path), { recursive: true });
  await writeFile(
    path,
    "import { writeFileSync } from 'node:fs';\nwriteFileSync('candidate-was-run', 'yes');\nprocess.exit(0);\n",
  );
  await writeFile(join(repo, 'candidate.txt'), secret + '\n');
  git(repo, 'add', '.');
  git(repo, 'commit', '-qm', 'Synthetic scanner bypass candidate');
  const head = git(repo, 'rev-parse', 'HEAD');
  git(repo, 'checkout', '-q', 'main');
  assert.deepEqual(await scanRepository({ repo, diffs: [], content: [] }), []);
  await assert.rejects(
    scanText(
      repo,
      ['Reviewed synthetic content.'],
      { repository: 'example/project', number: 3, base, head },
      { token: 'synthetic-read-token', run: transport(repo, head) },
    ),
    (error) => {
      assert.match(error.message, /Detected secret/);
      assert.ok(!error.message.includes(secret));
      return true;
    },
  );
  assert.equal(git(repo, 'rev-parse', 'HEAD'), base);
  assert.equal(git(repo, 'status', '--porcelain'), '');
  assert.equal(git(repo, 'for-each-ref', '--format=%(refname)', 'refs/publication-content/'), '');
  await assert.rejects(access(join(repo, 'candidate-was-run')));
  // Removing the temporary candidate ref prevents this failed candidate from
  // contaminating a later scan; pre-existing reachable refs remain in scope.
  assert.deepEqual(await scanRepository({ repo, diffs: [], content: [] }), []);
  await scanText(
    repo,
    ['Reviewed synthetic content.'],
    { repository: 'example/project', number: 4, base, head: base },
    { token: 'synthetic-read-token', run: transport(repo, base) },
  );
});

test('fetch credentials stay transient, fixed-origin and out of arguments and scanner callbacks', async (t) => {
  const { repo, base } = await fixture(t);
  const token = marker();
  const before = await readFile(join(repo, '.git/config'), 'utf8');
  let authenticated = 0;
  await withCandidateSource(
    repo,
    { repository: 'example/project', number: 3, base, head: base },
    async (selection) => {
      assert.deepEqual(selection, { base, head: base });
    },
    {
      token,
      run: transport(repo, base, (_command, args, options) => {
        assert.ok(!JSON.stringify(args).includes(token));
        assert.equal(options.env.GITHUB_TOKEN, undefined);
        assert.equal(options.env.GH_TOKEN, undefined);
        if (args.includes('fetch')) {
          authenticated++;
          assert.equal(args.at(-2), 'https://github.com/example/project.git');
          assert.match(
            options.env.GIT_CONFIG_KEY_0,
            /^http\.https:\/\/github\.com\/example\/project\.git\.extraheader$/,
          );
          assert.equal(
            options.env.GIT_CONFIG_VALUE_0,
            'AUTHORIZATION: basic ' + Buffer.from('x-access-token:' + token).toString('base64'),
          );
        } else assert.equal(options.env.GIT_CONFIG_VALUE_0, undefined);
      }),
    },
  );
  assert.equal(authenticated, 1);
  assert.equal(await readFile(join(repo, '.git/config'), 'utf8'), before);
});

test('fetch failure and head mismatch clean temporary refs and never expose raw diagnostics', async (t) => {
  const { repo, base } = await fixture(t);
  const source = { repository: 'example/project', number: 3, base, head: 'a'.repeat(40) };
  await assert.rejects(
    withCandidateSource(repo, source, () => assert.fail('Mismatched source must not scan'), {
      token: 'synthetic-token',
      run: transport(repo, base),
    }),
    /changed/,
  );
  const secret = marker();
  const real = transport(repo, base);
  await assert.rejects(
    withCandidateSource(
      repo,
      { ...source, head: base },
      () => assert.fail('Failed fetch must not scan'),
      {
        token: 'synthetic-token',
        run: (command, args, options) =>
          args.includes('fetch')
            ? { status: 128, stdout: '', stderr: secret }
            : real(command, args, options),
      },
    ),
    (error) => {
      assert.ok(!String(error.stack).includes(secret));
      return true;
    },
  );
  assert.equal(git(repo, 'for-each-ref', '--format=%(refname)', 'refs/publication-content/'), '');
});

test('candidate and base relay changes cannot disable trusted review reconciliation', async (t) => {
  for (const side of ['head', 'base']) {
    for (const change of ['missing', 'renamed', 'symlink', 'events', 'condition']) {
      await t.test(`${side}-${change}`, async (t) => {
        const { repo, base: trusted } = await fixture(t);
        git(repo, 'checkout', '--detach', '-q');
        const relay = join(repo, relayPath);
        if (change === 'missing') await rm(relay);
        if (change === 'renamed') git(repo, 'mv', relayPath, '.github/workflows/renamed.yml');
        if (change === 'symlink') {
          await writeFile(join(repo, '.github/workflows/target.yml'), relayText);
          await rm(relay);
          await symlink('target.yml', relay);
        }
        if (change === 'events')
          await writeFile(
            relay,
            relayText.replace(/on:[\s\S]*?# This/, 'on: workflow_dispatch\n\n# This'),
          );
        if (change === 'condition')
          await writeFile(relay, relayText.replace('  signal:\n', '  signal:\n    if: false\n'));
        git(repo, 'add', '-A');
        git(repo, 'commit', '-qm', 'Synthetic relay change');
        const changed = git(repo, 'rev-parse', 'HEAD');
        git(repo, 'checkout', '-q', 'main');
        const base = side === 'base' ? changed : trusted;
        const head = side === 'head' ? changed : trusted;
        await assert.rejects(
          withCandidateSource(
            repo,
            { repository: 'example/project', number: 3, base, head },
            () => assert.fail('Changed relay must not reach the scanner or certification'),
            { token: 'synthetic-token', run: transport(repo, head) },
          ),
          /relay/,
        );
        assert.equal(git(repo, 'rev-parse', 'HEAD'), trusted);
        assert.equal(git(repo, 'status', '--porcelain'), '');
        assert.equal(
          git(repo, 'for-each-ref', '--format=%(refname)', 'refs/publication-content/'),
          '',
        );
      });
    }
  }
});

test('retained force-pushed heads are scanned and every temporary ref is removed', async (t) => {
  const { repo, base } = await fixture(t);
  git(repo, 'checkout', '--detach', '-q');
  await writeFile(join(repo, 'removed.txt'), marker());
  git(repo, 'add', '.');
  git(repo, 'commit', '-qm', 'Synthetic former PR head');
  const former = git(repo, 'rev-parse', 'HEAD');
  git(repo, 'checkout', '-q', 'main');
  assert.deepEqual(await scanRepository({ repo, diffs: [], content: [] }), []);
  const run = (command, args, options) => {
    if (args.includes('fetch')) {
      const [source, ref] = args.at(-1).slice(1).split(':');
      git(repo, 'update-ref', ref, source.startsWith('refs/pull/') ? base : source);
      return { status: 0, stdout: '' };
    }
    return spawnSync(command, args, options);
  };
  await assert.rejects(
    scanText(
      repo,
      ['Synthetic public text.'],
      {
        repository: 'example/project',
        number: 3,
        base,
        head: base,
        retainedHeads: [former],
      },
      { token: 'synthetic-token', run },
    ),
    /Detected secret/,
  );
  assert.equal(git(repo, 'for-each-ref', '--format=%(refname)', 'refs/publication-content/'), '');
  assert.deepEqual(await scanRepository({ repo, diffs: [], content: [] }), []);
  await assert.rejects(
    withCandidateSource(
      repo,
      {
        repository: 'example/project',
        number: 3,
        base,
        head: base,
        retainedHeads: [former],
      },
      async () => assert.fail('Missing retained objects cannot reach scanning'),
      {
        token: 'synthetic-token',
        run: (command, args, options) =>
          args.includes('fetch') && args.at(-1).includes(former)
            ? { status: 1, stdout: '' }
            : run(command, args, options),
      },
    ),
  );
  assert.equal(git(repo, 'for-each-ref', '--format=%(refname)', 'refs/publication-content/'), '');
});
