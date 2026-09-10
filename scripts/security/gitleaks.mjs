import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmod, lstat, mkdir, readFile, realpath, rename, writeFile } from 'node:fs/promises';
import { arch, homedir, platform } from 'node:os';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';

// Official v8.30.1 release checksums:
// https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_checksums.txt
const RELEASE = '8.30.1';
const ARCHIVES = {
  'darwin-arm64': [
    'darwin_arm64',
    'b40ab0ae55c505963e365f271a8d3846efbc170aa17f2607f13df610a9aeb6a5',
  ],
  'darwin-x64': ['darwin_x64', 'dfe101a4db2255fc85120ac7f3d25e4342c3c20cf749f2c20a18081af1952709'],
  'linux-arm64': [
    'linux_arm64',
    'e4a487ee7ccd7d3a7f7ec08657610aa3606637dab924210b3aee62570fb4b080',
  ],
  'linux-x64': ['linux_x64', '551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb'],
};
const ARCHIVE_LIMIT = 32 * 1024 * 1024;

export class ScanFailure extends Error {
  constructor(category) {
    super(category);
    this.name = 'ScanFailure';
  }
}

export function outsideRepository(repo, path) {
  const rel = relative(repo, path);
  return rel === '..' || rel.startsWith('../') || isAbsolute(rel);
}

export function cleanEnvironment() {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      ([key]) => !/^(?:(?:GIT|GITLEAKS)_|GITHUB_TOKEN$|GH_TOKEN$)/.test(key),
    ),
  );
}

function digest(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

async function download(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(60_000) });
  if (!response.ok || !response.body) throw new ScanFailure('scanner-download-failed');
  const chunks = [];
  let length = 0;
  for await (const chunk of response.body) {
    length += chunk.length;
    if (length > ARCHIVE_LIMIT) throw new ScanFailure('scanner-archive-too-large');
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function extractBinary(archive) {
  const tar = gunzipSync(archive, { maxOutputLength: 128 * 1024 * 1024 });
  for (let offset = 0; offset + 512 <= tar.length;) {
    const header = tar.subarray(offset, offset + 512);
    const name = header.subarray(0, 100).toString().replace(/\0.*$/s, '');
    if (!name) break;
    const size = Number.parseInt(
      header.subarray(124, 136).toString().replace(/\0.*$/s, '').trim(),
      8,
    );
    if (!Number.isSafeInteger(size) || size < 0 || offset + 512 + size > tar.length) {
      throw new ScanFailure('scanner-archive-invalid');
    }
    if (name === 'gitleaks' && (header[156] === 0 || header[156] === 48)) {
      return tar.subarray(offset + 512, offset + 512 + size);
    }
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  throw new ScanFailure('scanner-binary-missing');
}

export async function verifiedScanner(repo, scratch) {
  const asset = ARCHIVES[`${platform()}-${arch()}`];
  if (!asset) throw new ScanFailure('unsupported-scanner-platform');
  const [suffix, expected] = asset;
  const cache = join(homedir(), '.cache', 'xtrace-publication', `gitleaks-${RELEASE}`);
  if (!outsideRepository(repo, cache)) throw new ScanFailure('scanner-cache-inside-repository');
  await mkdir(cache, { recursive: true, mode: 0o700 });
  const cacheInfo = await lstat(cache);
  if (!cacheInfo.isDirectory() || cacheInfo.isSymbolicLink())
    throw new ScanFailure('unsafe-scanner-cache');
  await chmod(cache, 0o700);
  if (!outsideRepository(repo, await realpath(cache)))
    throw new ScanFailure('scanner-cache-inside-repository');
  const archivePath = join(cache, `gitleaks_${RELEASE}_${suffix}.tar.gz`);
  let archive;
  try {
    const info = await lstat(archivePath);
    if (!info.isFile() || info.isSymbolicLink() || info.size > ARCHIVE_LIMIT) {
      throw new ScanFailure('unsafe-scanner-cache');
    }
    archive = await readFile(archivePath);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    archive = await download(
      `https://github.com/gitleaks/gitleaks/releases/download/v${RELEASE}/gitleaks_${RELEASE}_${suffix}.tar.gz`,
    );
    if (digest(archive) !== expected) throw new ScanFailure('scanner-checksum-mismatch');
    const pending = `${archivePath}.${process.pid}.pending`;
    await writeFile(pending, archive, { mode: 0o600, flag: 'wx' });
    await rename(pending, archivePath);
  }
  if (digest(archive) !== expected) throw new ScanFailure('scanner-checksum-mismatch');
  const binary = join(scratch, 'gitleaks');
  await writeFile(binary, extractBinary(archive), { mode: 0o700, flag: 'wx' });
  return binary;
}

// Kept separate so process failures and redaction can be exercised with a test executable.
// The CLI always supplies the executable produced by verifiedScanner, never a PATH override.
export async function scanFiles(binary, targets, scratch) {
  const config = join(scratch, 'defaults.toml');
  const ignore = join(scratch, 'empty-ignore');
  const report = join(scratch, 'report.json');
  await writeFile(config, '[extend]\nuseDefault = true\n', { mode: 0o400, flag: 'wx' });
  await writeFile(ignore, '', { mode: 0o400, flag: 'wx' });
  const result = spawnSync(
    binary,
    [
      'dir',
      targets,
      '--config',
      config,
      '--gitleaks-ignore-path',
      ignore,
      '--ignore-gitleaks-allow',
      '--redact=100',
      '--no-banner',
      '--no-color',
      '--log-level',
      'error',
      '--report-format',
      'json',
      '--report-path',
      report,
      '--exit-code',
      '17',
      '--timeout',
      '300',
    ],
    {
      cwd: scratch,
      env: cleanEnvironment(),
      encoding: 'utf8',
      timeout: 310_000,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
  if (result.error || result.signal || ![0, 17].includes(result.status) || result.stderr.trim()) {
    throw new ScanFailure('scanner-execution-failed');
  }
  let findings;
  try {
    findings = JSON.parse(await readFile(report, 'utf8'));
  } catch {
    throw new ScanFailure('scanner-report-invalid');
  }
  if (!Array.isArray(findings) || (result.status === 17) !== findings.length > 0) {
    throw new ScanFailure('scanner-result-inconsistent');
  }
  for (const finding of findings) {
    if (
      !finding ||
      typeof finding.File !== 'string' ||
      !Number.isInteger(finding.StartLine) ||
      finding.StartLine < 1 ||
      typeof finding.RuleID !== 'string' ||
      !/^[a-z0-9-]{1,100}$/.test(finding.RuleID)
    ) {
      throw new ScanFailure('scanner-report-invalid');
    }
  }
  return findings.map(({ File, StartLine, RuleID }) => ({
    file: resolve(dirname(report), File),
    line: StartLine,
    rule: RuleID,
  }));
}
