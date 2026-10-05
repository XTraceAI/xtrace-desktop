import { expect, it } from 'vitest';
import type { SessionSourceReason } from '../../data/generated/SessionSourceReason';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { SourceBlock } from '../../data/generated/SourceBlock';
import type { SourceRecord } from '../../data/generated/SourceRecord';
import { blockId, partialNote, transcriptProps, unavailableNote } from './transcript-adapter';
import type { ToolCallBlock, ToolResultBlock } from './transcript-view';

/** Every transcript here is written by this test, never taken from a session. */
const RECORD = '11111111-1111-4111-8111-111111111111';
const ANSWER = '22222222-2222-4222-8222-222222222222';

function read(
  records: SourceRecord[],
  gaps: ('missing' | 'unreadable')[] = [],
): SessionSourceStatus {
  return {
    state: 'loaded',
    generation: { generation: 'indexed', appended: false },
    sources: 1,
    dropped_records: 0,
    gaps: gaps.map((gap) => ({ gap })),
    records,
  };
}

const record = (id: string, blocks: SourceBlock[], at: string | null = null): SourceRecord => ({
  id,
  role: id === ANSWER ? 'user' : 'assistant',
  at,
  blocks,
});

it('gives a block the identity of its record and its place in it', () => {
  // The transport carries the two halves separately because that pair is the
  // locator a measured tool call already has. Composing them is this module's
  // job, and the view puts the result on the node untouched.
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'text', index: 0, text: 'first' },
        { kind: 'text', index: 1, text: 'second' },
      ]),
    ]),
  );
  expect(props.records[0].blocks.map((block) => block.id)).toEqual([`${RECORD}:0`, `${RECORD}:1`]);
  expect(blockId(RECORD, 7)).toBe(`${RECORD}:7`);
});

it('keeps two records that share an identity apart by their own blocks', () => {
  // A forked history can copy a record, so two records may arrive under one
  // UUID. Their blocks still name which record they are in, and both render.
  const props = transcriptProps(
    read([
      record(RECORD, [{ kind: 'text', index: 0, text: 'one' }]),
      record(RECORD, [{ kind: 'text', index: 0, text: 'two' }]),
    ]),
  );
  expect(props.records.map((item) => item.id)).toEqual([RECORD, RECORD]);
  expect(props.records.flatMap((item) => item.blocks.map((block) => block.id))).toEqual([
    `${RECORD}:0`,
    `${RECORD}:0`,
  ]);
});

it('pairs a call with the result that names it, wherever that result is', () => {
  // A host writes the result in the record *after* the call, so a pairing
  // built inside one record would call every result-bearing call unrecorded.
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_1', input: null },
        { kind: 'tool_call', index: 1, name: 'Bash', call_id: 'toolu_2', input: null },
      ]),
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_1', failed: false, parts: [] },
        {
          kind: 'tool_result',
          index: 1,
          call_id: 'toolu_2',
          failed: true,
          parts: [{ kind: 'text', text: 'no such flag' }],
        },
      ]),
    ]),
  );
  const [first, second] = props.records[0].blocks as ToolCallBlock[];
  expect(first.outcome.state).toBe('ok');
  expect(second.outcome.state).toBe('failed');
  // The result's own payload stays in the record that carried it, so nothing
  // is shown twice and the transcript reads in the order it happened.
  expect('output' in first.outcome && first.outcome.output).toBeFalsy();
  const results = props.records[1].blocks as ToolResultBlock[];
  expect(results[1].outcome).toEqual({
    state: 'failed',
    output: { kind: 'text', text: 'no such flag' },
  });
  // A result reads as the tool that produced it, through the call it names.
  expect(results.map((block) => block.name)).toEqual(['Read', 'Bash']);
});

it('calls a tool call unrecorded only when nothing in the session answers it', () => {
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_9', input: null },
        // A call the record gave no identifier: nothing could answer it.
        { kind: 'tool_call', index: 1, name: 'Read', call_id: null, input: null },
      ]),
    ]),
  );
  const [named, anonymous] = props.records[0].blocks as ToolCallBlock[];
  expect(named.outcome).toEqual({ state: 'unrecorded' });
  expect(anonymous.outcome).toEqual({ state: 'unrecorded' });
  expect(anonymous.callId).toBe(null);
});

it('keeps an unpaired result, and never invents the call it belonged to', () => {
  const props = transcriptProps(
    read([
      record(ANSWER, [
        {
          kind: 'tool_result',
          index: 0,
          call_id: 'toolu_absent',
          failed: false,
          parts: [{ kind: 'text', text: 'came back anyway' }],
        },
      ]),
    ]),
  );
  const [block] = props.records[0].blocks as ToolResultBlock[];
  expect(block.kind).toBe('tool_result');
  expect(block.callId).toBe('toolu_absent');
  expect(block.name).toBe(null);
});

