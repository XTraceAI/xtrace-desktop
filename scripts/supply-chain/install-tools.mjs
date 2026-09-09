import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFile, chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { arch, homedir, platform } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

const syftVersion = '1.51.1';
const aboutVersion = '0.9.2';
// https://github.com/anchore/syft/releases/tag/v1.51.1
const archives = {
  'darwin-arm64': [
    'darwin_arm64',
    'ac063af3b9874769deb7ea1e6d76841e68f9e3bb50cd654226fc977de65532c1',
  ],
  'darwin-x64': [
    'darwin_amd64',
    '0e186ce1d4351ec276126851ca3ff258ed070e93e73574ed64858d4fc2339867',
  ],
  'linux-arm64': [
    'linux_arm64',
    'a7fd2b784e6664acd44719270574f6cd8c6864fc2b1700bf9099bd1cccda7d7f',
  ],
  'linux-x64': ['linux_amd64', '8fcb33017a0dc1058298c923c436d19dfa68ae93968e0b423248542e3afb9fc3'],
};
const cache = join(homedir(), '.cache/xtrace-ci-tools');
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');

export function verifiedSyft(archive, expected) {
  if (digest(archive) !== expected) throw new Error('Tool archive checksum mismatch.');
  return extractSyft(archive);
}

export function extractSyft(archive) {
  const tar = gunzipSync(archive, { maxOutputLength: 256 * 1024 * 1024 });
  for (let offset = 0; offset + 512 <= tar.length;) {
    const header = tar.subarray(offset, offset + 512);
    const name = header.subarray(0, 100).toString().replace(/\0.*$/s, '');
    if (!name) break;
    const size = Number.parseInt(
      header.subarray(124, 136).toString().replace(/\0.*$/s, '').trim(),
      8,
    );
    if (!Number.isSafeInteger(size) || size < 0 || offset + 512 + size > tar.length)
      throw new Error('Invalid tool archive.');
    if (name === 'syft' && (header[156] === 0 || header[156] === 48))
      return tar.subarray(offset + 512, offset + 512 + size);
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  throw new Error('Tool archive has no regular Syft executable.');
}

export async function installSyft() {
  const asset = archives[`${platform()}-${arch()}`];
  if (!asset) throw new Error('Unsupported tool platform.');
  const [suffix, expected] = asset;
  const directory = join(cache, `syft-${syftVersion}`);
  await mkdir(directory, { recursive: true });
  const archivePath = join(directory, `syft_${syftVersion}_${suffix}.tar.gz`);
  let archive;
  try {
    archive = await readFile(archivePath);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    const response = await fetch(
      `https://github.com/anchore/syft/releases/download/v${syftVersion}/syft_${syftVersion}_${suffix}.tar.gz`,
      { signal: AbortSignal.timeout(60_000) },
    );
    if (!response.ok || !response.body) throw new Error('Tool download failed.', { cause: error });
    const chunks = [];
    let size = 0;
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > 64 * 1024 * 1024)
        throw new Error('Tool archive exceeds the size limit.', { cause: error });
      chunks.push(chunk);
    }
    archive = Buffer.concat(chunks);
    if (digest(archive) !== expected)
      throw new Error('Tool archive checksum mismatch.', { cause: error });
    await writeFile(archivePath, archive);
  }
  const executable = join(directory, 'syft');
  // Always restore the executable from the checksum-verified archive.
  await writeFile(executable, verifiedSyft(archive, expected));
  await chmod(executable, 0o755);
  return directory;
}

export async function installAbout() {
  const directory = join(cache, `cargo-about-${aboutVersion}`);
  const executable = join(directory, 'bin/cargo-about');
  const options = {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 900_000,
    maxBuffer: 16 * 1024 * 1024,
  };
  let installed = false;
  try {
    installed =
      execFileSync(executable, ['--version'], options).trim() === `cargo-about ${aboutVersion}`;
  } catch {
    /* Install the pinned source below. */
  }
  if (!installed)
    execFileSync(
      'cargo',
      [
        'install',
        '--locked',
        '--features',
        'cli',
        '--version',
        aboutVersion,
        '--root',
        directory,
        'cargo-about',
      ],
      options,
    );
  if (execFileSync(executable, ['--version'], options).trim() !== `cargo-about ${aboutVersion}`)
    throw new Error('cargo-about version mismatch.');
  return join(directory, 'bin');
}

async function main() {
  const [tool, ...extra] = process.argv.slice(2);
  if (extra.length || !['syft', 'cargo-about'].includes(tool))
    throw new Error('Choose syft or cargo-about.');
  const directory = await (tool === 'syft' ? installSyft() : installAbout());
  if (process.env.GITHUB_ACTIONS === 'true' && process.env.GITHUB_PATH)
    await appendFile(process.env.GITHUB_PATH, directory + '\n');
  console.log(`Installed pinned ${tool}.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(() => {
    console.error('Pinned tool installation failed; inspect the installation locally.');
    process.exitCode = 1;
  });
}
