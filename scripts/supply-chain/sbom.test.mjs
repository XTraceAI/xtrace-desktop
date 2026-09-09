import assert from 'node:assert/strict';
import test from 'node:test';
import { validateSbom } from './sbom.mjs';

const fixture = () => ({
  bomFormat: 'CycloneDX',
  specVersion: '1.5',
  version: 1,
  components: [
    {
      type: 'library',
      name: 'synthetic-rust',
      version: '1.0.0',
      purl: 'pkg:cargo/synthetic-rust@1.0.0',
    },
    {
      type: 'library',
      name: 'synthetic-npm',
      version: '1.0.0',
      purl: 'pkg:npm/synthetic-npm@1.0.0',
    },
  ],
});

test('offline official schema validates both ecosystems and rejects incomplete artifacts', async () => {
  assert.deepEqual(await validateSbom(fixture()), { components: 2, packages: 2 });
  const invalid = fixture();
  invalid.components[0].type = 'invalid-component-type';
  await assert.rejects(validateSbom(invalid), /schema/);
  await assert.rejects(validateSbom({ ...fixture(), components: [] }), /both Rust and npm/);
  const duplicate = fixture();
  duplicate.components.push({ ...duplicate.components[0], name: 'same-package-second-name' });
  await assert.rejects(validateSbom(duplicate), /duplicate/);
  const privatePath = fixture();
  privatePath.components[0].name = '/Users/synthetic/work/Cargo.lock';
  await assert.rejects(validateSbom(privatePath), /filesystem/);
});

test('only cataloged lockfile locations can enter the uploaded artifact', async () => {
  for (const path of ['/Cargo.lock', '/pnpm-lock.yaml']) {
    const document = fixture();
    document.components[0].properties = [{ name: 'syft:location:0:path', value: path }];
    await validateSbom(document);
  }
  for (const path of [
    '/var/folders/synthetic-private/work/Cargo.lock',
    '/tmp/synthetic/Cargo.lock',
    '/private/tmp/work/pnpm-lock.yaml',
    '/workspace/private/Cargo.lock',
    '../outside/Cargo.lock',
    'C:\\private\\Cargo.lock',
  ]) {
    const document = fixture();
    document.components[0].properties = [{ name: 'syft:location:0:path', value: path }];
    await assert.rejects(validateSbom(document), /filesystem/);
  }
});

test('IRI references allow package schemes and Unicode but reject malformed values', async () => {
  for (const url of [
    'registry+https://github.com/rust-lang/crates.io-index',
    'https://例え.jp/x',
    'mailto:a@example.com',
  ]) {
    const document = fixture();
    document.components[0].externalReferences = [{ type: 'distribution', url }];
    await validateSbom(document);
  }
  for (const url of ['https://ex ample.com', '%ZZ', 'https://[invalid']) {
    const document = fixture();
    document.components[0].externalReferences = [{ type: 'distribution', url }];
    await assert.rejects(validateSbom(document), /schema/);
  }
});
