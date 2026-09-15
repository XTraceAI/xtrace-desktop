import assert from 'node:assert/strict';
import { readFile, readdir, access } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import test from 'node:test';
import { Parser } from 'commonmark';

const root = resolve(import.meta.dirname, '../..');
const guides = [
  'README.md',
  'DEVELOPING.md',
  'docs/DEVELOPING.md',
  'docs/DESIGN.md',
  'docs/NATIVE-DEVELOPMENT.md',
];

test('contributor guide links resolve and pnpm commands exist', async () => {
  const builtins = new Set(['install', 'exec', '--version']);
  for (const guide of guides) {
    const text = await readFile(resolve(root, guide), 'utf8');
    const walker = new Parser().parse(text).walker();
    const code = [];
    let event;
    while ((event = walker.next())) {
      if (!event.entering) continue;
      if (['code', 'code_block'].includes(event.node.type)) code.push(event.node.literal);
      if (event.node.type !== 'link') continue;
      const target = event.node.destination.split('#')[0];
      if (!target || /^[a-z]+:/i.test(target)) continue;
      await access(resolve(root, dirname(guide), target));
    }
    for (const match of code.join('\n').matchAll(/\bpnpm (?:--dir ([\w/.-]+) )?([\w:-]+)/g)) {
      const [, directory = '.', command] = match;
      if (builtins.has(command)) continue;
      const pkg = JSON.parse(await readFile(resolve(root, directory, 'package.json'), 'utf8'));
      assert.ok(pkg.scripts[command], `${guide}: unknown pnpm command ${command}`);
    }
  }
});

test('architecture covers every workspace crate and contributor template retains DCO', async () => {
  const architecture = await readFile(resolve(root, 'docs/ARCHITECTURE.md'), 'utf8');
  for (const entry of await readdir(resolve(root, 'crates'), { withFileTypes: true })) {
    if (entry.isDirectory()) assert.ok(architecture.includes(`crates/${entry.name}/`), entry.name);
  }
  const template = await readFile(resolve(root, '.github/pull_request_template.md'), 'utf8');
  assert.match(template, /DCO/);
  assert.match(template, /acceptance contract/);
});
