import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { gzipSync } from 'node:zlib';
import { extractSyft, verifiedSyft } from './install-tools.mjs';

function archive(name, type = '0', declaredSize = 3) {
  const tar = Buffer.alloc(2048);
  tar.write(name, 0, 100);
  tar.write(declaredSize.toString(8).padStart(11, '0') + '\0', 124);
  tar.write(type, 156);
  tar.write('bin', 512);
  return gzipSync(tar);
}

test('only checksum-verified regular Syft bytes can be installed', () => {
  const bytes = archive('syft');
  const sha = createHash('sha256').update(bytes).digest('hex');
  assert.equal(verifiedSyft(bytes, sha).toString(), 'bin');
  assert.throws(() => verifiedSyft(bytes, '0'.repeat(64)), /checksum/);
  assert.throws(() => verifiedSyft(Buffer.from('invalid gzip'), sha), /checksum/);
  for (const name of ['../syft', '/syft', 'directory/syft'])
    assert.throws(() => extractSyft(archive(name)), /no regular/);
  for (const type of ['1', '2', '5'])
    assert.throws(() => extractSyft(archive('syft', type)), /no regular/);
  assert.throws(() => extractSyft(archive('syft', '0', 100_000)), /Invalid tool archive/);
});
