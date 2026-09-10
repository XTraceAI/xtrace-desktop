import assert from 'node:assert/strict';
import { join } from 'node:path';
import { test } from 'node:test';
import { smokeEnvironment } from './debug-bundle.mjs';

test('native smoke overrides live paths and clears inherited fixture selection', () => {
  const inherited = { XTRACE_DATA_DIR: '/existing-data', XTRACE_FIXTURE: 'F1', PATH: '/tools' };
  const env = smokeEnvironment('/owned-test-directory', inherited);
  assert.equal(env.XTRACE_DATA_DIR, join('/owned-test-directory', 'data'));
  assert.equal('XTRACE_FIXTURE' in env, false);
  assert.equal(env.PATH, inherited.PATH);
  assert.equal(env.GH_TOKEN, '');
  assert.equal(env.GITHUB_TOKEN, '');
  assert.equal(inherited.XTRACE_DATA_DIR, '/existing-data');
  assert.equal(inherited.XTRACE_FIXTURE, 'F1');
});
