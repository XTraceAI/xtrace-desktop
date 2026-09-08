import { execFileSync } from 'node:child_process';
import { readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { acceptedLicense, renderNotices } from './licenses.mjs';
import { rustNotices } from './rust-notices.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const run = (command, args) =>
  execFileSync(command, args, {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: 30 * 1024 * 1024,
    timeout: 180_000,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

async function main() {
  if (process.argv.slice(2).some((arg) => arg !== '--check'))
    throw new Error('Invalid notices option.');
  const tool = process.env.CARGO_ABOUT || 'cargo-about';
  if (run(tool, ['--version']).trim() !== 'cargo-about 0.9.2')
    throw new Error('Install the pinned cargo-about 0.9.2 tool.');
  const rust = JSON.parse(
    run(tool, ['generate', '--locked', '--workspace', '--fail', '--format', 'json']),
  );
  const entries = await rustNotices(rust);
  const npm = JSON.parse(run('pnpm', ['licenses', 'list', '--json', '--prod']));
  const modules = await realpath(join(root, 'node_modules'));
  for (const [license, packages] of Object.entries(npm)) {
    if (!acceptedLicense(license))
      throw new Error('A production npm dependency has an unsupported SPDX expression.');
    for (const pkg of packages) {
      if (!Array.isArray(pkg.paths) || pkg.paths.length === 0)
        throw new Error('npm license package paths are missing.');
      for (const path of pkg.paths) {
        const packageRoot = await realpath(path);
        const rel = relative(modules, packageRoot);
        if (rel.startsWith('..') || isAbsolute(rel))
          throw new Error('npm license source is outside the dependency tree.');
        const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
        if (manifest.name !== pkg.name || !pkg.versions.includes(manifest.version))
          throw new Error('npm license identity mismatch.');
        const files = (await readdir(packageRoot))
          .filter((name) => /^(?:licen[sc]e|copying|notice|ofl)(?:[._-].*)?$/i.test(name))
          .sort();
        if (files.length === 0)
          throw new Error('A production npm dependency has no bundled license text.');
        for (const file of files)
          entries.push({
            package: `npm: ${manifest.name}@${manifest.version}`,
            license,
            text: await readFile(join(packageRoot, file), 'utf8'),
          });
      }
    }
  }
  const output = renderNotices(entries);
  const destination = join(root, 'THIRD_PARTY_NOTICES.md');
  if (process.argv.includes('--check')) {
    if ((await readFile(destination, 'utf8')) !== output)
      throw new Error('Third-party notices are stale; run pnpm notices.');
  } else await writeFile(destination, output);
  console.log('Third-party license policy and notices passed.');
}

main().catch(() => {
  // Tool output, parser exceptions and filesystem errors may contain private paths.
  console.error(
    'License policy or notice generation failed; inspect the dependency inputs locally.',
  );
  process.exitCode = 1;
});