it('will not say how a call ended when two results name it', () => {
  // The tie between a call and its result is an identifier the record states.
  // Stated twice, it ties nothing in particular: reading it as a success, a
  // failure or a missing result would each be this adapter's claim.
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_1', input: null },
      ]),
      record(ANSWER, [
        {
          kind: 'tool_result',
          index: 0,
          call_id: 'toolu_1',
          failed: false,
          parts: [{ kind: 'text', text: 'the first answer' }],
        },
        {
          kind: 'tool_result',
          index: 1,
          call_id: 'toolu_1',
          failed: true,
          parts: [{ kind: 'text', text: 'the second answer' }],
        },
      ]),
    ]),
  );
  expect((props.records[0].blocks[0] as ToolCallBlock).outcome).toEqual({ state: 'ambiguous' });
  // Neither a success nor a missing result, and every answer still renders
  // with everything it carried.
  const results = props.records[1].blocks as ToolResultBlock[];
  expect(results).toHaveLength(2);
  expect(results[0].outcome).toEqual({
    state: 'ok',
    output: { kind: 'text', text: 'the first answer' },
  });
  expect(results[1].outcome).toEqual({
    state: 'failed',
    output: { kind: 'text', text: 'the second answer' },
  });
});

it('will not say how a call ended when two calls share its identifier', () => {
  // The duplication can be on the other side: one result, and no statement of
  // which of the two calls it answered.
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_1', input: null },
        { kind: 'tool_call', index: 1, name: 'Bash', call_id: 'toolu_1', input: null },
      ]),
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_1', failed: false, parts: [] },
      ]),
    ]),
  );
  const calls = props.records[0].blocks as ToolCallBlock[];
  expect(calls.map((block) => block.outcome)).toEqual([
    { state: 'ambiguous' },
    { state: 'ambiguous' },
  ]);
  // Both calls keep their own names and are both still drawn.
  expect(calls.map((block) => block.name)).toEqual(['Read', 'Bash']);
  // And the result is not given one of the two names, because the record does
  // not say which of them it came from.
  expect((props.records[1].blocks[0] as ToolResultBlock).name).toBe(null);
});

it('names a result after its call only when exactly one call claims it', () => {
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_one', input: null },
        { kind: 'tool_call', index: 1, name: 'Read', call_id: 'toolu_two', input: null },
        { kind: 'tool_call', index: 2, name: 'Bash', call_id: 'toolu_two', input: null },
      ]),
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_one', failed: false, parts: [] },
        { kind: 'tool_result', index: 1, call_id: 'toolu_two', failed: false, parts: [] },
        { kind: 'tool_result', index: 2, call_id: 'toolu_none', failed: false, parts: [] },
      ]),
    ]),
  );
  const results = props.records[1].blocks as ToolResultBlock[];
  // One call: its name. Two calls under one id: no single name to take. No
  // call at all: nothing to take a name from.
  expect(results.map((block) => block.name)).toEqual(['Read', null, null]);
});

it('keeps a unique relationship unambiguous even beside a duplicated one', () => {
  // The rule is per identifier, not per transcript: one bad identifier must
  // not turn every other call in the session into an ambiguous one.
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_clean', input: null },
        { kind: 'tool_call', index: 1, name: 'Bash', call_id: 'toolu_twice', input: null },
      ]),
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_clean', failed: true, parts: [] },
        { kind: 'tool_result', index: 1, call_id: 'toolu_twice', failed: false, parts: [] },
        { kind: 'tool_result', index: 2, call_id: 'toolu_twice', failed: false, parts: [] },
      ]),
    ]),
  );
  expect((props.records[0].blocks as ToolCallBlock[]).map((block) => block.outcome)).toEqual([
    { state: 'failed' },
    { state: 'ambiguous' },
  ]);
});

it('carries a call’s arguments as the value they are, and its name verbatim', () => {
  const props = transcriptProps(
    read([
      record(RECORD, [
        {
          kind: 'tool_call',
          index: 0,
          name: '__proto__',
          call_id: 'toolu_1',
          input: { kind: 'json', value: { command: "echo '<script>x</script>\tünïcödé 🙂'" } },
        },
      ]),
    ]),
  );
  const [block] = props.records[0].blocks as ToolCallBlock[];
  expect(block.name).toBe('__proto__');
  expect(block.input).toEqual({
    kind: 'json',
    value: { command: "echo '<script>x</script>\tünïcödé 🙂'" },
  });
});

