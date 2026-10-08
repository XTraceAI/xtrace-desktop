import { spawnSync } from 'node:child_process';
import { lstat, mkdtemp, readFile, readlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));

// The icon positions people should see come from bundle.macOS.dmg in tauri.conf.json;
// this check never keeps its own copy of the numbers.
export async function expectedLayout(
  configPath = join(root, 'apps/desktop/src-tauri/tauri.conf.json'),
) {
  const config = JSON.parse(await readFile(configPath, 'utf8'));
  const dmg = config.bundle?.macOS?.dmg;
  const position = (point) => {
    if (!Number.isInteger(point?.x) || !Number.isInteger(point?.y))
      throw new Error('tauri.conf.json must give whole-number disk image icon positions.');
    return { x: point.x, y: point.y };
  };
  return {
    app: `${config.productName}.app`,
    background: dmg?.background?.split('/').pop(),
    icons: {
      [`${config.productName}.app`]: position(dmg?.appPosition),
      Applications: position(dmg?.applicationFolderPosition),
    },
  };
}

// Reads the icon positions ("Iloc" records) that Finder saved in a .DS_Store file.
// The file is a buddy-allocated B-tree; see https://metacpan.org/dist/Mac-Finder-DSStore
// for the layout. Every other record type is skipped.
export function readIconPositions(bytes) {
  const data = Buffer.from(bytes);
  if (data.length < 36 || data.readUInt32BE(0) !== 1 || data.toString('latin1', 4, 8) !== 'Bud1')
    throw new Error('Not a .DS_Store file.');
  const cursor = (start, end) => {
    let at = start;
    const take = (length) => {
      if (at + length > end) throw new Error('Truncated .DS_Store block.');
      const slice = data.subarray(at, at + length);
      at += length;
      return slice;
    };
    return { take, u32: () => take(4).readUInt32BE(0), u8: () => take(1)[0] };
  };
  const rootOffset = data.readUInt32BE(8);
  if (rootOffset !== data.readUInt32BE(16)) throw new Error('Inconsistent .DS_Store header.');
  const rootBlock = cursor(rootOffset + 4, rootOffset + 4 + data.readUInt32BE(12));
  const blockCount = rootBlock.u32();
  rootBlock.u32();
  const addresses = [];
  for (let i = 0; i < Math.ceil(blockCount / 256) * 256; i += 1) addresses.push(rootBlock.u32());
  const directory = new Map();
  for (let entries = rootBlock.u32(); entries > 0; entries -= 1) {
    const name = rootBlock.take(rootBlock.u8()).toString('latin1');
    directory.set(name, rootBlock.u32());
  }
  const block = (number) => {
    const address = addresses[number];
    if (number >= blockCount || address === undefined) throw new Error('Missing .DS_Store block.');
    const start = (address & ~0x1f) + 4;
    return cursor(start, Math.min(start + 2 ** (address & 0x1f), data.length));
  };
  if (!directory.has('DSDB')) throw new Error('The .DS_Store file has no record tree.');
  const tree = block(directory.get('DSDB'));
  const rootNode = tree.u32();

  const positions = new Map();
  const readRecord = (node) => {
    const name = Buffer.from(node.take(node.u32() * 2))
      .swap16()
      .toString('utf16le');
    const code = node.take(4).toString('latin1');
    const type = node.take(4).toString('latin1');
    let value;
    if (type === 'bool') value = node.take(1);
    else if (type === 'long' || type === 'shor' || type === 'type') value = node.take(4);
    else if (type === 'comp' || type === 'dutc') value = node.take(8);
    else if (type === 'blob') value = node.take(node.u32());
    else if (type === 'ustr') value = node.take(node.u32() * 2);
    else throw new Error(`Unknown .DS_Store value type ${type}.`);
    if (code === 'Iloc' && type === 'blob' && value.length >= 8)
      positions.set(name, { x: value.readInt32BE(0), y: value.readInt32BE(4) });
  };
  const visited = new Set();
  const walk = (number) => {
    if (visited.has(number)) throw new Error('The .DS_Store record tree has a cycle.');
    visited.add(number);
    const node = block(number);
    const last = node.u32();
    const count = node.u32();
    for (let i = 0; i < count; i += 1) {
      if (last) walk(node.u32());
      readRecord(node);
    }
    if (last) walk(last);
  };
  walk(rootNode);
  return positions;
}

