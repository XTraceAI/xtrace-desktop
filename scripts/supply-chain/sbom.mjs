import { execFileSync } from 'node:child_process';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import Ajv from 'ajv';
import addFormats from 'ajv-formats';
import addInternationalFormats from 'ajv-formats-draft2019';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
let validator;

export async function validateSbom(document) {
  if (!validator) {
    const ajv = new Ajv({ strict: false, allErrors: false });
    addFormats(ajv);
    addInternationalFormats(ajv, { formats: ['idn-email'] });
    const uriReference = ajv.formats['uri-reference'];
    // IRI references allow unregistered schemes (including Cargo's registry+https).
    // Convert Unicode to URI escapes before the standard URI-reference check.
    ajv.addFormat('iri-reference', (value) => {
      if (
        [...value].some(
          (character) => character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127,
        ) ||
        /[<>"{}|\\^`]|%(?![0-9a-f]{2})/i.test(value)
      )
        return false;
      try {
        const encoded = encodeURI(value)
          .replace(/%25([0-9a-f]{2})/gi, '%$1')
          .replace(/%5B/g, '[')
          .replace(/%5D/g, ']');
        return uriReference instanceof RegExp ? uriReference.test(encoded) : uriReference(encoded);
      } catch {
        return false;
      }
    });
    for (const name of ['spdx', 'jsf-0.82', 'bom-1.5']) {
      const schema = JSON.parse(
        await readFile(new URL(`./schema/${name}.schema.json`, import.meta.url), 'utf8'),
      );
      ajv.addSchema(schema);
    }
    validator = ajv.getSchema('http://cyclonedx.org/schema/bom-1.5.schema.json');
  }
  if (!validator(document)) throw new Error('SBOM does not match the CycloneDX 1.5 schema.');
  const purls = document.components?.map((component) => component.purl).filter(Boolean) ?? [];
  if (
    !purls.some((purl) => purl.startsWith('pkg:cargo/')) ||
    !purls.some((purl) => purl.startsWith('pkg:npm/'))
  )
    throw new Error('SBOM must contain both Rust and npm dependencies.');
  if (new Set(purls).size !== purls.length)
    throw new Error('SBOM contains duplicate package URLs.');
  // These are the only files read by the explicitly selected lockfile catalogers.
  const locations = new Set(['/Cargo.lock', '/pnpm-lock.yaml']);
  const inspect = (value) => {
    if (typeof value === 'string') {
      if (
        (value.startsWith('/') && !locations.has(value)) ||
        /(?:\/(?:Users|home|var|private|tmp)\/|^[A-Za-z]:[\\/]|^\\\\|file:\/\/)/i.test(value)
      )
        throw new Error('SBOM contains a host filesystem path.');
    } else if (value && typeof value === 'object') {
      if (/^syft:location:\d+:path$/.test(value.name) && !locations.has(value.value))
        throw new Error('SBOM contains an unexpected filesystem location.');
      for (const child of Object.values(value)) inspect(child);
    }
  };
  inspect(document);
  return { components: document.components.length, packages: purls.length };
}

async function main() {
  if (process.argv.length !== 2) throw new Error('SBOM generation takes no arguments.');
  const tool = process.env.SYFT || 'syft';
  const options = {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: 40 * 1024 * 1024,
    timeout: 180_000,
    stdio: ['ignore', 'pipe', 'pipe'],
  };
  const version = JSON.parse(execFileSync(tool, ['version', '-o', 'json'], options));
  if (version.version !== '1.51.1') throw new Error('Install the pinned Syft 1.51.1 tool.');
  const document = JSON.parse(
    execFileSync(
      tool,
      [
        'scan',
        'dir:.',
        '--override-default-catalogers',
        'rust-cargo-lock-cataloger,javascript-lock-cataloger',
        '--select-catalogers=-file',
        '--exclude',
        '**/node_modules/**',
        '--exclude',
        '**/target/**',
        '--exclude',
        '**/.git/**',
        '--source-name',
        'xtrace-desktop',
        '--source-version',
        '0.1.0',
        '--output',
        'cyclonedx-json@1.5',
      ],
      options,
    ),
  );
  const counts = await validateSbom(document);
  await mkdir(resolve(root, 'artifacts'), { recursive: true });
  await writeFile(
    resolve(root, 'artifacts/sbom.cdx.json'),
    `${JSON.stringify(document, null, 2)}\n`,
  );
  console.log(
    `CycloneDX 1.5 validation passed: ${counts.components} components, ${counts.packages} package URLs.`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(() => {
    // Subprocess errors and schema details can contain machine-local paths.
    console.error('SBOM generation or validation failed; inspect the tool locally.');
    process.exitCode = 1;
  });
}