it('joins a result’s parts in order and names what is not shown', () => {
  const props = transcriptProps(
    read([
      record(ANSWER, [
        {
          kind: 'tool_result',
          index: 0,
          call_id: 'toolu_1',
          failed: false,
          parts: [
            { kind: 'text', text: 'before' },
            { kind: 'unshown', label: 'image' },
            { kind: 'unshown', label: null },
            { kind: 'text', text: 'after' },
          ],
        },
      ]),
    ]),
  );
  const [block] = props.records[0].blocks as ToolResultBlock[];
  expect(block.outcome).toEqual({
    state: 'ok',
    output: {
      kind: 'text',
      text: [
        'before',
        '[a image in this result is not shown here]',
        '[part of this result is not shown here]',
        'after',
      ].join('\n'),
    },
  });
});

it('gives a result with no parts no output at all', () => {
  const props = transcriptProps(
    read([
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_1', failed: false, parts: [] },
      ]),
    ]),
  );
  expect((props.records[0].blocks[0] as ToolResultBlock).outcome).toEqual({
    state: 'ok',
    output: undefined,
  });
});

it('words a recorded instant in the reader’s own zone and borrows none', () => {
  const props = transcriptProps(
    read([
      record(RECORD, [{ kind: 'text', index: 0, text: 'said' }], '2026-09-07T12:00:00.000Z'),
      record(ANSWER, [{ kind: 'text', index: 0, text: 'replied' }], null),
    ]),
  );
  expect(props.records[0].at?.iso).toBe('2026-09-07T12:00:00.000Z');
  expect(props.records[0].at?.label).toBe(
    new Date('2026-09-07T12:00:00.000Z').toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
    }),
  );
  // A record with no recorded time shows none rather than a neighbour's.
  expect(props.records[1].at).toBe(null);
});

it('shows a record whose time cannot be read, without its time', () => {
  const props = transcriptProps(
    read([record(RECORD, [{ kind: 'text', index: 0, text: 'said' }], 'not-a-time')]),
  );
  expect(props.records[0].at).toBe(null);
  expect(props.records[0].blocks).toHaveLength(1);
});

it('never counts a turn number the session did not record', () => {
  // The records are the session's files in read order — its transcript, then
  // its sub-agent transcripts — so a count here would number turns wrongly.
  const props = transcriptProps(
    read([
      record(RECORD, [{ kind: 'text', index: 0, text: 'one' }]),
      record(ANSWER, [{ kind: 'text', index: 0, text: 'two' }]),
    ]),
  );
  expect(props.records.every((item) => item.ordinal === undefined)).toBe(true);
});

it('names an unsupported block by its own label and draws nothing for it', () => {
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'unsupported', index: 0, label: 'image' },
        { kind: 'unsupported', index: 1, label: null },
      ]),
    ]),
  );
  expect(props.records[0].blocks).toEqual([
    { kind: 'unsupported', id: `${RECORD}:0`, label: 'image' },
    { kind: 'unsupported', id: `${RECORD}:1`, label: null },
  ]);
});

it('reads as ready only when the read covered the whole session', () => {
  expect(transcriptProps(read([])).state).toEqual({ kind: 'ready' });
  const partial = transcriptProps(read([], ['missing', 'unreadable']));
  expect(partial.state.kind).toBe('incomplete');
  expect(partial.state.kind === 'incomplete' && partial.state.note).toBe(
    'This is part of this session’s record: a sub-agent transcript it counted is no longer on this Mac; one of its files could not be read.',
  );
});

it('says a gap once however many files carried it', () => {
  expect(partialNote([{ gap: 'missing' }, { gap: 'missing' }, { gap: 'missing' }], 0)).toBe(
    'This is part of this session’s record: a sub-agent transcript it counted is no longer on this Mac.',
  );
  expect(partialNote([], 0)).toBe(null);
  expect(partialNote([{ gap: 'stopped', line: 41 }], 0)).toContain('line 41');
});

