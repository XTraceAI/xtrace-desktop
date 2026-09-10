import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync, spawnSync } from 'node:child_process';
import { webkit } from '@playwright/test';
import {
  approvedRoot,
  captureOptions,
  currentInputs,
  digest,
  diffOptions,
  imageRecords,
  readJson,
  repoRoot,
  reviewPage,
  uiRoot,
  validateManifest,
  writeJson,
} from './parity-contract.mjs';

const require = createRequire(import.meta.url);
const [mode = 'compare', candidate, expectedDigest, ...extra] = process.argv.slice(2);
assert.ok(
  ['compare', 'capture', 'review', 'probe', 'accept'].includes(mode),
  'Unknown parity command',
);
assert.equal(extra.length, 0, 'Unexpected arguments');
assert.ok(mode === 'accept' || !expectedDigest, 'Unexpected arguments');
assert.ok(
  !['capture', 'compare'].includes(mode) || !candidate,
  'This command accepts no arguments',
);
assert.equal(
  process.platform,
  'darwin',
  'Visual comparisons run locally on the recorded macOS runner',
);
const git = (...args) => execFileSync('git', args, { cwd: repoRoot, encoding: 'utf8' }).trim();
const cleanHead = () => {
  assert.equal(
    git('status', '--porcelain'),
    '',
    'Commit source changes before capturing or accepting baselines',
  );
  return git('rev-parse', 'HEAD');
};
const head = git('rev-parse', 'HEAD');
const inputs = currentInputs();
const browser = await webkit.launch();
const environment = {
  platform: process.platform,
  arch: os.arch(),
  macOS: execFileSync('sw_vers', ['-productVersion'], { encoding: 'utf8' }).trim(),
  playwright: require('@playwright/test/package.json').version,
  webkit: browser.version(),
  revision: webkit.executablePath().match(/webkit-(\d+)/)?.[1],
  ...captureOptions,
  ...diffOptions,
};
await browser.close();
assert.ok(environment.revision, 'Could not identify the pinned WebKit revision');
const artifacts = path.join(repoRoot, 'artifacts/parity');
fs.mkdirSync(artifacts, { recursive: true });
const output = fs.mkdtempSync(path.join(artifacts, `${mode}-${head.slice(0, 8)}-`));

function run(directory, outputDirectory, capture = false, probe = false) {
  const env = {
    ...process.env,
    XTRACE_PARITY_MODE: capture ? 'capture' : 'compare',
    XTRACE_PARITY_BASELINES: path.join(directory, 'images'),
    XTRACE_PARITY_OUTPUT: outputDirectory,
    XTRACE_PARITY_PROBE: probe ? 'visible-drift' : '',
  };
  delete env.GH_TOKEN;
  delete env.GITHUB_TOKEN;
  const args = ['exec', 'playwright', 'test', '--config', 'playwright.parity.config.mjs'];
  if (probe) args.push('--grep', 'sidebar-active-dashboard-dark');
  const result = spawnSync('pnpm', args, { cwd: uiRoot, env, stdio: 'inherit' });
  if (result.error) throw result.error;
  return result.status;
}

