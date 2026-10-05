import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { SessionTranscript } from './SessionTranscript';
import { CLAMP_LINES, exceedsClamp } from './TranscriptText';
import { SYNTHETIC_TRANSCRIPT } from './transcript-fixtures';

afterEach(cleanup);

/**
 * Activation from the keyboard, as far as jsdom can carry it.
 *
 * Every control here is a native `<button>`, so a browser turns Enter and Space on the
 * focused element into exactly this click — jsdom does not, which is why the focus is placed
 * first and the event is dispatched at `document.activeElement`. What the assertions are
 * really holding is that the control is focusable, reachable, states its own expanded-ness,
 * and does not throw the reader's place away when it acts.
 */
function pressFocused(control: HTMLElement) {
  control.focus();
  expect(document.activeElement).toBe(control);
  fireEvent.click(document.activeElement as HTMLElement);
}

const transcript = () => screen.getByRole('region', { name: 'Session transcript' });

/**
 * Open every run of tool work.
 *
 * A closed row holds each call's identity and not its payload, so a test about what a card
 * says has to open the row first — exactly as a reader does.
 */
function openEveryRun() {
  for (const row of screen.queryAllByRole('button', { expanded: false })) fireEvent.click(row);
}

it('opens a run of tool work from the keyboard and keeps the reader on the control', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const row = screen.getByRole('button', { name: /Read 2 files/ });
  expect(row.tagName).toBe('BUTTON');
  expect(row.getAttribute('tabindex')).toBeNull();
  expect(row.getAttribute('aria-expanded')).toBe('false');

  const panel = document.getElementById(row.getAttribute('aria-controls')!)!;
  expect(panel.hidden).toBe(true);

  pressFocused(row);
  expect(row.getAttribute('aria-expanded')).toBe('true');
  expect(panel.hidden).toBe(false);
  // Opening a row must not move focus: the reader's place is the control they pressed.
  expect(document.activeElement).toBe(row);

  pressFocused(row);
  expect(row.getAttribute('aria-expanded')).toBe('false');
  expect(panel.hidden).toBe(true);
});

it('summarises a run before it is opened, and says how many calls failed', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  expect(screen.getByRole('button', { name: /Read 2 files/ }).textContent).toContain(
    '/synthetic/example/fixture.csv · /synthetic/example/notes.txt',
  );
  expect(screen.getByRole('button', { name: /Ran 1 command/ }).textContent).toContain('1 failed');
});

it('shows each outcome as the different fact it is', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);
  openEveryRun();

  const unrecorded = transcript().querySelector('[data-call-id="call-synthetic-read-2"]')!;
  // Terminal, not pending: a finished transcript will never fill this in, so nothing here
  // may read as still running.
  expect(unrecorded.textContent).toContain('No result for this call is in the record.');
  expect(unrecorded.textContent).not.toMatch(/running|pending|waiting|loading/i);

  const failed = transcript().querySelector('[data-call-id="call-synthetic-bash-1"]')!;
  expect(failed.textContent).toContain('synthetic-tool: exit status 2');
  // The error summarises the call; it does not replace what the call printed first.
  expect(failed.querySelector('[data-payload="output"] pre')?.textContent).toContain('read 2 rows');

  const omitted = transcript().querySelector('[data-call-id="call-synthetic-mcp-1"]')!;
  expect(omitted.textContent).toContain('input not shown here · input was not retained');
  expect(omitted.textContent).toContain('output not shown here · output was not retained');
  // A payload that is not here is a sentence about its absence, with nothing behind it.
  expect(omitted.querySelectorAll('pre, button, a')).toHaveLength(0);
});

it('renders code and tool payloads as inert, bounded text', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);
  openEveryRun();

  const code = transcript().querySelector(
    '[data-block-id="66666666-7777-4888-8999-000000000001:7"] pre',
  )!;
  expect(code.textContent).toBe('column_one,column_two\nalpha,17\nbeta,4\n');
  expect(code.querySelector('*')).toBeNull();
  expect(
    within(
      transcript().querySelector('[data-block-id="66666666-7777-4888-8999-000000000001:7"]')!,
    ).getByText('csv'),
  ).toBeTruthy();

  // The payload scrolls inside its own box rather than being clamped: every character stays
  // reachable without a control, and one result cannot push the session off the page.
  const output = transcript().querySelector(
    '[data-call-id="call-synthetic-read-1"] [data-payload="output"] pre',
  )!;
  expect(output.textContent).toContain('alpha,17');
  expect(output.closest('[data-clamped]')).toBeNull();
});

