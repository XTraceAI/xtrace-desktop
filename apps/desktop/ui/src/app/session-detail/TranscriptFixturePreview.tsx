import { useState } from 'react';
import { SessionTranscript } from './SessionTranscript';
import { SYNTHETIC_TRANSCRIPT } from './transcript-fixtures';
import type { TranscriptState } from './transcript-view';
import './transcript.css';

/**
 * Every state of {@link SessionTranscript}, over synthetic records, in one page.
 *
 * A fixture harness, not a screen: it is reached only by the local preview entry described in
 * this work's report and by this folder's tests. It is not routed, not exported to the app
 * and not wired into the gallery — so nothing that ships imports it, and it stays a way to
 * look at the renderer rather than a claim that a session detail page exists.
 */
const STATES: { label: string; state: TranscriptState; withRecords: boolean }[] = [
  { label: 'ready', state: { kind: 'ready' }, withRecords: true },
  { label: 'loading', state: { kind: 'loading' }, withRecords: false },
  {
    label: 'unavailable',
    state: { kind: 'unavailable', note: 'The caller supplies this sentence.' },
    withRecords: false,
  },
  { label: 'cancelled', state: { kind: 'cancelled' }, withRecords: true },
  {
    label: 'incomplete',
    state: { kind: 'incomplete', note: 'The caller supplies this sentence.' },
    withRecords: true,
  },
  { label: 'no turns', state: { kind: 'ready' }, withRecords: false },
];

export function TranscriptFixturePreview() {
  const [index, setIndex] = useState(0);
  const chosen = STATES[index];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, padding: 12 }}>
      <div role="group" aria-label="Transcript state" style={{ display: 'flex', gap: 6 }}>
        {STATES.map((option, position) => (
          <button
            key={option.label}
            type="button"
            aria-pressed={position === index}
            onClick={() => setIndex(position)}
          >
            {option.label}
          </button>
        ))}
      </div>
      <SessionTranscript
        records={chosen.withRecords ? SYNTHETIC_TRANSCRIPT : []}
        state={chosen.state}
      />
    </div>
  );
}
