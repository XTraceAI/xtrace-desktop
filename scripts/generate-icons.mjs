import { spawnSync } from 'node:child_process';
import { copyFile, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const assets = join(root, 'apps/desktop/ui/public');
const temporary = await mkdtemp(join(tmpdir(), 'xtrace-icons-'));
try {
  const result = spawnSync(
    'pnpm',
    ['tauri', 'icon', join(assets, 'mark.png'), '--output', temporary],
    { cwd: root, stdio: 'inherit' },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error('Tauri icon generation failed.');
  // Tauri emits ICNS entries in varying order. Keep the image payloads intact
  // and order their tagged records so regeneration does not dirty the checkout.
  const icns = await readFile(join(temporary, 'icon.icns'));
  if (
    icns.length < 8 ||
    icns.toString('ascii', 0, 4) !== 'icns' ||
    icns.readUInt32BE(4) !== icns.length
  )
    throw new Error('Invalid generated ICNS header.');
  const entries = new Map();
  for (let offset = 8; offset < icns.length;) {
    if (offset + 8 > icns.length) throw new Error('Truncated ICNS entry.');
    const tag = icns.toString('ascii', offset, offset + 4);
    const size = icns.readUInt32BE(offset + 4);
    if (size < 8 || offset + size > icns.length || entries.has(tag))
      throw new Error('Invalid or duplicate ICNS entry.');
    entries.set(tag, icns.subarray(offset, offset + size));
    offset += size;
  }
  const ordered = [...entries.keys()].sort().map((tag) => entries.get(tag));
  // The desktop targets macOS; retain only its configured bundle assets.
  await writeFile(
    join(assets, 'icons/icon.icns'),
    Buffer.concat([icns.subarray(0, 8), ...ordered]),
  );
  await copyFile(join(temporary, 'icon.png'), join(assets, 'icons/icon.png'));
} finally {
  await rm(temporary, { recursive: true, force: true });
}