it('clamps a long block and hands the rest over from the keyboard', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const pasted = transcript().querySelectorAll('[data-block-id="pasted:0"]')[0];
  const box = pasted.querySelector('[data-clamped]') as HTMLElement;
  expect(box.getAttribute('data-clamped')).toBe(String(CLAMP_LINES));
  expect(box.style.maxHeight).toBe(`${CLAMP_LINES}lh`);
  // Nothing is dropped while it is clamped — the whole block is in the document, cut by CSS.
  expect(box.textContent).toContain('row 60: synthetic value 413');

  const toggle = within(pasted as HTMLElement).getByRole('button', {
    name: 'Show the rest of this block',
  });
  pressFocused(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(toggle.textContent).toBe('Show less');
  expect(pasted.querySelector('[data-clamped]')).toBeNull();
  expect((box as HTMLElement).style.maxHeight).toBe('');
  expect(document.activeElement).toBe(toggle);

  pressFocused(toggle);
  expect(pasted.querySelector('[data-clamped]')).toBeTruthy();
});

it('offers no control on a block that fits, so nothing is ever hidden without one', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const short = transcript().querySelector(
    '[data-block-id="11111111-2222-4333-8444-555555555555:0"]',
  )!;
  expect(short.querySelector('[data-clamped]')).toBeNull();
  expect(short.querySelector('button')).toBeNull();

  // Both triggers are read off the text: a budget of lines, and one enormous line.
  expect(exceedsClamp('one line')).toBe(false);
  expect(exceedsClamp('line\n'.repeat(CLAMP_LINES + 1))).toBe(true);
  expect(exceedsClamp('x'.repeat(2_001))).toBe(true);
  expect(exceedsClamp('x'.repeat(2_000))).toBe(false);
});

it('makes every scrollable payload a named stop the keyboard can reach', () => {
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);
  openEveryRun();

  const boxes = [...transcript().querySelectorAll('pre')];
  expect(boxes.length).toBeGreaterThan(0);
  for (const box of boxes) {
    // WebKit puts no plain scroll container in the tab order, so the box has to ask. Without
    // this, the hidden part of a long payload is reachable by pointer and by nothing else.
    expect(box.tabIndex).toBe(0);
    expect(box.getAttribute('role')).toBe('region');
    expect(box.getAttribute('aria-label')?.trim()).toBeTruthy();
  }

  // Named from the label the reader can see, so the announcement and the screen agree.
  const output = transcript().querySelector(
    '[data-call-id="call-synthetic-read-1"] [data-payload="output"] pre',
  ) as HTMLElement;
  expect(output.getAttribute('aria-label')).toBe('Output, scrollable code');
  expect(screen.getByRole('region', { name: 'csv, scrollable code' })).toBeTruthy();

  // And it takes focus, which is what the browser then scrolls with arrow and page keys. The
  // scrolling itself is the browser's; it is proved in WebKit, not here — jsdom lays nothing
  // out, so every box measures as empty and unscrollable.
  output.focus();
  expect(document.activeElement).toBe(output);
});

it('holds a closed run’s identities without holding its payloads', () => {
  // The identity has to be in the document before anyone opens the row, so a jump can find
  // the node and reveal it. The payload does not, and mounting it is what a long session
  // pays for: on a synthetic 1,500-record session, the closed cards put 4,000 payload boxes
  // and 24.4 million characters into the DOM before a reader had opened anything.
  render(<SessionTranscript records={SYNTHETIC_TRANSCRIPT} />);

  const anchor = transcript().querySelector('[data-call-id="call-synthetic-bash-1"]')!;
  expect(anchor.getAttribute('data-block-id')).toBe('66666666-7777-4888-8999-000000000001:4');
  expect(anchor.getAttribute('data-tool')).toBe('Bash');
  expect(anchor.getAttribute('data-tool-state')).toBe('failed');
  // Addressable, and nothing else: no text, no control, nothing to focus, and not a second
  // thing for a reader to meet on the way down the page.
  expect(anchor.textContent).toBe('');
  expect(anchor.getAttribute('aria-hidden')).toBe('true');
  expect(anchor.querySelectorAll('*')).toHaveLength(0);
  // No tool payload is formatted or mounted while every run is closed. The record's own code
  // block is not a tool payload and is unaffected.
  expect(transcript().querySelectorAll('[data-payload] pre')).toHaveLength(0);

  // Opening the run the anchor sits in replaces it with the card, under the same identity.
  const row = screen.getByRole('button', { name: /Ran 1 command/ });
  fireEvent.click(row);
  const card = transcript().querySelector('[data-call-id="call-synthetic-bash-1"]')!;
  expect(card.getAttribute('data-block-id')).toBe('66666666-7777-4888-8999-000000000001:4');
  expect(card.getAttribute('aria-hidden')).toBeNull();
  expect(card.querySelector('[data-payload="output"] pre')?.textContent).toContain('read 2 rows');
});
