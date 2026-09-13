import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { parsePin, readPin, verifyCheckout } from './plugin-pin.mjs';

const valid = {
  repository: 'https://github.com/XTraceAI/agent-plugins.git',
  commit: 'd7c94227cc9bd8ff46f933539ac65b478dd8f8ef',
  plugin_root: 'plugins/memhub',
  plugin_version: '0.55.0',
  reader_sources: {
    'plugins/memhub/scripts/readers_cli.py': 'b95348419605e61029447d10de818618c59c131d',
  },
};

test('the committed pin parses and names a full commit, root, version and reader sources', async () => {
  const pin = await readPin(process.cwd());
  assert.match(pin.commit, /^[0-9a-f]{40}$/);
  assert.equal(pin.plugin_root, 'plugins/memhub');
  assert.ok(Object.keys(pin.reader_sources).length >= 3);
  assert.deepEqual(parsePin(await readFile('.plugin-pin', 'utf8')), pin);
});

test('malformed pins are rejected with a specific reason', () => {
  const variants = [
    ['not json', /JSON object/],
    ['[]', /JSON object/],
    [JSON.stringify({ ...valid, extra: 1 }), /known keys/],
    [
      JSON.stringify({ ...valid, repository: 'https://example.com/x.git' }),
      /reviewed public producer/,
    ],
    [JSON.stringify({ ...valid, commit: 'd7c9422' }), /full SHA/],
    [JSON.stringify({ ...valid, plugin_root: '../memhub' }), /relative path/],
    [JSON.stringify({ ...valid, plugin_version: 'v0.55' }), /release version/],
    [JSON.stringify({ ...valid, reader_sources: {} }), /list reader sources/],
    [JSON.stringify({ ...valid, reader_sources: { 'a/../b': valid.commit } }), /object IDs/],
    [JSON.stringify({ ...valid, reader_sources: { 'plugins/x.py': 'abc' } }), /object IDs/],
  ];
  for (const [text, reason] of variants) assert.throws(() => parsePin(text), reason, text);
  assert.deepEqual(parsePin(JSON.stringify(valid)), valid);
});

test('a checkout passes only at the pinned commit with identical reader objects and a clean tree', () => {
  const fake =
    (head, sources, dirty = '') =>
    (args) => {
      if (args[0] === 'rev-parse' && args[1] === 'HEAD') return `${head}\n`;
      if (args[0] === 'rev-parse') {
        const path = args[2].slice('HEAD:'.length);
        if (!(path in sources)) throw new Error('fatal: Needed a single revision');
        return `${sources[path]}\n`;
      }
      if (args[0] === 'status') return dirty;
      throw new Error(`unexpected git ${args.join(' ')}`);
    };
  const path = 'plugins/memhub/scripts/readers_cli.py';
  assert.equal(
    verifyCheckout(valid, fake(valid.commit, { [path]: valid.reader_sources[path] })),
    valid.commit,
  );
  assert.throws(
    () => verifyCheckout(valid, fake('0'.repeat(40), { [path]: valid.reader_sources[path] })),
    /not at the pinned commit/,
  );
  assert.throws(
    () => verifyCheckout(valid, fake(valid.commit, { [path]: '1'.repeat(40) })),
    /differs/,
  );
  assert.throws(() => verifyCheckout(valid, fake(valid.commit, {})), /absent/);
  assert.throws(
    () =>
      verifyCheckout(valid, fake(valid.commit, { [path]: valid.reader_sources[path] }, ' M x\n')),
    /local modifications/,
  );
});
