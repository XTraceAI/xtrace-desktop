import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lintHex } from './lint-hex.mjs';

test('reports nested component drift with location and permits only exact token/test paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'xtrace-hex-'));
  try {
    await mkdir(join(root, 'styles'));
    await mkdir(join(root, 'kit'));
    await writeFile(join(root, 'kit', 'Button.tsx'), 'const color =\n  "#123456";\n');
    await writeFile(join(root, 'styles', 'tokens.css'), ':root { --ink: #fff; }');
    await writeFile(join(root, 'kit', 'Button.test.tsx'), 'const fixture = "#aabbcc";');
    await writeFile(join(root, 'kit', 'tokens.css'), 'button { color: #ABC; }');
    const findings = await lintHex(root);
    assert.equal(findings.length, 2);
    assert.ok(findings.some((line) => line.startsWith('kit/Button.tsx:2:4:')));
    assert.ok(findings.some((line) => line.startsWith('kit/tokens.css:1:17:')));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
