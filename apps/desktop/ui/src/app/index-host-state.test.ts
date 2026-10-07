import { expect, it } from 'vitest';
import type { NativeHostState } from '../data/generated/NativeHostState';
import { hostScanText } from './index-host-state';

const states: NativeHostState[] = [
  'pending',
  'complete',
  'incomplete',
  'missing_source',
  'missing_runtime',
  'pin_mismatch',
  'reader_failed',
  'cancelled',
];
const problemTones = ['warning', 'danger'];

it.each(states.flatMap((state) => [true, false].map((flag) => [state, flag] as const)))(
  'colours %s as a problem exactly when the app flags it (flag %s)',
  (state, needs_attention) => {
    const { label, tone } = hostScanText({ state, needs_attention });
    expect(label).not.toBe('');
    expect(problemTones.includes(tone)).toBe(needs_attention);
  },
);

it('keeps the label from the state and the colour from the flag', () => {
  expect(hostScanText({ state: 'missing_source', needs_attention: false })).toEqual({
    label: 'No local history found',
    tone: 'meta',
  });
  expect(hostScanText({ state: 'reader_failed', needs_attention: true })).toEqual({
    label: 'Reader failed',
    tone: 'danger',
  });
  expect(hostScanText({ state: 'pending', needs_attention: false })).toEqual({
    label: 'Waiting to be read',
    tone: 'accent',
  });
  expect(hostScanText({ state: 'pending', needs_attention: true })).toEqual({
    label: 'Not read yet',
    tone: 'warning',
  });
});
