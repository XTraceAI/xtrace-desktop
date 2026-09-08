import assert from 'node:assert/strict';
import test from 'node:test';
import { acceptedLicense, renderNotices } from './licenses.mjs';

test('SPDX policy evaluates alternatives, obligations and exceptions structurally', () => {
  for (const expression of [
    'MIT',
    'MPL-2.0',
    'Zlib',
    'Unicode-3.0',
    '(MPL-2.0 AND MIT)',
    '(MIT OR Apache-2.0)',
    '(MIT AND BSD-3-Clause)',
    '(GPL-3.0-only OR MIT)',
  ])
    assert.equal(acceptedLicense(expression), true, expression);
  for (const expression of [
    'GPL-3.0-only',
    'AGPL-3.0-only',
    '(MPL-2.0 AND GPL-3.0-only)',
    'SSPL-1.0',
    '(MIT AND GPL-3.0-only)',
    'MIT WITH LLVM-exception',
    'MIT-ish',
    '',
    'MIT OR',
  ])
    assert.equal(acceptedLicense(expression), false, expression);
});

test('notices preserve distinct copyright texts and reproduce deterministically', () => {
  const a = {
    package: 'npm: one@1.0.0',
    license: 'MIT',
    text: 'Copyright One\nPermission granted.',
  };
  const b = { ...a, package: 'npm: two@1.0.0', text: 'Copyright Two\nPermission granted.' };
  const output = renderNotices([a, b, a]);
  assert.equal(output, renderNotices([b, a]));
  assert.match(output, /Copyright One/);
  assert.match(output, /Copyright Two/);
  assert.notEqual(output, renderNotices([a]));
  assert.throws(() => renderNotices([{ ...a, text: '' }]), /missing/);
  assert.throws(() => renderNotices([{ ...a, license: 'AGPL-3.0-only' }]), /unsupported/);
  assert.throws(() => renderNotices([]), /No dependency/);
});

test('MPL notices link the exact covered crate source version', () => {
  const output = renderNotices([
    { package: 'cargo: example@1.2.3', license: 'MPL-2.0', text: 'Covered license text' },
  ]);
  assert.match(output, /https:\/\/crates.io\/api\/v1\/crates\/example\/1.2.3\/download/);
  assert.match(output, /docs\/DEPENDENCY_LICENSES.md/);
});
