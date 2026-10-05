import { expect, it } from 'vitest';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import { readFor } from './useSessionTranscript';

const status: SessionSourceStatus = {
  state: 'loaded',
  generation: { generation: 'indexed', appended: false },
  sources: 1,
  dropped_records: 0,
  gaps: [],
  records: [],
};

it('hands over an answer only for the session being rendered', () => {
  // Effects run after paint, so on the first committed frame with a new
  // session in the address this hook still holds the previous session's
  // answer. The guard is checked during render — which is when the mismatch
  // exists — rather than reset in an effect, which is by definition too late.
  const held = { phase: 'read', status, of: 'session-a' } as const;
  expect(readFor(held, 'session-a')).toBe(held);
  expect(readFor(held, 'session-b')).toEqual({ phase: 'loading' });
  // A failed read is the same: it says something about one session, and
  // nothing about the next one.
  expect(readFor({ phase: 'failed', of: 'session-a' }, 'session-b')).toEqual({ phase: 'loading' });
  expect(readFor({ phase: 'failed', of: 'session-a' }, 'session-a')).toEqual({
    phase: 'failed',
    of: 'session-a',
  });
});

it('holds nothing for an address that names no session', () => {
  expect(readFor({ phase: 'read', status, of: 'session-a' }, undefined)).toEqual({
    phase: 'loading',
  });
});
