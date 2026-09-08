import assert from 'node:assert/strict';
import test from 'node:test';
import { checkpointEvidence, checkpointFix, requiredCheckpoints } from './checkpoints.mjs';

const identity = {
  checkpoint: 'CP1',
  source: 'a'.repeat(40),
  artifacts: [],
  decisions: 'b'.repeat(64),
};
const issue = { state: 'open', body: JSON.stringify(identity) };
const approval = (overrides = {}) => ({
  id: 1,
  user: { type: 'User', login: 'maintainer' },
  body: JSON.stringify({ ...identity, status: 'approved', ...overrides }),
});

test('hard gates follow producer stages and CP7 additionally guards publication', () => {
  assert.deepEqual(requiredCheckpoints(1), []);
  assert.deepEqual(requiredCheckpoints(2), []);
  assert.deepEqual(requiredCheckpoints(3), ['CP1']);
  assert.deepEqual(requiredCheckpoints(6), ['CP1']);
  assert.deepEqual(requiredCheckpoints(7), ['CP1', 'CP3']);
  assert.deepEqual(requiredCheckpoints(11), ['CP1', 'CP3']);
  assert.deepEqual(requiredCheckpoints(12), ['CP1', 'CP3', 'CP5']);
  assert.deepEqual(requiredCheckpoints(14), ['CP1', 'CP3', 'CP5']);
  assert.deepEqual(requiredCheckpoints(14, 'publication'), ['CP1', 'CP3', 'CP5', 'CP7']);
  assert.deepEqual(requiredCheckpoints(15), ['CP1', 'CP3', 'CP5', 'CP7']);
  for (const stage of [0, 16, null, '3']) assert.throws(() => requiredCheckpoints(stage));
});

test('approval requires a configured person and exact current source, artifacts and decisions', () => {
  assert.deepEqual(checkpointEvidence(issue, [approval()], ['maintainer'], 'CP1'), identity);
  for (const comments of [
    [],
    [approval({ source: 'c'.repeat(40) })],
    [approval({ status: 'rejected' })],
    [approval({ decisions: 'c'.repeat(64) })],
    [approval({ artifacts: ['d'.repeat(64)] })],
    [{ ...approval(), user: { type: 'Bot', login: 'maintainer' } }],
    [{ ...approval(), body: '```json\n' + approval().body + '\n```' }],
  ])
    assert.throws(() => checkpointEvidence(issue, comments, ['maintainer'], 'CP1'));
  assert.throws(() => checkpointEvidence(issue, [approval()], [], 'CP1'));
  assert.throws(() => checkpointEvidence(issue, [approval()], ['someone-else'], 'CP1'));
  assert.throws(() =>
    checkpointEvidence(
      { ...issue, body: JSON.stringify({ ...identity, source: 'd'.repeat(40) }) },
      [approval()],
      ['maintainer'],
      'CP1',
    ),
  );
  assert.throws(() =>
    checkpointEvidence(
      issue,
      [approval(), { ...approval({ status: 'revoked' }), id: 2 }],
      ['maintainer'],
      'CP1',
    ),
  );
  assert.throws(() =>
    checkpointEvidence(issue, [approval()], ['maintainer'], 'CP1', 'publication'),
  );
});

test('checkpoint repair exception is current-head, maintainer-controlled and never publication approval', () => {
  const comments = [approval({ kind: 'checkpoint-fix' })];
  assert.equal(checkpointFix(comments, ['maintainer'], 'CP1', identity.source), true);
  assert.equal(checkpointFix(comments, ['maintainer'], 'CP1', 'c'.repeat(40)), false);
  assert.equal(
    checkpointFix(comments, ['maintainer'], 'CP1', identity.source, 'publication'),
    false,
  );
  assert.equal(checkpointFix(comments, ['someone-else'], 'CP1', identity.source), false);
});
