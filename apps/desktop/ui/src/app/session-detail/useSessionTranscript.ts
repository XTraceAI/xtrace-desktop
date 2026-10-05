/**
 * One open session's transcript, for exactly as long as it is open.
 *
 * Deliberately **not** a cached query. Every other read in this app is one:
 * measurements are small, shared between screens and worth keeping. A
 * transcript is none of those things — it is one session's whole text, held
 * in memory, and a cache would keep somebody's work alive behind a screen
 * they have left. So this is a plain effect with an explicit lifecycle:
 *
 * - **One read per open, named by this open.** The name is this hook's, is
 *   unique per attempt, and is what the backend registers a cancel token
 *   under.
 * - **Leaving cancels.** Navigating away, or opening a different session,
 *   ends the effect, which cancels that read by name. A cancel that overtakes
 *   its own read is still honoured, so the small window between asking and
 *   starting is closed on the other side of the seam too.
 * - **A stale answer is dropped.** The effect that asked is the only one that
 *   may answer; an answer arriving after it ended is discarded unread rather
 *   than rendered over whatever is open now.
 * - **Closing clears it.** The records live in this hook's state and nowhere
 *   else, so unmounting is the whole of their lifecycle.
 */
import { useCallback, useEffect, useState } from 'react';
import { useData } from '../../data/DataProvider';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';

export type TranscriptRead =
  | { readonly phase: 'loading' }
  | { readonly phase: 'read'; readonly status: SessionSourceStatus }
  /** The command itself failed — not a session that cannot be shown. */
  | { readonly phase: 'failed' };

/** A read, and the session it is an answer about. */
type HeldRead = TranscriptRead & { readonly of: string | undefined };

const LOADING: TranscriptRead = { phase: 'loading' };

/**
 * What the caller may render for the session it is rendering now.
 *
 * Effects run **after** the browser has painted. On the first committed frame
 * with a new session in the address, this hook's state still holds the answer
 * about the previous one — the effect that starts the new read has not run
 * yet. Handing that over would paint one session's text under another
 * session's name, and the strip in it is a row of buttons a reader could press
 * in that frame.
 *
 * So the state carries the session it is about, and is only handed over when
 * that still matches. The comparison happens during render, which is when the
 * mismatch exists; a reset scheduled in an effect is by definition too late.
 */
export function readFor(held: HeldRead, sessionId: string | undefined): TranscriptRead {
  return held.of === sessionId ? held : LOADING;
}

/**
 * A name for one read, unique within this window.
 *
 * `crypto.randomUUID` where it exists, and a counter where it does not, so a
 * name is never reused: reusing one could attach this open to a cancel meant
 * for the last.
 */
let reads = 0;
function readId(): string {
  reads += 1;
  const unique =
    typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `read-${reads}-${unique}`;
}

export function useSessionTranscript(sessionId: string | undefined): {
  read: TranscriptRead;
  /** Ask again, as a new open: a new name, a new token, a new answer. */
  retry: () => void;
} {
  const { source } = useData();
  const [held, setHeld] = useState<HeldRead>({ phase: 'loading', of: sessionId });
  /** Bumped to start the read over; the effect below is the whole of a read. */
  const [attempt, setAttempt] = useState(0);
  const retry = useCallback(() => setAttempt((value) => value + 1), []);

  useEffect(() => {
    if (!sessionId) return;
    const id = readId();
    let open = true;
    setHeld({ phase: 'loading', of: sessionId });
    source
      .sessionTranscript(sessionId, id)
      .then((status) => {
        if (open) setHeld({ phase: 'read', status, of: sessionId });
      })
      .catch(() => {
        if (open) setHeld({ phase: 'failed', of: sessionId });
      });
    return () => {
      open = false;
      // The records this open held are dropped with the state below; this ends
      // the read that would otherwise still be filling them.
      setHeld({ phase: 'loading', of: sessionId });
      void source.cancelSessionTranscript(id).catch(() => {
        // A cancel that cannot be delivered leaves a bounded read running and
        // an answer nobody reads. There is nothing to tell the reader.
      });
    };
  }, [sessionId, source, attempt]);

  return { read: readFor(held, sessionId), retry };
}
