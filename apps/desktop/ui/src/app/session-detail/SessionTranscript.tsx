import { useMemo } from 'react';
import { TranscriptRecordView } from './TranscriptRecordView';
import {
  type RevealChannel,
  RevealContext,
  type RevealOutcome,
  type RevealRequest,
} from './transcript-reveal';
import type { TranscriptRecord, TranscriptState } from './transcript-view';
import './transcript.css';

/**
 * The one wording for each thing this view can be told, kept out of the markup so a test can
 * hold the sentence and not a paraphrase of it.
 *
 * None of them diagnoses anything. "Not available" says what the reader can see, never why —
 * the why lives behind this view, and a reason enum invented here would be a vocabulary the
 * loader never agreed to. And nothing that is not an empty history is ever worded as one:
 * a transcript that could not be shown is a different fact from a session with no turns, and
 * reading the first as the second is how a page tells a reader their work was never recorded.
 */
export const TRANSCRIPT_TEXT = {
  loading: 'Loading this transcript.',
  unavailable: 'This transcript is not available to show.',
  cancelled: 'This session was cancelled. Below is what it recorded before it stopped.',
  incomplete: 'This is part of this session’s record. Some of it is not shown here.',
  empty: 'This view was given no turns to show.',
} as const;

/**
 * A session's transcript: the records it was handed, in the order it was handed them.
 *
 * Presentation only. It reads no file, opens no connection, keeps nothing and reports
 * nothing — every fact on the screen arrives as a prop, and the screen's whole job is to
 * state those facts without adding to them. It measures nothing either: no durations, no
 * counts across records, no attribution.
 *
 * A notice leads rather than trails. A reader who is about to scroll a long record needs to
 * know it is partial before they read it, not after.
 */
const NO_REPORT = () => {};

export function SessionTranscript({
  records,
  state = { kind: 'ready' },
  reveal = null,
  onReveal = NO_REPORT,
}: {
  records: readonly TranscriptRecord[];
  state?: TranscriptState;
  /** Show one tool call — see `transcript-reveal.ts`. Absent means none is asked for. */
  reveal?: RevealRequest | null;
  /** Told once per request whether its card was shown. Keep it stable across renders. */
  onReveal?: (token: number, outcome: RevealOutcome) => void;
}) {
  const channel = useMemo<RevealChannel>(
    () => ({ request: reveal, report: onReveal }),
    [reveal, onReveal],
  );
  if (state.kind === 'loading') {
    return (
      <section className="xt-transcript" aria-label="Session transcript" data-state="loading">
        <div className="xt-transcript-notice" role="status">
          <p>{TRANSCRIPT_TEXT.loading}</p>
        </div>
      </section>
    );
  }

  // `unavailable` replaces the turns rather than joining them: anything rendered beside it
  // would be offered as the transcript, which is the claim this state exists to withhold.
  if (state.kind === 'unavailable') {
    return (
      <section className="xt-transcript" aria-label="Session transcript" data-state="unavailable">
        <div className="xt-transcript-notice" data-state="unavailable" role="note">
          <p>{TRANSCRIPT_TEXT.unavailable}</p>
          {state.note?.trim() ? <p className="xt-transcript-note">{state.note}</p> : null}
        </div>
      </section>
    );
  }

  const partial = state.kind === 'cancelled' || state.kind === 'incomplete';

  return (
    <RevealContext.Provider value={channel}>
      <section className="xt-transcript" aria-label="Session transcript" data-state={state.kind}>
        {partial ? (
          <div className="xt-transcript-notice" data-state={state.kind} role="note">
            <p>
              {state.kind === 'cancelled' ? TRANSCRIPT_TEXT.cancelled : TRANSCRIPT_TEXT.incomplete}
            </p>
            {state.note?.trim() ? <p className="xt-transcript-note">{state.note}</p> : null}
          </div>
        ) : null}

        {records.length === 0 ? (
          <p className="xt-tx-unsupported" data-empty="true">
            {TRANSCRIPT_TEXT.empty}
          </p>
        ) : (
          records.map((record, index) => (
            // Position-prefixed for React only; the id itself reaches the DOM untouched.
            <TranscriptRecordView key={`${index}:${record.id}`} record={record} />
          ))
        )}
      </section>
    </RevealContext.Provider>
  );
}
