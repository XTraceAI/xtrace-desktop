import { readdir, readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import ts from 'typescript';
import { ESLint } from 'eslint';
import { expect, it } from 'vitest';
import * as kit from '../kit';
import { stories } from './stories';

const ui = resolve(import.meta.dirname, '../..');
it('covers every exported kit component with a unique declared story and matching barrel', async () => {
  const components: string[] = [];
  for (const file of await readdir(join(ui, 'src/kit'))) {
    if (!file.endsWith('.tsx') || file.includes('.test.')) continue;
    const source = ts.createSourceFile(
      file,
      await readFile(join(ui, 'src/kit', file), 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    for (const node of source.statements) {
      if (
        ts.isFunctionDeclaration(node) &&
        node.modifiers?.some((value) => value.kind === ts.SyntaxKind.ExportKeyword) &&
        node.name &&
        /^[A-Z]/.test(node.name.text)
      )
        components.push(node.name.text);
    }
  }
  expect(Object.keys(kit).sort()).toEqual(components.sort());
  expect([...new Set(stories.flatMap((story) => story.components))].sort()).toEqual(
    components.sort(),
  );
  expect(new Set(stories.map(({ id }) => id)).size).toBe(stories.length);
  for (const { id, size } of stories) {
    expect(id).toMatch(/^[a-z0-9-]+\/[a-z0-9-]+$/);
    expect(size.every((value) => Number.isInteger(value) && value > 0)).toBe(true);
  }
});
it('keeps the generated review inventory in sync with the story registry', async () => {
  const inventory = stories.map(({ id, components, size }) => ({ id, components, size }));
  const path = join(ui, 'src/gallery/inventory.json');
  if (process.env.UPDATE_GALLERY_INVENTORY === '1')
    await writeFile(path, JSON.stringify(inventory, null, 2) + '\n');
  expect(JSON.parse(await readFile(path, 'utf8'))).toEqual(inventory);
});

it('rejects a real scratch product import through the repository ESLint boundary', async () => {
  const root = await mkdtemp(join(ui, 'src/boundary-negative-'));
  try {
    const file = join(root, 'example.ts');
    await writeFile(file, "import { sample } from '../gallery/sample'; void sample;\n");
    const lint = new ESLint({ cwd: resolve(ui, '../../..') });
    const [result] = await lint.lintFiles([file]);
    expect(result.messages.map(({ ruleId }) => ruleId)).toContain('gallery/boundary');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
it('rejects re-exports, dynamic imports, require and globs including unguarded entry imports', async () => {
  const lint = new ESLint({ cwd: resolve(ui, '../../..') });
  for (const source of [
    "export * from '../gallery/sample';",
    "void import('../gallery/sample');",
    "void require('../gallery/' + 'sample');",
    "void import.meta.glob('../gallery/*.ts');",
    'void import(`../gallery/sample`);',
    'void import(`../gallery/${name}.tsx`);',
    "void import.meta.glob('../**/*.tsx');",
  ]) {
    const [result] = await lint.lintText(source, { filePath: join(ui, 'src/app/negative.ts') });
    expect(result.messages.some(({ ruleId }) => ruleId === 'gallery/boundary')).toBe(true);
  }
  const [entry] = await lint.lintText("void import('../gallery/Gallery');", {
    filePath: join(ui, 'src/app/AppRoutes.tsx'),
  });
  expect(entry.messages.some(({ ruleId }) => ruleId === 'gallery/boundary')).toBe(true);
  const [real] = await lint.lintFiles([join(ui, 'src/app/AppRoutes.tsx')]);
  expect(real.errorCount).toBe(0);
});
