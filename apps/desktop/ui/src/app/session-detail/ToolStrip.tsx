import { useContext, useId, useLayoutEffect, useRef, useState } from 'react';
import { StatePill } from '../../kit';
import { CodeSurface } from './TranscriptText';
import { prefersReducedMotion, RevealContext } from './transcript-reveal';
import {
  type ToolBlock,
  type TranscriptPayload,
  toolDetail,
  toolFailureCount,
  toolGroups,
  toolLabel,
  toolPrimaryArgument,
  toolSummary,
} from './transcript-view';
import './transcript.css';

/** A payload is whatever the tool was given or returned — shown, never interpreted. */
function pretty(value: unknown): string {
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2) ?? String(value);
  } catch {
    // A payload with a cycle in it is not worth a crash, and the honest answer is that this
    // one cannot be laid out. `JSON.stringify` is the only thing here that can throw.
    return String(value);
  }
}

/**
 * One side of a tool call — what it was given, or what came back.
 *
 * An `omitted` payload becomes a sentence about its absence rather than a box with nothing in
 * it, and an inert one: there is nothing behind it to open or fetch. The note is the caller's
 * own wording, because why a payload is not here is a fact from behind this view.
 */
export function ToolPayload({ label, payload }: { label: string; payload?: TranscriptPayload }) {
  if (!payload) return null;

  if (payload.kind === 'omitted') {
    const note = payload.note?.trim();
    return (
      <p className="xt-tool-note" data-omitted={label.toLowerCase()}>
        {label.toLowerCase()} not shown here{note ? ` · ${note}` : ''}
      </p>
    );
  }

  return (
    <CodeSurface
      label={label}
      slot={label.toLowerCase()}
      code={payload.kind === 'text' ? payload.text : pretty(payload.value)}
    />
  );
}

/**
 * One tool call, rendered generically — the same card for `Bash`, for `Read`, and for an MCP
 * name that ships next week.
 *
 * **No per-tool special-casing.** The name comes straight off the record, so a card that
 * switched on it would render half the sessions it is shown. What has to be true first is
 * that an unrecognised name renders completely.
 *
 * The three outcomes are three different facts:
 *
 * - `unrecorded` — the call is in the record and no result is. That is terminal, because a
 *   transcript is finished: it settles into a plain sentence, never a spinner.
 * - `failed` — the header is marked and the error leads, but any output is still rendered
 *   underneath. A command that failed after printing still printed, and that is usually the
 *   thing the card was opened for.
 * - `ambiguous` — the record used this call's identifier more than once, so which result
 *   answered it is not stated. Said plainly, and marked as neither a success nor a failure,
 *   because it is neither. The wording promises nothing about where any result is, or that
 *   there is one: a transcript can use that identifier twice and answer it once, never, or
 *   in a turn the reader has already scrolled past.
 * - `ok` — the plain card.
 */
export function ToolCallCard({
  block,
  cardRef,
  highlighted = false,
}: {
  block: ToolBlock;
  /** Where the owning row keeps this card, so a reveal takes it by position. */
  cardRef?: (element: HTMLDivElement | null) => void;
  /** This card is the one a reveal was asked for and landed on. */
  highlighted?: boolean;
}) {
  const name = toolLabel(block);
  const argument = toolPrimaryArgument(block);
  const outcome = block.outcome;
  const failed = outcome.state === 'failed';

  return (
    <div
      ref={cardRef}
      className="xt-tool-card"
      data-highlighted={highlighted || undefined}
      // Identity, carried verbatim: which record, which block, and which call, exactly as
      // the caller stated them. A reveal never looks a card up by these; they are here so
      // the DOM says what it holds.
      data-block-id={block.id}
      data-block={block.kind}
      data-tool={name}
      data-tool-state={outcome.state}
      data-call-id={block.callId ?? undefined}
    >
      <div className="xt-tool-card-head">
        <span className="xt-tool-name">{name}</span>
        {argument ? (
          <span className="xt-tool-arg">{argument}</span>
        ) : (
          <span className="xt-tool-arg" />
        )}
        {failed ? (
          <StatePill tone="danger" height={20}>
            failed
          </StatePill>
        ) : null}
      </div>

      {failed && outcome.error?.trim() ? (
        <p className="xt-tool-note" data-tone="error">
          {outcome.error}
        </p>
      ) : null}

      {outcome.state === 'unrecorded' ? (
        <p className="xt-tool-note">No result for this call is in the record.</p>
      ) : null}

      {outcome.state === 'ambiguous' ? (
        <p className="xt-tool-note">
          This call’s identifier is used more than once in this transcript, so which result answered
          it is not stated. Any recorded results remain in their original turns.
        </p>
      ) : null}

      {block.kind === 'tool_call' ? <ToolPayload label="Input" payload={block.input} /> : null}
      {outcome.state === 'unrecorded' || outcome.state === 'ambiguous' ? null : (
        // A failed call's output is not an error message: it is what the command printed
        // before it gave up, and colouring it red would restate the failure as the content.
        <ToolPayload label="Output" payload={outcome.output} />
      )}
    </div>
  );
}

function Chevron() {
  return (
    <svg
      className="xt-tool-chevron"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M9 6l6 6-6 6" />
    </svg>
  );
}

