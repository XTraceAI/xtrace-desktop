import { mkdtempSync, mkdirSync, cpSync, writeFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const root = resolve(import.meta.dirname, '../../..');
const source = join(root, 'apps/desktop/ui/src/data/generated');
test('generated DTO gate rejects stale, missing and unexpected files', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'dto-parity-'));
  try {
    for (const mutation of ['none', 'stale', 'missing', 'extra', 'root-link']) {
      const candidate = join(temporary, mutation);
      cpSync(source, candidate, { recursive: true });
      if (mutation === 'stale') writeFileSync(join(candidate, 'DbCounts.ts'), 'stale');
      if (mutation === 'missing') rmSync(join(candidate, 'AppInfo.ts'));
      if (mutation === 'extra') writeFileSync(join(candidate, 'Extra.ts'), 'unexpected');
      if (mutation === 'root-link') {
        rmSync(candidate, { recursive: true });
        symlinkSync(source, candidate, 'dir');
      }
      const result = spawnSync('bash', ['scripts/ci/check-dto.sh', candidate], {
        cwd: root,
        encoding: 'utf8',
      });
      assert.equal(result.status, mutation === 'none' ? 0 : 1, `${mutation}: ${result.stderr}`);
      assert.match(
        result.stdout + result.stderr,
        mutation === 'none'
          ? /parity passed/
          : mutation === 'root-link'
            ? /symlink/
            : /missing, stale or unexpected/,
      );
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});

test('default generated directory rejects a symlinked repository parent before generation', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'dto-parent-'));
  try {
    const scripts = join(temporary, 'scripts/ci');
    mkdirSync(scripts, { recursive: true });
    cpSync(join(root, 'scripts/ci/check-dto.sh'), join(scripts, 'check-dto.sh'));
    const parent = join(temporary, 'apps/desktop/ui/src');
    mkdirSync(parent, { recursive: true });
    symlinkSync(resolve(source, '..'), join(parent, 'data'), 'dir');
    const result = spawnSync('bash', ['scripts/ci/check-dto.sh'], {
      cwd: temporary,
      encoding: 'utf8',
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /generated DTO parent is a symlink/);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});
