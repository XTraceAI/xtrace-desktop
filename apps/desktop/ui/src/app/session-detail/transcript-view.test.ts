import { expect, it } from 'vitest';
import type { ToolBlock, TranscriptRecord } from './transcript-view';
import {
  humanizeToolName,
  isDrawableBlock,
  recordSegments,
  toolDetail,
  toolFailureCount,
  toolGroups,
  toolPrimaryArgument,
  toolSummary,
} from './transcript-view';

const record = (blocks: TranscriptRecord['blocks']): TranscriptRecord => ({
  id: 'record-1',
  role: 'assistant',
  blocks,
});

const call = (id: string, name: string, extra: Partial<ToolBlock> = {}): ToolBlock =>
  ({
    kind: 'tool_call',
    id,
    name,
    outcome: { state: 'ok' },
    ...extra,
  }) as ToolBlock;

it('folds contiguous tool blocks into one strip and lets prose close the run', () => {
  const segments = recordSegments(
    record([
      { kind: 'text', id: 'b0', text: 'first' },
      call('b1', 'Read'),
      call('b2', 'Read'),
      { kind: 'text', id: 'b3', text: 'second' },
      call('b4', 'Bash'),
    ]),
  );

  expect(segments.map((segment) => segment.kind)).toEqual(['block', 'tools', 'block', 'tools']);
  expect(segments[1].kind === 'tools' && segments[1].blocks.map((block) => block.id)).toEqual([
    'b1',
    'b2',
  ]);
  // Two strips, each where the work happened: merging them would file the second burst under
  // an explanation that was written before it.
  expect(segments[3].kind === 'tools' && segments[3].blocks).toHaveLength(1);
});

it('keeps a run open across a block that draws nothing, and drops that block', () => {
  const segments = recordSegments(
    record([call('b0', 'Read'), { kind: 'text', id: 'b1', text: '  \n ' }, call('b2', 'Read')]),
  );

  expect(segments).toHaveLength(1);
  expect(segments[0].kind === 'tools' && segments[0].blocks.map((block) => block.id)).toEqual([
    'b0',
    'b2',
  ]);
  expect(isDrawableBlock({ kind: 'text', id: 'b1', text: '  ' })).toBe(false);
  // An unsupported block is always drawable: the fallback is the point of it.
  expect(isDrawableBlock({ kind: 'unsupported', id: 'b3' })).toBe(true);
});

it('keys segments by position as well as id so two blocks may share one id', () => {
  const segments = recordSegments(
    record([
      { kind: 'text', id: 'same', text: 'one' },
      { kind: 'text', id: 'same', text: 'two' },
    ]),
  );

  expect(new Set(segments.map((segment) => segment.key)).size).toBe(2);
});

it('groups a strip by tool name in the order each name first appeared', () => {
  const groups = toolGroups([call('b0', 'Read'), call('b1', 'Bash'), call('b2', 'Read')]);

  expect(groups.map((group) => group.map((block) => block.id))).toEqual([['b0', 'b2'], ['b1']]);
});

it('counts and verbs a run, and falls back to a readable name for anything else', () => {
  expect(toolSummary([call('b0', 'Read'), call('b1', 'Read'), call('b2', 'Bash')])).toBe(
    'Read 2 files · ran 1 command',
  );
  expect(toolSummary([call('b0', 'TodoWrite')])).toBe('Updated the plan');
  // The tool set is open: an MCP name that ships next week has to come out readable with no
  // entry in the table, and the default counts rather than guessing a verb for it.
  expect(toolSummary([call('b0', 'mcp__synthetic__search_notes')])).toBe('Search notes');
  expect(
    toolSummary([
      call('b0', 'mcp__synthetic__search_notes'),
      call('b1', 'mcp__synthetic__search_notes'),
    ]),
  ).toBe('Search notes ×2');
  expect(humanizeToolName('mcp__synthetic__')).toBe('Tool');
});

