import { type ReactNode, useId, useState } from 'react';
import './transcript.css';

/**
 * How much of one block is shown before the reader has to ask for the rest.
 *
 * A recorded session routinely holds a turn that is a whole test run or a pasted file —
 * several screens of scrolling between two sentences, which stops the transcript reading as a
 * session. Nothing is dropped: the rest is one keystroke away, and this is a DISPLAY budget.
 */
export const CLAMP_LINES = 24;

/**
 * The second trigger: one enormous line.
 *
 * The budget is in lines because `max-height` in `lh` cuts in lines, but a single pasted
 * 30,000-character line is one line and still fills the screen. Both triggers are read off
 * the TEXT rather than measured from the box: a measurement is unavailable until layout has
 * run, and a clamp that hides content the control has not appeared for yet is content the
 * reader cannot reach.
 */
export const CLAMP_CHARACTERS = 2_000;

export function exceedsClamp(text: string): boolean {
  if (text.length > CLAMP_CHARACTERS) return true;
  let lines = 1;
  for (let index = text.indexOf('\n'); index !== -1; index = text.indexOf('\n', index + 1)) {
    if (++lines > CLAMP_LINES) return true;
  }
  return false;
}

/**
 * The first part of a block, as characters: at most {@link CLAMP_LINES} lines and
 * {@link CLAMP_CHARACTERS} characters.
 *
 * What a collapsed formatted block shows instead of its formatting. Cutting Markdown at a line
 * would leave half a table or an open fence for a parser to guess at, and parsing all of it to
 * then hide most of it is the cost the clamp exists to avoid — so the preview is literal, and
 * the block is parsed only once the reader asks for it.
 */
export function clampPreview(text: string): string {
  let end = Math.min(text.length, CLAMP_CHARACTERS);
  let lines = 1;
  for (
    let index = text.indexOf('\n');
    index !== -1 && index < end;
    index = text.indexOf('\n', index + 1)
  ) {
    if (++lines > CLAMP_LINES) {
      end = index;
      break;
    }
  }
  return text.slice(0, end);
}

/**
 * Inert text, cut to the budget, with a control to read the rest in place.
 *
 * **Inert is the point.** The text is a child, so React escapes it: a transcript carries
 * whatever the model and the tools emitted — HTML, a shell line, a `javascript:` URL — and
 * this renders all of it as characters. There is no `dangerouslySetInnerHTML` here and no
 * auto-linking, so nothing in a recorded session becomes something the reader can click or
 * the page can run.
 *
 * `formatted` is the one exception, and the caller decides it: the agent's own answer, drawn
 * by `TranscriptMarkdown` (which keeps the same promise — see there). A formatted block that
 * fits is drawn formatted. One that does not is shown collapsed as a literal preview of its
 * first lines ({@link clampPreview}), and the formatted whole is mounted — and so parsed — only
 * when the reader expands it. That keeps two things true a CSS clamp over formatted content
 * would break: a long answer nobody opened costs no parse, and nothing focusable (a code box, a
 * table) ever sits hidden behind the clamp where the keyboard could reach it and the eye could
 * not. Expanded, the block is the whole answer; nothing of it is dropped.
 *
 * Literal blocks keep the whole text in the document and cut it with CSS, as before.
 *
 * The control is a real `<button>`: it is in the tab order, Enter and Space operate it, and
 * `aria-expanded` says which way it goes. Expanding never moves focus, so a keyboard reader
 * stays exactly where they were.
 */
export function TranscriptText({
  text,
  kind,
  formatted,
}: {
  text: string;
  kind: 'text' | 'thinking';
  /** The block drawn formatted, when it is the agent's answer. Absent means literal. */
  formatted?: ReactNode;
}) {
  const [expanded, setExpanded] = useState(false);
  const bodyId = useId();
  const clampable = exceedsClamp(text);
  const clamped = clampable && !expanded;

  return (
    <div className="xt-clamp">
      <div
        id={bodyId}
        className="xt-clamp-box"
        // Observable, because the height below is expressed in `lh`, which jsdom does not
        // parse — a test asserting the style would assert nothing.
        data-clamped={clamped ? CLAMP_LINES : undefined}
        style={clamped ? { maxHeight: `${CLAMP_LINES}lh` } : undefined}
      >
        {formatted === undefined ? (
          <p className="xt-tx-text" data-block={kind}>
            {text}
          </p>
        ) : clamped ? (
          <p className="xt-tx-text" data-block={kind} data-preview="">
            {clampPreview(text)}
          </p>
        ) : (
          formatted
        )}
      </div>
      {clampable && (
        <button
          type="button"
          className="xt-clamp-toggle"
          aria-expanded={expanded}
          aria-controls={bodyId}
          onClick={() => setExpanded((open) => !open)}
        >
          {expanded ? 'Show less' : 'Show the rest of this block'}
        </button>
      )}
    </div>
  );
}

/**
 * A bounded, scrolling code surface — a code block, or one side of a tool call.
 *
 * Bounded rather than clamped: a payload scrolls inside its own box, so every character is
 * reachable without a control and without the box pushing the rest of the transcript off the
 * page. Rendered as a text child of `<pre>` for the same reason as above — it is shown, never
 * run, and never parsed for links.
 *
 * **"Reachable" has to mean reachable without a mouse.** A `max-height` with `overflow: auto`
 * and no focusable element inside it is scrollable by pointer and by nothing else: WebKit does
 * not put a plain scroll container in the tab order at all, so on Safari — the engine this app
 * ships inside — the hidden part of a long payload was simply unreachable from the keyboard.
 * Chrome and Firefox now focus such a container themselves, which is exactly how this stayed
 * invisible in development.
 *
 * So the box says what it is and takes focus: `tabIndex` puts it in the tab order,
 * `role="region"` with a name makes it something a screen reader announces and can jump to
 * rather than an unlabelled stop, and the focus ring is drawn (see `transcript.css`) because a
 * stop the reader cannot see is a stop they cannot use. Arrow keys, Page Up/Down, Home and End
 * then scroll it — the browser's own behaviour, with no key handler of ours in the way.
 */
export function CodeSurface({
  code,
  label,
  slot,
}: {
  code: string;
  label?: string | null;
  /** Which side of a tool call this is, when it is one — `input`, `output`. */
  slot?: string;
}) {
  if (!code.trim()) return null;

  return (
    <div className="xt-tx-code" data-payload={slot}>
      {label ? <span className="xt-tx-code-label">{label}</span> : null}
      {/* Named from the visible label where there is one, so what is announced and what is
          read on screen are the same thing. */}
      <pre
        tabIndex={0}
        role="region"
        aria-label={label ? `${label}, scrollable code` : 'Scrollable code'}
      >
        {code}
      </pre>
    </div>
  );
}
