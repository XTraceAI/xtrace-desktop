import { createHash } from 'node:crypto';
import { readFile, readdir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const upstream = fileURLToPath(new URL('./upstream-licenses/', import.meta.url));
const licenseName = /^(?:licen[sc]e|copying|notice|ofl)(?:[._-].*)?$/i;

export async function rustNotices(rust) {
  const manifest = JSON.parse(await readFile(join(upstream, 'manifest.json'), 'utf8'));
  const entries = [];
  // cargo-about selects allowed SPDX alternatives and supplies full license text.
  // Add the package's own notices too: generic text alone loses copyrights.
  for (const license of rust.licenses) {
    for (const user of license.used_by) {
      if (!user.crate.source) continue;
      entries.push({
        package: `cargo: ${user.crate.name}@${user.crate.version}`,
        license: license.id,
        text: license.text,
      });
    }
  }
  for (const { package: pkg, license } of rust.crates) {
    if (!pkg.source) continue;
    const identity = `${pkg.name}@${pkg.version}`;
    const directory = await realpath(dirname(pkg.manifest_path));
    const files = (await readdir(directory, { withFileTypes: true }))
      .filter((entry) => entry.isFile() && licenseName.test(entry.name))
      .map((entry) => entry.name)
      .sort();
    if (pkg.license_file && !files.includes(pkg.license_file)) files.push(pkg.license_file);
    for (const file of files) {
      const path = await realpath(join(directory, file));
      const rel = relative(directory, path);
      if (rel === '..' || rel.startsWith('../') || isAbsolute(rel))
        throw new Error('Rust license source is outside its package.');
      entries.push({ package: `cargo: ${identity}`, license, text: await readFile(path, 'utf8') });
    }
    if (files.length) continue;
    const sources = manifest[identity];
    if (!Array.isArray(sources) || sources.length === 0)
      throw new Error('A Rust package needs its original upstream notice.');
    const vcs = JSON.parse(await readFile(join(directory, '.cargo_vcs_info.json'), 'utf8'));
    const repository = pkg.repository?.replace(/^https:\/\/github.com\//, '').replace(/\/$/, '');
    for (const source of sources) {
      const prefix = `https://raw.githubusercontent.com/${repository}/${vcs.git.sha1}/`;
      if (!source.url.startsWith(prefix) || !/^[a-f0-9]{64}\.txt$/.test(source.file))
        throw new Error('Upstream notice provenance does not match the locked package.');
      const text = await readFile(join(upstream, source.file), 'utf8');
      if (createHash('sha256').update(text).digest('hex') !== source.sha256)
        throw new Error('Upstream license notice checksum does not match.');
      entries.push({ package: `cargo: ${identity}`, license, text });
    }
  }
  return entries;
}
