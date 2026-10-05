import { expect, it } from 'vitest';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { SourceBlock } from '../../data/generated/SourceBlock';
import type { SourceRecord } from '../../data/generated/SourceRecord';
import { jumpUnavailableText, resolveJump } from './timeline-jump';
import { blockId } from './transcript-adapter';
import type { TranscriptRead } from './useSessionTranscript';

/** Synthetic throughout: identities and blocks invented for these cases. */
const CALLER = '22222222-2222-4222-8222-222222222222';

const call = (index: number, name = 'Read'): SourceBlock => ({
  kind: 'tool_call',
  index,
  name,
  call_id: `toolu_${index}`,
  input: null,
});
const text = (index: number): SourceBlock => ({ kind: 'text', index, text: 'said' });

const record = (id: string, blocks: SourceBlock[]): SourceRecord => ({
  id,
  role: 'assistant',
  at: null,
  blocks,
});

function loaded(records: SourceRecord[]): TranscriptRead {
  const status: SessionSourceStatus = {
    state: 'loaded',
    generation: { generation: 'indexed', appended: false },
    sources: 1,
    dropped_records: 0,
    gaps: [],
    records,
  };
  return { phase: 'read', status };
}

it('finds the one tool call at the position M-09 named, and composes the id the DOM carries', () => {
  const read = loaded([record(CALLER, [text(0), call(1, 'mcp__server__search')])]);
  const jump = resolveJump({ record_uuid: CALLER, block_index: 1 }, read);
  expect(jump).toEqual({
    kind: 'target',
    blockId: blockId(CALLER, 1),
    tool: 'mcp__server__search',
  });
});

it('shows nothing when M-09 could not say which call a stretch started with', () => {
  expect(resolveJump(null, loaded([record(CALLER, [call(0)])]))).toEqual({
    kind: 'unavailable',
    reason: 'no_locator',
  });
});

it('shows nothing while the transcript is still being read', () => {
  expect(resolveJump({ record_uuid: CALLER, block_index: 0 }, { phase: 'loading' })).toEqual({
    kind: 'unavailable',
    reason: 'transcript_loading',
  });
});

it('shows nothing when the transcript could not be read or may not be shown', () => {
  const locator = { record_uuid: CALLER, block_index: 0 };
  expect(resolveJump(locator, { phase: 'failed' })).toEqual({
    kind: 'unavailable',
    reason: 'transcript_unavailable',
  });
  expect(
    resolveJump(locator, {
      phase: 'read',
      status: { state: 'unavailable', reason: { reason: 'replaced' } },
    }),
  ).toEqual({ kind: 'unavailable', reason: 'transcript_unavailable' });
});

it('does not fall back to the record when the block is not in it', () => {
  // The record is here and the block is not. Jumping to the record would put
  // the reader somewhere M-09 never pointed.
  const read = loaded([record(CALLER, [text(0), call(1)])]);
  expect(resolveJump({ record_uuid: CALLER, block_index: 5 }, read)).toEqual({
    kind: 'unavailable',
    reason: 'not_in_transcript',
  });
});

it('does not look anywhere else when the record is not in the transcript', () => {
  const read = loaded([record('33333333-3333-4333-8333-333333333333', [call(0)])]);
  expect(resolveJump({ record_uuid: CALLER, block_index: 0 }, read)).toEqual({
    kind: 'unavailable',
    reason: 'not_in_transcript',
  });
});

it('refuses a position that names more than one block', () => {
  // A fork can carry two records under one UUID. The position then names two
  // blocks, and choosing one would be a guess.
  const twoCalls = loaded([record(CALLER, [call(0)]), record(CALLER, [call(0, 'Bash')])]);
  expect(resolveJump({ record_uuid: CALLER, block_index: 0 }, twoCalls)).toEqual({
    kind: 'unavailable',
    reason: 'ambiguous',
  });
  // Even when only one of them is a tool call: the id the transcript puts on
  // the node is shared, so the position is not a single place.
  const callAndText = loaded([record(CALLER, [call(0)]), record(CALLER, [text(0)])]);
  expect(resolveJump({ record_uuid: CALLER, block_index: 0 }, callAndText)).toEqual({
    kind: 'unavailable',
    reason: 'ambiguous',
  });
});

it('refuses a position whose one block is not a tool call', () => {
  for (const block of [
    text(0),
    { kind: 'thinking', index: 0, text: 'weighing' } as SourceBlock,
    { kind: 'tool_result', index: 0, call_id: 'toolu_0', failed: false, parts: [] } as SourceBlock,
    { kind: 'unsupported', index: 0, label: 'image' } as SourceBlock,
  ]) {
    expect(
      resolveJump({ record_uuid: CALLER, block_index: 0 }, loaded([record(CALLER, [block])])),
    ).toEqual({ kind: 'unavailable', reason: 'not_a_tool_call' });
  }
});

it('never substitutes the host’s call id for the position', () => {
  // The block at the position is text; another block carries a call id that
  // looks like it could be the one. There is no fallback to it.
  const read = loaded([record(CALLER, [text(0), call(1)])]);
  expect(resolveJump({ record_uuid: CALLER, block_index: 0 }, read)).toEqual({
    kind: 'unavailable',
    reason: 'not_a_tool_call',
  });
});

it('compares the record identity as the bytes it was recorded with', () => {
  const padded = ` ${CALLER}`;
  const read = loaded([record(CALLER, [call(0, 'Read')]), record(padded, [call(0, 'Bash')])]);
  // Two distinct identities, so each position names exactly one call.
  expect(resolveJump({ record_uuid: CALLER, block_index: 0 }, read)).toMatchObject({
    kind: 'target',
    tool: 'Read',
  });
  expect(resolveJump({ record_uuid: padded, block_index: 0 }, read)).toMatchObject({
    kind: 'target',
    tool: 'Bash',
    blockId: `${padded}:0`,
  });
  // And a whitespace variant nothing carries is not in the transcript at all.
  expect(resolveJump({ record_uuid: `${CALLER} `, block_index: 0 }, read)).toEqual({
    kind: 'unavailable',
    reason: 'not_in_transcript',
  });
});

it('keeps punctuation in an identity as it is', () => {
  // A colon in the identity cannot make two positions compose to one id: the
  // index after the last colon is only ever digits.
  const odd = 'a:1 "quoted" [x]';
  const read = loaded([record(odd, [text(0), text(1), call(2)]), record('a', [call(12)])]);
  expect(resolveJump({ record_uuid: odd, block_index: 2 }, read)).toEqual({
    kind: 'target',
    blockId: `${odd}:2`,
    tool: 'Read',
  });
  expect(resolveJump({ record_uuid: 'a', block_index: 12 }, read)).toMatchObject({
    kind: 'target',
    blockId: 'a:12',
  });
});

it('has a sentence for every reason, and none of them names a place on disk', () => {
  for (const reason of [
    'no_locator',
    'transcript_loading',
    'transcript_unavailable',
    'not_in_transcript',
    'ambiguous',
    'not_a_tool_call',
  ] as const) {
    const said = jumpUnavailableText(reason);
    expect(said.length, reason).toBeGreaterThan(0);
    expect(said).not.toMatch(/\.claude|\/Users/);
  }
});
