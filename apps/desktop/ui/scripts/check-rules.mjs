import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';

// Run when updating the technical excerpt contract; the full source is never copied.
const args = process.argv.slice(2);
if (args.length !== 2 || args[0] !== '--spec') {
  process.stderr.write('Usage: node scripts/check-rules.mjs --spec APPROVED_SPEC.md\n');
  process.exit(2);
}
try {
  const contract = JSON.parse(
    await readFile(new URL('../../../../design/rule-contract.json', import.meta.url), 'utf8'),
  );
  const source = await readFile(args[1], 'utf8');
  const selected = {};
  for (const line of source.split('\n')) {
    const match = line.match(/^- \*\*((?:M|R|P|U|O|C)-\d{2}[a-z]?)(.*)/);
    if (match && Object.hasOwn(contract.rules, match[1])) {
      if (Object.hasOwn(selected, match[1])) throw new Error('Duplicate definition');
      selected[match[1]] = match[2]
        .replaceAll('**', '')
        .replaceAll('`', '')
        .replaceAll('*records*', 'records')
        .trim();
    }
  }
  const before = (id, marker) => {
    if (!selected[id]?.includes(marker)) throw new Error('Source excerpt boundary changed');
    selected[id] = selected[id].split(marker)[0];
  };
  before('M-01', ' (Found on real data:');
  before('M-02', ' (Staging dashboard filter,');
  selected['M-03'] = selected['M-03']?.replace(/ \(real data:[^)]+\)/, '');
  const clause = 'Surfaces are discovered, never enumerated in code';
  if (!selected['O-11']?.includes(clause)) throw new Error('Source clause changed');
  before('O-11', ' Every host stamps');
  selected['O-11'] += ` ${clause}`;
  before('O-12', ' Verified on a real machine');
  const mismatches = Object.keys(contract.rules).filter(
    (id) => selected[id] !== contract.rules[id],
  );
  if (mismatches.length) {
    process.stderr.write(`Rule excerpt mismatch: ${mismatches.join(', ')}\n`);
    process.exitCode = 1;
  } else {
    const report = {
      scope: 'FND-07a normative excerpt comparison; no full SPEC copy',
      matched: Object.keys(selected).length,
      rules: Object.entries(selected)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([id, text]) => ({
          id,
          matched: true,
          textSha256: createHash('sha256').update(text).digest('hex'),
        })),
    };
    await writeFile(
      new URL('../../../../docs/acceptance/FND-07a-rule-coverage.json', import.meta.url),
      JSON.stringify(report, null, 2) + '\n',
    );
    process.stdout.write(`${report.matched} rule excerpts match the supplied SPEC.\n`);
  }
} catch {
  process.stderr.write(
    'Rule comparison failed: source or excerpt contract unavailable or changed.\n',
  );
  process.exitCode = 2;
}
