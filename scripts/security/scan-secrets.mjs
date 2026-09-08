import { spawnSync } from 'node:child_process';
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  cleanEnvironment,
  outsideRepository,
  ScanFailure,
  scanFiles,
  verifiedScanner,
} from './gitleaks.mjs';

const MAX_BYTES = 256 * 1024 * 1024;
const MAX_FILE_BYTES = 32 * 1024 * 1024;
const MAX_COMMITS = 20_000;
const MAX_VERSIONS = 50_000;

function git(repo, args) {
  const result = spawnSync(
    'git',
    [
      '--no-replace-objects',
      '-c',
      'core.fsmonitor=false',
      '-c',
      'core.hooksPath=/dev/null',
      ...args,
    ],
    {
      cwd: repo,
      env: {
        ...cleanEnvironment(),
        GIT_CONFIG_NOSYSTEM: '1',
        GIT_CONFIG_GLOBAL: '/dev/null',
        GIT_TERMINAL_PROMPT: '0',
      },
      encoding: null,
      maxBuffer: 64 * 1024 * 1024,
      timeout: 30_000,
    },
  );
  if (result.error || result.signal || result.status !== 0)
    throw new ScanFailure('git-command-failed');
  return result.stdout;
}

function decode(bytes) {
  const text = bytes.toString('utf8');
  if (!Buffer.from(text).equals(bytes)) throw new ScanFailure('unsupported-git-filename-encoding');
  return text;
}

function commit(repo, ref) {
  const value = git(repo, ['rev-parse', '--verify', '--end-of-options', `${ref}^{commit}`])
    .toString()
    .trim();
  if (!/^[a-f0-9]{40,64}$/.test(value)) throw new ScanFailure('git-commit-invalid');
  return value;
}

export function parseArguments(args) {
  const options = { repo: process.cwd(), diffs: [], content: [] };
  for (let i = 0; i < args.length; i++) {
    const option = args[i];
    if (
      !['--repo', '--diff', '--content'].includes(option) ||
      !args[i + 1] ||
      args[i + 1].startsWith('--')
    ) {
      throw new ScanFailure('invalid-arguments');
    }
    const value = args[++i];
    if (option === '--repo') options.repo = resolve(value);
    else options[option === '--diff' ? 'diffs' : 'content'].push(value);
  }
  return options;
}

