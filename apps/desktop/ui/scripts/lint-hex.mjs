import { readdir, readFile } from 'node:fs/promises';
import { relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

export async function lintHex(root) {
  const findings = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) {
        const name = relative(root, path).split(sep).join('/');
        if (name === 'styles/tokens.css' || /\.test\.[^.]+$/.test(name)) continue;
        const lines = (await readFile(path, 'utf8')).split('\n');
        lines.forEach((line, index) => {
          const match = /#[0-9a-f]{3,8}\b/i.exec(line);
          if (match)
            findings.push(
              `${name}:${index + 1}:${match.index + 1}: raw hex color; use a design token`,
            );
        });
      }
    }
  }
  await visit(root);
  return findings;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const findings = await lintHex(fileURLToPath(new URL('../src', import.meta.url)));
    if (findings.length) {
      console.error(findings.join('\n'));
      process.exitCode = 1;
    } else console.log('Hex lint passed.');
  } catch {
    console.error('Hex lint could not read the source tree.');
    process.exitCode = 2;
  }
}
