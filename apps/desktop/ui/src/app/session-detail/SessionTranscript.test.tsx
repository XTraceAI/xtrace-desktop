import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { SessionTranscript, TRANSCRIPT_TEXT } from './SessionTranscript';
import { SYNTHETIC_TRANSCRIPT } from './transcript-fixtures';
import type { TranscriptRecord } from './transcript-view';

afterEach(cleanup);

const transcript = () => screen.getByRole('region', { name: 'Session transcript' });

it('renders the records it was given, in the order it was given them', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const ids = [...transcript().querySelectorAll('[data-record-id]')].map((node) =>
    node.getAttribute('data-record-id'),
  );
  expect(ids).toEqual(SYNTHETIC_TRANSCRIPT.map((record) => record.id));
  // Read off the records, never counted from the array: a turn with no ordinal shows none.
  expect(transcript().querySelector('[data-role="user"]')?.textContent).toContain('turn 1');
});

it('carries every id through to the DOM exactly, including a duplicate one', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const pasted = transcript().querySelectorAll('[data-block-id="pasted:0"]');
  // Two blocks legitimately share an id. Both are rendered — keying on identity alone would
  // silently drop one of them.
  expect(pasted).toHaveLength(2);

  // A jump target exists before anyone opens the row it sits in: while the run is closed
  // that node is the call's anchor, and it carries the same identity the card carries.
  const anchor = transcript().querySelector('[data-call-id="call-synthetic-bash-1"]')!;
  expect(anchor.getAttribute('data-block-id')).toBe('66666666-7777-4888-8999-000000000001:4');
  expect(anchor.getAttribute('data-tool')).toBe('Bash');
  expect(anchor.getAttribute('data-tool-state')).toBe('failed');
});

it('states a recorded time and borrows none for a record without one', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const first = transcript().querySelector(
    '[data-record-id="11111111-2222-4333-8444-555555555555"]',
  )!;
  const time = first.querySelector('time')!;
  expect(time.getAttribute('datetime')).toBe('2026-01-02T03:04:05.000Z');
  expect(time.textContent).toBe('Jan 2, 03:04 AM');

  const untimed = transcript().querySelector(
    '[data-record-id="66666666-7777-4888-8999-000000000002"]',
  )!;
  expect(untimed.querySelector('time')).toBeNull();
});

it('names a block it cannot draw and loads nothing to draw it with', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  expect(
    screen.getByText('A block of type \u201cimage\u201d in this turn is not shown here.'),
  ).toBeTruthy();
  expect(screen.getByText('A block in this turn is not shown here.')).toBeTruthy();

  // Nothing remote, and nothing clickable that the record did not put there: no media
  // elements at all, no src anywhere, and no anchors auto-made out of text.
  const region = transcript();
  expect(region.querySelectorAll('img, video, audio, iframe, object, embed, source')).toHaveLength(
    0,
  );
  expect(region.querySelectorAll('[src], [href], a')).toHaveLength(0);
});

it('renders recorded markup as characters, never as markup', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const region = transcript();
  expect(region.querySelector('script')).toBeNull();
  expect(
    screen.getByText(/<script>should never run<\/script>/, { exact: false }).textContent,
  ).toContain('https://example.invalid/not-a-link');
});

it('says a turn carried nothing this view shows, rather than dropping the turn', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const blank = transcript().querySelector(
    '[data-record-id="66666666-7777-4888-8999-000000000003"]',
  )!;
  expect(blank.textContent).toContain('No content from this turn is shown here.');
});

it('distinguishes a transcript that cannot be shown from a session with no turns', () => {
  render(
    <SessionTranscript records={[]} state={{ kind: 'unavailable', note: 'Told by the caller.' }} />,
  );
  const unavailable = transcript();
  expect(within(unavailable).getByText(TRANSCRIPT_TEXT.unavailable)).toBeTruthy();
  expect(within(unavailable).getByText('Told by the caller.')).toBeTruthy();
  // It withholds a claim; it does not make the opposite one, and it diagnoses nothing.
  expect(unavailable.textContent).not.toMatch(/no turns|empty|nothing|never/i);
  expect(unavailable.textContent).not.toMatch(/error|failed|corrupt|missing|broken/i);

  cleanup();
  render(<SessionTranscript records={[]} />);
  expect(screen.getByText(TRANSCRIPT_TEXT.empty)).toBeTruthy();
  expect(TRANSCRIPT_TEXT.empty).toBe('This view was given no turns to show.');
});

it('offers nothing beside an unavailable transcript, and says so while it is still loading', () => {
  const records: TranscriptRecord[] = [
    { id: 'r1', role: 'assistant', blocks: [{ kind: 'text', id: 'b1', text: 'half an answer' }] },
  ];

  render(<SessionTranscript records={records} state={{ kind: 'unavailable' }} />);
  // Anything rendered here would be offered as the transcript, which is the claim this state
  // exists to withhold.
  expect(screen.queryByText('half an answer')).toBeNull();

  cleanup();
  render(<SessionTranscript records={records} state={{ kind: 'loading' }} />);
  expect(screen.getByRole('status').textContent).toBe(TRANSCRIPT_TEXT.loading);
  expect(screen.queryByText('half an answer')).toBeNull();
});

it('leads with what is true about a partial record and still shows the turns', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} state={{ kind: 'cancelled' }} />);
  const region = transcript();
  const notice = within(region).getByRole('note');
  expect(notice.textContent).toBe(TRANSCRIPT_TEXT.cancelled);
  // First, so a reader knows what they are reading before they scroll it.
  expect(region.firstElementChild).toBe(notice);
  expect(region.querySelectorAll('[data-record-id]')).toHaveLength(SYNTHETIC_TRANSCRIPT.length);

  cleanup();
  render(
    <SessionTranscript
      records={SYNTHETIC_TRANSCRIPT}
      state={{ kind: 'incomplete', note: 'Told by the caller.' }}
    />,
  );
  expect(screen.getByRole('note').textContent).toContain(TRANSCRIPT_TEXT.incomplete);
  expect(screen.getByText('Told by the caller.')).toBeTruthy();
});

it('renders a record whose tools are named after Object\u2019s own prototype', () => {
  // Not hypothetical: the tool set is open, an MCP server may export `toString`, and
  // `__proto__` is one malformed import away. Every one of these used to reach an inherited
  // value through the summary and argument tables, and the record took the whole screen down
  // with it rather than drawing an oddly named tool.
  const names = ['__proto__', 'constructor', 'toString', 'hasOwnProperty', 'valueOf'];
  const records: TranscriptRecord[] = [
    {
      id: 'prototype-names',
      role: 'assistant',
      blocks: names.map((name, index) => ({
        kind: 'tool_call' as const,
        id: `prototype-names:${index}`,
        name,
        callId: `call-${index}`,
        input: { kind: 'json' as const, value: { file_path: `/synthetic/${index}.txt` } },
        outcome: { state: 'ok' as const },
      })),
    },
  ];

  render(<SessionTranscript records={records} />);

  const cards = [...transcript().querySelectorAll('[data-tool]')];
  expect(cards.map((card) => card.getAttribute('data-tool'))).toEqual(names);
  // Each row still summarises itself, and the name survives the humanising untouched.
  expect(screen.getByRole('button', { name: /^Proto/ })).toBeTruthy();
  expect(screen.getByRole('button', { name: /^ToString/ })).toBeTruthy();
  expect(screen.getByRole('button', { name: /^Constructor/ }).textContent).toContain(
    '/synthetic/1.txt',
  );
});
