import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { TRANSCRIPT_TEXT } from './SessionTranscript';
import { TranscriptFixturePreview } from './TranscriptFixturePreview';
import { SYNTHETIC_TRANSCRIPT } from './transcript-fixtures';

afterEach(cleanup);

it('shows every state of the view over synthetic records', () => {
  render(<TranscriptFixturePreview />);

  const transcript = () => screen.getByRole('region', { name: 'Session transcript' });
  expect(transcript().querySelectorAll('[data-record-id]')).toHaveLength(
    SYNTHETIC_TRANSCRIPT.length,
  );

  fireEvent.click(screen.getByRole('button', { name: 'loading' }));
  expect(screen.getByRole('status').textContent).toBe(TRANSCRIPT_TEXT.loading);

  fireEvent.click(screen.getByRole('button', { name: 'unavailable' }));
  expect(screen.getByText(TRANSCRIPT_TEXT.unavailable)).toBeTruthy();

  fireEvent.click(screen.getByRole('button', { name: 'no turns' }));
  expect(screen.getByText(TRANSCRIPT_TEXT.empty)).toBeTruthy();

  fireEvent.click(screen.getByRole('button', { name: 'incomplete' }));
  expect(screen.getByRole('note').textContent).toContain(TRANSCRIPT_TEXT.incomplete);
  expect(transcript().querySelectorAll('[data-record-id]')).toHaveLength(
    SYNTHETIC_TRANSCRIPT.length,
  );
});

it('holds nobody\u2019s history — every fixture string is written here', () => {
  const serialised = JSON.stringify(SYNTHETIC_TRANSCRIPT);
  // Synthetic throughout: invented paths, an invented tool, an unroutable host.
  expect(serialised).not.toMatch(/\/Users\/|\/home\/|xtrace|memhub|claude/i);
  for (const match of serialised.matchAll(/https?:\/\/[^"\\ ]+/g)) {
    expect(match[0]).toContain('example.invalid');
  }
});
