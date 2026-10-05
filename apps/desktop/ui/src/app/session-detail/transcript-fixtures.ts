import type { TranscriptRecord } from './transcript-view';

/**
 * A tool result longer than the payload box is tall, so the scroll region below has something
 * to scroll — the case that has to be reachable from the keyboard as well as the pointer.
 */
const LONG_RESULT = [
  'column_one,column_two',
  'alpha,17',
  'beta,4',
  ...Array.from({ length: 80 }, (_, row) => `gamma-${row + 1},${row * 3}`),
].join('\n');

/**
 * Synthetic transcript records — written here, by hand, for this view's tests and its local
 * preview.
 *
 * **Nothing in this file came from a session.** No captured history, no exported design
 * sample, no path or command anyone ran: every string is invented to exercise a shape this
 * view has to survive — a long paste, a failed call, a payload the store did not keep, a
 * block type that does not exist. That is deliberate. A fixture taken from a real transcript
 * would put somebody's work into the repository to test a renderer with, and the renderer
 * does not care whose text it is.
 *
 * The ids are the awkward ones on purpose: a uuid, a name with a colon in it, and two blocks
 * that share an id — the case a keyed list silently drops if it keys on identity alone.
 */
export const SYNTHETIC_TRANSCRIPT: readonly TranscriptRecord[] = [
  {
    id: '11111111-2222-4333-8444-555555555555',
    role: 'user',
    ordinal: 1,
    at: { iso: '2026-01-02T03:04:05.000Z', label: 'Jan 2, 03:04 AM' },
    blocks: [
      {
        kind: 'text',
        id: '11111111-2222-4333-8444-555555555555:0',
        text: 'Check the sample fixture and tell me what the second column holds.',
      },
    ],
  },
  {
    id: '66666666-7777-4888-8999-000000000001',
    role: 'assistant',
    ordinal: 2,
    at: { iso: '2026-01-02T03:04:19.000Z', label: 'Jan 2, 03:04 AM' },
    blocks: [
      {
        kind: 'thinking',
        id: '66666666-7777-4888-8999-000000000001:0',
        text: 'The question is about the fixture, so read it before answering.',
      },
      {
        kind: 'text',
        id: '66666666-7777-4888-8999-000000000001:1',
        text: 'Reading the fixture now.',
      },
      {
        kind: 'tool_call',
        id: '66666666-7777-4888-8999-000000000001:2',
        name: 'Read',
        callId: 'call-synthetic-read-1',
        input: { kind: 'json', value: { file_path: '/synthetic/example/fixture.csv' } },
        outcome: { state: 'ok', output: { kind: 'text', text: LONG_RESULT } },
      },
      {
        kind: 'tool_call',
        id: '66666666-7777-4888-8999-000000000001:3',
        name: 'Read',
        callId: 'call-synthetic-read-2',
        input: { kind: 'json', value: { file_path: '/synthetic/example/notes.txt' } },
        outcome: { state: 'unrecorded' },
      },
      {
        kind: 'tool_call',
        id: '66666666-7777-4888-8999-000000000001:4',
        name: 'Bash',
        callId: 'call-synthetic-bash-1',
        input: { kind: 'json', value: { command: 'synthetic-tool --count' } },
        outcome: {
          state: 'failed',
          error: 'synthetic-tool: exit status 2',
          output: { kind: 'text', text: 'read 2 rows\nsynthetic-tool: no such flag: --count' },
        },
      },
      {
        kind: 'tool_call',
        id: '66666666-7777-4888-8999-000000000001:5',
        name: 'mcp__synthetic__search_notes',
        callId: 'call-synthetic-mcp-1',
        input: { kind: 'omitted', note: 'input was not retained' },
        outcome: { state: 'ok', output: { kind: 'omitted', note: 'output was not retained' } },
      },
      {
        kind: 'text',
        id: '66666666-7777-4888-8999-000000000001:6',
        text: 'The second column holds a count. The whole file is below.',
      },
      {
        kind: 'code',
        id: '66666666-7777-4888-8999-000000000001:7',
        label: 'csv',
        text: 'column_one,column_two\nalpha,17\nbeta,4\n',
      },
    ],
  },
  {
    id: '66666666-7777-4888-8999-000000000002',
    role: 'user',
    ordinal: 3,
    blocks: [
      {
        kind: 'text',
        id: 'pasted:0',
        // A long paste, so the clamp and its control have something to do. Synthetic rows,
        // generated here rather than copied from anywhere.
        text: [
          'Here is the whole synthetic run, pasted in:',
          ...Array.from({ length: 60 }, (_, row) => `row ${row + 1}: synthetic value ${row * 7}`),
        ].join('\n'),
      },
      {
        kind: 'text',
        id: 'pasted:0',
        // The same id twice: two blocks a keyed list must both keep.
        text: 'And the markup the tool printed: <script>should never run</script> & https://example.invalid/not-a-link',
      },
      { kind: 'unsupported', id: 'pasted:1', label: 'image' },
      { kind: 'unsupported', id: 'pasted:2' },
    ],
  },
  {
    id: '66666666-7777-4888-8999-000000000003',
    role: 'assistant',
    ordinal: 4,
    blocks: [{ kind: 'text', id: '66666666-7777-4888-8999-000000000003:0', text: '   ' }],
  },
];
