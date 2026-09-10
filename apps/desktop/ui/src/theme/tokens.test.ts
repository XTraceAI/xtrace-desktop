// @vitest-environment node
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, it } from 'vitest';

const css = readFileSync(resolve('src/styles/tokens.css'), 'utf8');
const contract = JSON.parse(
  readFileSync(resolve('../../../design/token-contract.json'), 'utf8'),
) as Record<string, Record<string, string>>;
const normalize = (value: string) => value.replace(/\s+/g, '');
function declarations(selector: RegExp) {
  const block = css.match(selector)?.[1];
  if (!block) throw new Error('Missing token block');
  return Object.fromEntries(
    [...block.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((match) => [match[1], normalize(match[2])]),
  );
}

it('dark/light define identical keys and match every normative contract value', () => {
  const dark = declarations(/:root,\s*\[data-theme=['"]dark['"]\]\s*{([^}]+)}/);
  const light = declarations(/\[data-theme=['"]light['"]\]\s*{([^}]+)}/);
  expect(Object.keys(dark).sort()).toEqual(Object.keys(light).sort());
  for (const [name, values] of Object.entries({
    dark,
    light,
    shared: declarations(/:root\s*{([^}]+)}/),
  })) {
    expect(values).toEqual(
      Object.fromEntries(
        Object.entries(contract[name]).map(([key, value]) => [key, normalize(value)]),
      ),
    );
  }
});

it('font faces are local, swap, and include each required weight in both packaged formats', () => {
  for (const [font, weights] of Object.entries({
    manrope: [400, 500, 600, 700, 800],
    'geist-mono': [400, 500, 600],
  })) {
    for (const weight of weights) {
      expect(css).toContain(`/fonts/${font}-${weight}.woff2`);
      for (const extension of ['ttf', 'woff2']) {
        const data = readFileSync(resolve(`public/fonts/${font}-${weight}.${extension}`));
        expect(data.subarray(0, 4)).toEqual(
          extension === 'woff2' ? Buffer.from('wOF2') : Buffer.from([0, 1, 0, 0]),
        );
      }
    }
  }
  expect(css.match(/font-display: swap/g)).toHaveLength(8);
  expect(css).not.toMatch(/url\(['"]?https?:/);
});