if (mode === 'capture') {
  cleanHead();
  fs.mkdirSync(path.join(output, 'images'));
  assert.equal(run(output, output, true), 0, 'Candidate capture failed');
  assert.equal(cleanHead(), head, 'Source changed during capture');
  const manifest = {
    version: 1,
    sourceCommit: head,
    inputs,
    environment,
    images: imageRecords(output, inputs.comparisons),
    review: null,
  };
  writeJson(path.join(output, 'manifest.json'), manifest);
  assert.equal(run(output, path.join(output, 'repeat')), 0, 'Independent repeat capture differed');
  assert.deepEqual(
    imageRecords(output, inputs.comparisons),
    manifest.images,
    'Repeat modified candidate images',
  );
  assert.equal(cleanHead(), head, 'Source changed during repeat verification');
  if (fs.existsSync(approvedRoot)) {
    fs.cpSync(approvedRoot, path.join(output, 'previous'), { recursive: true });
    // Comparison artifacts support review even when a deliberate font or map
    // change invalidates the old manifest. This run is not acceptance evidence.
    run(approvedRoot, path.join(output, 'changes'));
    const report = readJson(path.join(output, 'changes/results.json'));
    assert.equal(report.errors.length, 0, 'Could not generate baseline update evidence');
    assert.deepEqual(imageRecords(output, inputs.comparisons), manifest.images);
    assert.equal(cleanHead(), head, 'Source changed during update comparison');
  }
  reviewPage(output, manifest);
  console.log(`Review candidates: ${output}\nReview digest: ${digest(manifest)}`);
} else {
  const directory = candidate ? path.resolve(candidate) : approvedRoot;
  assert.ok(!['review', 'accept'].includes(mode) || candidate, 'Supply a candidate directory');
  const manifest = validateManifest(directory, inputs, environment, mode === 'compare');
  if (mode === 'accept') {
    cleanHead();
    assert.equal(manifest.review, null, 'Supply an unapproved candidate set');
    assert.equal(expectedDigest, digest(manifest), 'Review digest does not match these candidates');
    // Capture provenance remains valid after a squash merge. Only promotion
    // needs the original commit locally to verify unchanged rendering inputs;
    // normal comparison uses the committed manifest and PNGs independently.
    git(
      'diff',
      '--exit-code',
      manifest.sourceCommit,
      '--',
      'apps/desktop/ui/src',
      'apps/desktop/ui/public',
      'apps/desktop/ui/vite.config.ts',
      'apps/desktop/ui/e2e/parity',
      'apps/desktop/ui/playwright.parity.config.mjs',
      'apps/desktop/ui/scripts/parity.mjs',
      'apps/desktop/ui/scripts/parity-contract.mjs',
      'pnpm-lock.yaml',
    );
    const prepared = path.join(output, 'approved');
    const previous = path.join(output, 'previous');
    fs.mkdirSync(path.join(prepared, 'images'), { recursive: true });
    for (const name of Object.keys(manifest.images))
      fs.copyFileSync(path.join(directory, 'images', name), path.join(prepared, 'images', name));
    writeJson(path.join(prepared, 'manifest.json'), {
      ...manifest,
      review: { digest: expectedDigest },
    });
    if (fs.existsSync(approvedRoot)) fs.renameSync(approvedRoot, previous);
    try {
      fs.renameSync(prepared, approvedRoot);
    } catch (error) {
      if (fs.existsSync(previous)) fs.renameSync(previous, approvedRoot);
      throw error;
    }
    console.log(
      'Reviewed baselines installed in the working tree. Inspect the diff and commit them.',
    );
  } else {
    const code = run(directory, output, false, mode === 'probe');
    assert.deepEqual(
      imageRecords(directory, inputs.comparisons),
      manifest.images,
      'Comparison modified baseline images',
    );
    if (mode === 'probe') {
      const report = readJson(path.join(output, 'results.json'));
      assert.equal(code, 1, 'Deliberate drift was not rejected');
      assert.equal(report.stats.unexpected, 1, 'Probe must fail exactly one comparison');
      assert.equal(report.errors.length, 0, 'Probe failed before comparison');
      const files = fs.readdirSync(path.join(output, 'test-results'), { recursive: true });
      for (const suffix of ['-expected.png', '-actual.png', '-diff.png'])
        assert.ok(
          files.some((file) => file.endsWith(suffix)),
          'Probe did not emit before/after/diff images',
        );
      console.log(`Visible drift rejected; baseline images unchanged. Evidence: ${output}`);
    } else {
      assert.equal(code, 0, 'Visual comparison failed');
      console.log(`40 comparisons passed. Evidence: ${output}`);
    }
  }
}
