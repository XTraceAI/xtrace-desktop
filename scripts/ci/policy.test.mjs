import assert from 'node:assert/strict';
import test from 'node:test';
import { requireSuccessfulJobs, validateDco } from './policy.mjs';

const commit = {
  sha: 'a'.repeat(40),
  author: { name: 'Test Contributor', email: 'contributor@example.com' },
  message: 'Add synthetic test\n\nSigned-off-by: Test Contributor <contributor@example.com>',
};

test('DCO requires a final trailer matching the actual source author', () => {
  validateDco(commit);
  validateDco({ ...commit, message: commit.message.replace('@example.com', '@EXAMPLE.COM') });
  for (const message of [
    'Unsigned change',
    'Signed-off-by: Test Contributor <contributor@example.com>',
    `${commit.message}\n\nExample only`,
    'Change\n\nSigned-off-by: Another Contributor <another@example.com>',
    'Change\n\n> Signed-off-by: Test Contributor <contributor@example.com>',
    'Change\n\n```\nSigned-off-by: Test Contributor <contributor@example.com>\n```',
  ]) {
    assert.throws(() => validateDco({ ...commit, message }), /sign-off/);
  }
  assert.throws(() => validateDco({ ...commit, author: null }), /Author metadata/);
  assert.throws(() => validateDco({ ...commit, sha: 'unknown' }), /Commit metadata/);
});

test('DCO allows multiple valid trailers but does not substitute committer identity', () => {
  validateDco({ ...commit, message: `${commit.message}\nordinary text` });
  validateDco({
    ...commit,
    message: `${commit.message}\nReviewed-by: Reviewer <reviewer@example.com>`,
  });
  assert.throws(
    () =>
      validateDco({ ...commit, author: { name: 'Different Author', email: 'other@example.com' } }),
    /Author-matching/,
  );
});

test('aggregate fails on failure, cancellation, missing jobs and unapproved skips', () => {
  const required = ['rust', 'ui', 'browser'];
  const good = Object.fromEntries(required.map((name) => [name, { result: 'success' }]));
  requireSuccessfulJobs(good, required);
  for (const result of ['failure', 'cancelled', 'skipped', 'in_progress', undefined]) {
    for (const name of required) {
      assert.throws(
        () => requireSuccessfulJobs({ ...good, [name]: { result } }, required),
        /did not succeed/,
      );
    }
  }
  assert.throws(() => requireSuccessfulJobs({}, required), /did not succeed/);
  assert.throws(() => requireSuccessfulJobs(good, []), /policy/);
});

test('only explicitly inapplicable jobs may be skipped, never failed', () => {
  requireSuccessfulJobs({ optional: { result: 'skipped' } }, ['optional'], ['optional']);
  assert.throws(
    () => requireSuccessfulJobs({ optional: { result: 'failure' } }, ['optional'], ['optional']),
    /did not succeed/,
  );
  assert.throws(() => requireSuccessfulJobs({}, ['rust'], ['other']), /outside/);
});