export async function scanRepository(options) {
  const repo = await realpath(options.repo);
  if (git(repo, ['rev-parse', '--is-shallow-repository']).toString().trim() !== 'false') {
    throw new ScanFailure('shallow-history');
  }
  const root = await realpath(git(repo, ['rev-parse', '--show-toplevel']).toString().trim());
  if (root !== repo) throw new ScanFailure('repository-root-required');
  const temporaryRoot = await realpath(tmpdir());
  if (!outsideRepository(repo, temporaryRoot))
    throw new ScanFailure('temporary-directory-inside-repository');
  const scratch = await mkdtemp(join(temporaryRoot, 'xtrace-secret-scan-'));
  try {
    const targets = join(scratch, 'sources');
    await mkdir(targets, { mode: 0o700 });
    const labels = new Map();
    const versions = new Set();
    const paths = new Set();
    let bytes = 0;
    let count = 0;
    async function add(data, kind, path) {
      if (
        !/^(?:git-ref|ref-target|git-path|git-blob|worktree|tag|tag-target|commit|diff|outbound)$/.test(
          kind,
        )
      )
        throw new ScanFailure('invalid-source-kind');
      bytes += data.length;
      count++;
      if (data.length > MAX_FILE_BYTES || bytes > MAX_BYTES || count > MAX_VERSIONS)
        throw new ScanFailure('scan-size-limit');
      const base = join(targets, String(count));
      // Preserve path-sensitive rules, and also scan an opaque text alias so default path
      // exclusions cannot hide credentials in lockfiles, generated files or fixture names.
      const destinations = [join(base, 'content.txt')];
      if (path) {
        if (isAbsolute(path) || path.split('/').some((part) => part === '..' || part === ''))
          throw new ScanFailure('unsafe-git-path');
        destinations.push(join(base, 'original', path));
      }
      for (const destination of destinations) {
        await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
        await writeFile(destination, data, { mode: 0o600, flag: 'wx' });
        // Paths may contain credentials of any shape; never derive public labels from them.
        labels.set(destination, `${kind}[${count}]`);
      }
    }
    async function blob(oid, path) {
      // A Git filename is published data even when its blob is empty. Scan it
      // as opaque text; Gitleaks otherwise uses paths only for filtering/reporting.
      if (!paths.has(path)) {
        paths.add(path);
        await add(Buffer.from(path + '\n'), 'git-path');
      }
      const key = `${oid}:${path}`;
      if (versions.has(key)) return;
      versions.add(key);
      await add(git(repo, ['cat-file', 'blob', oid]), 'git-blob', path);
    }
    const head = commit(repo, 'HEAD');
    // Ref names are published independently of the objects they reference.
    for (const ref of decode(git(repo, ['for-each-ref', '--format=%(refname)']))
      .split('\n')
      .filter(Boolean))
      await add(Buffer.from(ref + '\n'), 'git-ref');
    // rev-list follows tag targets but omits the annotated tag objects. Scan
    // every tag object in reachable chains, including its message and tagger.
    const targetsByType = git(repo, ['for-each-ref', '--format=%(objecttype) %(objectname)'])
      .toString()
      .trim()
      .split('\n')
      .filter(Boolean);
    const tags = [];
    for (const target of new Set(targetsByType)) {
      const match = /^(tag|commit|blob) ([a-f0-9]{40,64})$/.exec(target);
      if (!match) throw new ScanFailure('ref-target-unsupported');
      if (match[1] === 'tag') tags.push(match[2]);
      else if (match[1] === 'blob')
        await add(git(repo, ['cat-file', 'blob', match[2]]), 'ref-target');
    }
    const seenTags = new Set();
    while (tags.length) {
      const oid = tags.pop();
      if (seenTags.has(oid)) continue;
      if (!/^[a-f0-9]{40,64}$/.test(oid)) throw new ScanFailure('invalid-tag-object');
      seenTags.add(oid);
      const data = git(repo, ['cat-file', 'tag', oid]);
      await add(data, 'tag');
      const target = /^object ([a-f0-9]{40,64})\ntype (tag|commit|blob|tree)\n/.exec(
        data.toString(),
      );
      if (!target) throw new ScanFailure('invalid-tag-object');
      if (target[2] === 'tag') tags.push(target[1]);
      else if (target[2] === 'blob')
        await add(git(repo, ['cat-file', 'blob', target[1]]), 'tag-target');
      else if (target[2] !== 'commit') throw new ScanFailure('tag-target-unsupported');
    }
    const commits = new Set(git(repo, ['rev-list', '--all', head]).toString().trim().split('\n'));
    for (const range of options.diffs) {
      const parts = range.split('..');
      if (
        parts.length !== 2 ||
        !parts[0] ||
        !parts[1] ||
        parts.some((part) => part.startsWith('.'))
      )
        throw new ScanFailure('invalid-diff-range');
      const base = commit(repo, parts[0]);
      const tip = commit(repo, parts[1]);
      commits.add(base);
      commits.add(tip);
      for (const item of git(repo, ['rev-list', `${base}..${tip}`])
        .toString()
        .trim()
        .split('\n')
        .filter(Boolean))
        commits.add(item);
      await add(
        git(repo, [
          'diff',
          '--no-ext-diff',
          '--no-textconv',
          '--no-color',
          '--unified=0',
          base,
          tip,
          '--',
        ]),
        'diff',
      );
    }
    if (commits.size > MAX_COMMITS) throw new ScanFailure('scan-history-limit');
    for (const oid of commits) {
      await add(git(repo, ['cat-file', 'commit', oid]), 'commit');
      const entries = decode(git(repo, ['ls-tree', '-rz', '--full-tree', oid]))
        .split('\0')
        .filter(Boolean);
      for (const entry of entries) {
        const match = /^(\d+) (\w+) ([a-f0-9]+)\t([\s\S]+)$/.exec(entry);
        if (!match || match[2] !== 'blob')
          throw new ScanFailure('submodule-or-tree-entry-unsupported');
        await blob(match[3], match[4]);
      }
    }
    const indexEntries = decode(git(repo, ['ls-files', '--stage', '-z']))
      .split('\0')
      .filter(Boolean);
    const current = new Set();
    for (const entry of indexEntries) {
      const match = /^(\d+) ([a-f0-9]+) [0-3]\t([\s\S]+)$/.exec(entry);
      if (!match || match[1] === '160000')
        throw new ScanFailure('submodule-or-index-entry-unsupported');
      await blob(match[2], match[3]);
      current.add(match[3]);
    }
    for (const path of current) {
      const file = join(repo, path);
      let info;
      try {
        info = await lstat(file);
      } catch (error) {
        if (error.code === 'ENOENT') continue;
        throw error;
      }
      if (info.isSymbolicLink()) await add(Buffer.from(await readlink(file)), 'worktree', path);
      else if (info.isFile() && info.size <= MAX_FILE_BYTES) {
        if (outsideRepository(repo, await realpath(file)))
          throw new ScanFailure('tracked-file-outside-repository');
        await add(await readFile(file), 'worktree', path);
      } else throw new ScanFailure('tracked-file-unsupported');
    }
    for (const path of options.content) {
      const info = await lstat(path);
      if (!info.isFile() || info.isSymbolicLink() || info.size > MAX_FILE_BYTES)
        throw new ScanFailure('outbound-content-unsupported');
      await add(await readFile(path), 'outbound');
    }
    const binary = await verifiedScanner(repo, scratch);
    const findings = await scanFiles(binary, targets, scratch);
    const result = new Map();
    for (const finding of findings) {
      const path = labels.get(finding.file);
      if (!path) throw new ScanFailure('scanner-report-path-invalid');
      const diagnostic = { rule: finding.rule, path, line: finding.line };
      result.set(JSON.stringify(diagnostic), diagnostic);
    }
    return [...result.values()];
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

async function main() {
  try {
    const findings = await scanRepository(parseArguments(process.argv.slice(2)));
    for (const finding of findings) process.stderr.write(`${JSON.stringify(finding)}\n`);
    process.stdout.write(
      findings.length
        ? `Secret scan failed: ${findings.length} finding(s).\n`
        : 'Secret scan passed.\n',
    );
    process.exitCode = findings.length ? 1 : 0;
  } catch (error) {
    const category = error instanceof ScanFailure ? error.message : 'scan-could-not-complete';
    process.stderr.write(`Secret scan could not complete (${category}).\n`);
    process.exitCode = 2;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