export function layoutProblems(positions, expected) {
  const problems = [];
  for (const [name, want] of Object.entries(expected.icons)) {
    const found = positions.get(name);
    if (!found) problems.push(`${name} has no saved icon position.`);
    else if (found.x !== want.x || found.y !== want.y)
      problems.push(`${name} is at ${found.x},${found.y}; expected ${want.x},${want.y}.`);
  }
  return problems;
}

const run = (command, args) => {
  const result = spawnSync(command, args, { encoding: 'utf8', stdio: 'pipe' });
  if (result.error || result.status !== 0)
    throw new Error(`${command} ${args[0]} failed: ${(result.stderr || '').trim()}`);
  return result.stdout;
};

// Mounts the disk image read-only at a private folder, hands the folder to `inspect`,
// and always detaches it again.
export async function withMountedImage(image, inspect) {
  const mountPoint = await mkdtemp(join(tmpdir(), 'xtrace-dmg-'));
  let attached = false;
  try {
    run('hdiutil', [
      'attach',
      image,
      '-readonly',
      '-nobrowse',
      '-noautoopen',
      '-noverify',
      '-mountpoint',
      mountPoint,
    ]);
    attached = true;
    return await inspect(mountPoint);
  } finally {
    if (attached) {
      const detach = spawnSync('hdiutil', ['detach', mountPoint], { stdio: 'pipe' });
      if (detach.status !== 0) spawnSync('hdiutil', ['detach', '-force', mountPoint]);
    }
    await rm(mountPoint, { recursive: true, force: true });
  }
}

// Checks that the mounted disk image shows the app and an Applications link where
// tauri.conf.json places them, with the branded background in place.
export async function inspectMountedLayout(mountPoint, expected) {
  const problems = [];
  if (!(await lstat(join(mountPoint, expected.app)).catch(() => null))?.isDirectory())
    problems.push(`${expected.app} is missing.`);
  const link = join(mountPoint, 'Applications');
  if (!(await lstat(link).catch(() => null))?.isSymbolicLink()) {
    problems.push('The Applications shortcut is missing.');
  } else if ((await readlink(link)) !== '/Applications') {
    problems.push('The Applications shortcut does not point to /Applications.');
  }
  if (
    expected.background &&
    !(await lstat(join(mountPoint, '.background', expected.background)).catch(() => null))
  )
    problems.push('The window background picture is missing.');
  const store = await readFile(join(mountPoint, '.DS_Store')).catch(() => null);
  if (!store) {
    problems.push('Finder saved no window layout (.DS_Store is missing).');
    return { positions: new Map(), problems };
  }
  const positions = readIconPositions(store);
  problems.push(...layoutProblems(positions, expected));
  return { positions, problems };
}

export async function checkDmgLayout(image, expected) {
  return withMountedImage(image, (mountPoint) => inspectMountedLayout(mountPoint, expected));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [image, ...extra] = process.argv.slice(2);
  if (process.platform !== 'darwin' || !image || extra.length) {
    console.error('Usage (macOS): node scripts/ci/dmg-layout.mjs PATH_TO.dmg');
    process.exitCode = 1;
  } else {
    expectedLayout()
      .then((expected) => checkDmgLayout(resolve(image), expected))
      .then(({ positions, problems }) => {
        for (const [name, { x, y }] of positions) console.log(`${name}: ${x},${y}`);
        if (problems.length) {
          for (const problem of problems) console.error(problem);
          process.exitCode = 1;
        } else {
          console.log('Disk image window layout matches tauri.conf.json.');
        }
      })
      .catch((error) => {
        console.error(error.message);
        process.exitCode = 1;
      });
  }
}