it('names a result with no call in view rather than leaving the row blank', () => {
  expect(toolSummary([{ kind: 'tool_result', id: 'b0', outcome: { state: 'ok' } }])).toBe('Tool');
});

it('reads the argument that says what a call was about, and nothing when it cannot', () => {
  expect(
    toolPrimaryArgument(
      call('b0', 'Read', { input: { kind: 'json', value: { file_path: ' /synthetic/a.txt ' } } }),
    ),
  ).toBe('/synthetic/a.txt');
  // An unknown tool falls back to the fields the known ones actually use.
  expect(
    toolPrimaryArgument(
      call('b0', 'SomethingNew', { input: { kind: 'json', value: { query: 'synthetic' } } }),
    ),
  ).toBe('synthetic');
  // A payload the store did not keep has no argument to show, and free text is not a field.
  expect(toolPrimaryArgument(call('b0', 'Bash', { input: { kind: 'omitted' } }))).toBeNull();
  expect(
    toolPrimaryArgument(call('b0', 'Bash', { input: { kind: 'text', text: 'ls' } })),
  ).toBeNull();
  expect(toolPrimaryArgument(call('b0', 'Bash'))).toBeNull();
  expect(
    toolPrimaryArgument({ kind: 'tool_result', id: 'b0', outcome: { state: 'ok' } }),
  ).toBeNull();
});

it('lists each distinct argument once, in call order, and counts the failures', () => {
  const blocks = [
    call('b0', 'Read', { input: { kind: 'json', value: { file_path: '/synthetic/a.txt' } } }),
    call('b1', 'Read', { input: { kind: 'json', value: { file_path: '/synthetic/a.txt' } } }),
    call('b2', 'Read', { input: { kind: 'json', value: { file_path: '/synthetic/b.txt' } } }),
    call('b3', 'Read', { outcome: { state: 'failed', error: 'no such file' } }),
  ];

  expect(toolDetail(blocks)).toBe('/synthetic/a.txt · /synthetic/b.txt');
  expect(toolFailureCount(blocks)).toBe(1);
});

/**
 * The names that are not names — every key an object literal answers out of its prototype.
 *
 * A tool name comes straight off the record and the set is open, so any of these can arrive:
 * an MCP server may export `toString`, and `__proto__` is one malformed import away. Looked up
 * on an object, they came back inherited rather than absent — `phrase` was not callable,
 * `hasOwnProperty(1)` threw, and the argument table's answer was not iterable. The tables are
 * `Map`s now; these hold them to it.
 */
const INHERITED_NAMES = ['__proto__', 'constructor', 'toString', 'hasOwnProperty', 'valueOf'];

it.each(INHERITED_NAMES)(
  'summarises a tool named %s from the table, not from a prototype',
  (name) => {
    expect(toolSummary([call('b0', name)])).toBe(humanizeToolName(name));
    expect(toolSummary([call('b0', name), call('b1', name)])).toBe(`${humanizeToolName(name)} ×2`);
  },
);

it.each(INHERITED_NAMES)(
  'reads the argument of a tool named %s from the fallback fields',
  (name) => {
    expect(
      toolPrimaryArgument(
        call('b0', name, { input: { kind: 'json', value: { file_path: '/synthetic/a.txt' } } }),
      ),
    ).toBe('/synthetic/a.txt');
  },
);

it('reads an argument the payload owns, never one it inherited', () => {
  // A payload whose prototype carries `command`: the value is a string and would have passed
  // the type test, but the tool was never given it.
  const inherited = Object.create({ command: 'never-ran --this' }) as Record<string, unknown>;
  expect(
    toolPrimaryArgument(call('b0', 'Bash', { input: { kind: 'json', value: inherited } })),
  ).toBeNull();

  inherited.command = 'synthetic-tool --count';
  expect(
    toolPrimaryArgument(call('b0', 'Bash', { input: { kind: 'json', value: inherited } })),
  ).toBe('synthetic-tool --count');
});
