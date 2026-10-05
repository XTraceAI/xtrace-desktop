/**
 * The transcript's one way in from outside: "show this block".
 *
 * A timeline that measured a stretch can say which tool call the stretch began
 * with, and the reader should be able to go there. What it must not do is reach
 * into the transcript and look for the call itself — building a selector out of
 * an identifier that came from somebody's history, and guessing at which of the
 * nodes that answer to it is meant. So the request is small and explicit, and
 * the transcript answers it from what it already holds:
 *
 * - The caller names a block by the presentation id the adapter composed, and
 *   only after confirming against the loaded payload that exactly one tool call
 *   sits there. A request is a token and an id; nothing else crosses.
 * - The one tool row holding that block opens itself **only if it is closed**,
 *   and after the commit that mounts its cards it takes the card at the block's
 *   own position in the row — the anchor that stood there is gone by then.
 *   Nothing is searched for in the document.
 * - It scrolls that card into view, respecting a reader's reduced-motion
 *   setting, marks it highlighted, and reports back once per token. Focus is
 *   never moved: the reader stays on the control they pressed.
 *
 * A request whose token the caller has since replaced is simply not the current
 * request any more; its highlight goes with it, and a late report for it is the
 * caller's to ignore.
 */
import { createContext } from 'react';

export interface RevealRequest {
  /** Unique per request, so one request is answered once. */
  readonly token: number;
  /** The presentation id of the one tool call to show. */
  readonly blockId: string;
}

/** `not_found`: no card stood at that block's place after the row opened. */
export type RevealOutcome = 'revealed' | 'not_found';

export interface RevealChannel {
  readonly request: RevealRequest | null;
  readonly report: (token: number, outcome: RevealOutcome) => void;
}

export const RevealContext = createContext<RevealChannel>({
  request: null,
  report: () => {},
});

/** Whether the reader asked for less motion. Absent support means no preference. */
export function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}