it('has a sentence for every reason the read can give, and diagnoses none of them', () => {
  const reasons: SessionSourceReason[] = [
    { reason: 'missing' },
    { reason: 'moved' },
    { reason: 'replaced' },
    { reason: 'unreadable' },
    { reason: 'ambiguous', candidates: 2 },
    { reason: 'too_large', limit: 'bytes', reached: 70 * 1024 * 1024, ceiling: 64 * 1024 * 1024 },
    { reason: 'too_large', limit: 'records', reached: 200_001, ceiling: 200_000 },
    { reason: 'cancelled' },
    { reason: 'invalid_identifier' },
    { reason: 'not_indexed' },
    { reason: 'prerequisite_unavailable' },
    { reason: 'unsupported_host' },
    { reason: 'reader_unavailable', cause: 'interpreter' },
    { reason: 'reader_unavailable', cause: 'readers' },
    { reason: 'reader_unavailable', cause: 'index' },
    ...(
      [
        'source_bytes',
        'files',
        'native_rows',
        'records',
        'line_bytes',
        'output_bytes',
        'header_bytes',
        'discovery_entries',
        'header_probe_bytes',
        'probe_bytes',
        'probes',
      ] as const
    ).map((limit): SessionSourceReason => ({ reason: 'reader_limit', limit })),
    { reason: 'reader_deadline' },
    { reason: 'store_unsupported' },
    { reason: 'reader_protocol' },
  ];
  for (const reason of reasons) {
    const note = unavailableNote(reason);
    expect(note.length, JSON.stringify(reason)).toBeGreaterThan(0);
    // None of them says the session did not happen, and none of them shows a
    // reader a place on their disk.
    expect(note).not.toMatch(/\.claude|\/Users|error|failed to/i);
  }
  // Every one is its own sentence: no two reasons read alike.
  const notes = reasons.map(unavailableNote);
  expect(new Set(notes).size).toBe(notes.length);
  expect(unavailableNote({ reason: 'reader_limit', limit: 'native_rows' })).toContain(
    'too many lines',
  );
  // Finding which file holds the session is not the session's size: those
  // ceilings never say the session is too large, and the body ceilings do.
  for (const limit of [
    'discovery_entries',
    'header_probe_bytes',
    'probe_bytes',
    'probes',
  ] as const) {
    const note = unavailableNote({ reason: 'reader_limit', limit });
    expect(note, limit).toContain('could not confirm which file holds this session');
    expect(note, limit).not.toMatch(/larger|too large/);
  }
  expect(unavailableNote({ reason: 'reader_limit', limit: 'source_bytes' })).toContain(
    'larger than XTrace reads at once',
  );
  expect(unavailableNote({ reason: 'store_unsupported' })).toContain('database');
  expect(unavailableNote({ reason: 'reader_unavailable', cause: 'interpreter' })).toContain(
    'Python 3.10',
  );
  expect(unavailableNote({ reason: 'ambiguous', candidates: 3 })).toContain('(3)');
  expect(
    unavailableNote({ reason: 'too_large', limit: 'records', reached: 200_001, ceiling: 200_000 }),
  ).toContain('200,001');
});

it('renders no turns beside an unavailable text, and never as an empty session', () => {
  const props = transcriptProps({ state: 'unavailable', reason: { reason: 'replaced' } });
  expect(props.records).toEqual([]);
  expect(props.state.kind).toBe('unavailable');
  expect(props.state.kind === 'unavailable' && props.state.note).toContain('has changed');
});

it('treats a cancelled read as unavailable, never as a cancelled session', () => {
  // The view's `cancelled` state says somebody's *session* stopped early and
  // shows what it recorded. A read this app abandoned is a different fact.
  const props = transcriptProps({ state: 'unavailable', reason: { reason: 'cancelled' } });
  expect(props.state.kind).toBe('unavailable');
});

it('calls a turn that carries only results what it is, not who it is filed under', () => {
  // A host writes a tool's results as a record of its own and files it under
  // `user`. Nobody typed it, and labelling it "Human" would put a person's
  // name on a machine's reply — and on the tool strip beneath it.
  const props = transcriptProps(
    read([
      record(RECORD, [
        { kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_1', input: null },
      ]),
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_1', failed: false, parts: [] },
      ]),
    ]),
  );
  expect(props.records.map((item) => item.role)).toEqual(['assistant', 'tool']);
});

it('still calls a turn a person contributed to theirs', () => {
  // Anything a person wrote alongside the results keeps the turn theirs, and a
  // turn with nothing in it is not re-labelled on the strength of no evidence.
  const withText = transcriptProps(
    read([
      record(ANSWER, [
        { kind: 'tool_result', index: 0, call_id: 'toolu_1', failed: false, parts: [] },
        { kind: 'text', index: 1, text: 'and here is what I wanted next' },
      ]),
    ]),
  );
  expect(withText.records[0].role).toBe('user');
  expect(transcriptProps(read([record(ANSWER, [])])).records[0].role).toBe('user');
});

it('treats records the read could not identify as part of the record being missing', () => {
  // Dropped records are inside the file and are not shown as turns, and the
  // measurements beside the text still count them. A transcript missing them
  // is not the whole record and must not be offered as one.
  const status = read([record(RECORD, [{ kind: 'text', index: 0, text: 'said' }])]);
  if (status.state !== 'loaded') throw new Error('the fixture is a loaded read');
  const partial = transcriptProps({ ...status, dropped_records: 3 });
  expect(partial.state.kind).toBe('incomplete');
  expect(partial.state.kind === 'incomplete' && partial.state.note).toBe(
    'This is part of this session’s record: 3 of its records could not be identified and are not shown.',
  );
  // The turns it did read are still all there.
  expect(partial.records[0].blocks).toHaveLength(1);
  // One reads as one.
  expect(partialNote([], 1)).toContain('one of its records could not be identified');
  // And they are said together with the gaps, not instead of them.
  const both = partialNote([{ gap: 'missing' }], 2);
  expect(both).toContain('no longer on this Mac');
  expect(both).toContain('2 of its records');
});