/**
 * A closed row's stand-in for one call: its identity, and nothing else.
 *
 * The identity has to be in the document before anyone opens the row — a later "jump to this
 * tool call" needs a node to find and to reveal. The *payload* does not: formatting it costs
 * a `JSON.stringify` and puts every character of a tool's output into the DOM for a box
 * nobody has opened.
 *
 * Measured on a synthetic session of 1,500 records (25 MB of payload, comfortably inside the
 * read's own ceiling): mounting the cards closed put 4,000 payload boxes and 24.4 million
 * characters into the document, in 46,001 nodes. The anchors carry the same attributes in
 * five nodes each and no text at all.
 *
 * It is inert in every other way: `aria-hidden`, no text, no control, nothing to focus. What
 * a reader can reach is the row button above it, which is what reveals the card.
 */
function ToolAnchor({ block }: { block: ToolBlock }) {
  return (
    <span
      className="xt-tool-anchor"
      aria-hidden="true"
      data-block-id={block.id}
      data-block={block.kind}
      data-tool={toolLabel(block)}
      data-tool-state={block.outcome.state}
      data-call-id={block.callId ?? undefined}
    />
  );
}

/**
 * One collapsed row of the strip — every contiguous call of the same tool, under one summary.
 *
 * The row is a real `<button>`, so it is in the tab order and Enter and Space open it; the
 * cards it reveals are the next thing in reading order, and opening one never moves focus off
 * the control the reader pressed.
 *
 * **The panel always exists, and holds anchors until the row is opened.** `aria-expanded`
 * therefore points at something real, and every call's id is in the document whether or not
 * anyone has opened the row it sits in. What waits for the open is the payload, not the
 * identity.
 *
 * **A reveal is answered here, by the row that holds the block.** It compares the requested
 * id with its own blocks, opens itself only if it is closed, and once the commit that mounts
 * its cards has happened, takes the card at that block's position from its own refs — see
 * `transcript-reveal.ts`. Nothing is looked up in the document, and focus stays where the
 * reader left it.
 */
function ToolStripRow({ blocks }: { blocks: ToolBlock[] }) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const failures = toolFailureCount(blocks);
  const detail = toolDetail(blocks);
  const { request, report } = useContext(RevealContext);
  // This row's own cards, by position. Filled as cards mount and emptied as they unmount,
  // so after an open commits the card at a position is the one standing there now.
  const cards = useRef(new Map<number, HTMLDivElement>());
  // The request this row last answered, so each one is answered exactly once.
  const answered = useRef<number | null>(null);
  const [revealed, setRevealed] = useState<number | null>(null);
  // Where the requested block sits in this row, if it is here exactly once.
  const places = request
    ? blocks.flatMap((block, index) => (block.id === request.blockId ? [index] : []))
    : [];
  const place = places.length === 1 ? places[0] : null;

  useLayoutEffect(() => {
    if (!request || places.length === 0 || answered.current === request.token) return;
    // Two blocks here answer to one id. The caller confirmed uniqueness against the
    // payload, so this cannot arise from a reveal it sent; it is refused, not guessed at.
    if (place === null) {
      answered.current = request.token;
      report(request.token, 'not_found');
      return;
    }
    // Opened only if closed. The next commit mounts the cards and runs this again.
    if (!open) {
      setOpen(true);
      return;
    }
    answered.current = request.token;
    const card = cards.current.get(place);
    if (!card) {
      report(request.token, 'not_found');
      return;
    }
    setRevealed(request.token);
    // jsdom has no layout, and so no scrolling to do.
    if (typeof card.scrollIntoView === 'function') {
      card.scrollIntoView({
        block: 'center',
        behavior: prefersReducedMotion() ? 'auto' : 'smooth',
      });
    }
    report(request.token, 'revealed');
  }, [request, place, places.length, open, report]);

  return (
    <div className="xt-tool-group">
      <button
        type="button"
        className="xt-tool-row"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((value) => !value)}
      >
        <Chevron />
        <span className="xt-tool-summary">{toolSummary(blocks)}</span>
        {detail ? (
          <span className="xt-tool-detail">{detail}</span>
        ) : (
          <span className="xt-tool-detail" />
        )}
        {failures > 0 ? (
          <StatePill tone="danger" height={20}>
            {`${failures} failed`}
          </StatePill>
        ) : null}
      </button>
      <div className="xt-tool-cards" id={panelId} hidden={!open}>
        {blocks.map((block, index) =>
          open ? (
            <ToolCallCard
              key={`${index}:${block.id}`}
              block={block}
              cardRef={(element) => {
                if (element) cards.current.set(index, element);
                else cards.current.delete(index);
              }}
              highlighted={index === place && request !== null && revealed === request.token}
            />
          ) : (
            <ToolAnchor key={`${index}:${block.id}`} block={block} />
          ),
        )}
      </div>
    </div>
  );
}

/** A stretch of tool work inside one record, as rows that open in place. */
export function ToolActivityStrip({ blocks }: { blocks: ToolBlock[] }) {
  const groups = toolGroups(blocks);

  return (
    <div className="xt-tool-strip">
      {groups.map((group, index) => (
        <ToolStripRow key={`${index}:${group[0]?.id ?? ''}`} blocks={group} />
      ))}
    </div>
  );
}
