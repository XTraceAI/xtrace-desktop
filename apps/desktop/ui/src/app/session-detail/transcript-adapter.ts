/**
 * The one place the wire becomes the view.
 *
 * `SessionSourceStatus` is the transport: a session's records with their real
 * identities, the host's own block indexes and call ids, and the closed states
 * that say how complete the read was. `TranscriptRecord` is the presentation
 * contract next door, which renders what it is handed and reads nothing. This
 * module maps one to the other, and it is the only module that knows both.
 *
 * It holds every judgment the mapping needs, so the renderer holds none:
 *
 * - **A block's presentation id is composed here**, out of the record's UUID
 *   and the block's own index. The transport carries those two separately
 *   because `{record uuid, block index}` is the locator a measured tool call
 *   already has; the view wants one string to put in the DOM. Composing is
 *   this module's job precisely so nothing downstream has to take it apart.
 * - **A call is paired with its result by the identifier the record states.**
 *   That link is the transcript's own, not an inference: a `tool_result`
 *   names the `tool_use` it answers. What is *not* done is moving the result
 *   — it renders in the record that carried it, where it happened.
 * - **The clock is the app's.** A record carries the instant it recorded; the
 *   label beside it is worded here, in the reader's own zone, the way every
 *   other screen in this app words one.
 * - **Every reason gets a sentence.** The transport's states are closed
 *   values; the words for them are the view's, and they say what a reader can
 *   see rather than diagnosing a machine.
 */
import type { ReaderLimit } from '../../data/generated/ReaderLimit';
import type { ReaderUnavailableCause } from '../../data/generated/ReaderUnavailableCause';
import type { SessionSourceGap } from '../../data/generated/SessionSourceGap';
import type { SessionSourceReason } from '../../data/generated/SessionSourceReason';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { SourceBlock } from '../../data/generated/SourceBlock';
import type { SourcePayload } from '../../data/generated/SourcePayload';
import type { SourceRecord } from '../../data/generated/SourceRecord';
import type { SourceResultPart } from '../../data/generated/SourceResultPart';
import type {
  ToolOutcome,
  TranscriptBlock,
  TranscriptPayload,
  TranscriptRecord,
  TranscriptState,
} from './transcript-view';

/** What one transcript screen is handed. */
export interface TranscriptProps {
  readonly records: readonly TranscriptRecord[];
  readonly state: TranscriptState;
}

/**
 * A block's identity for the DOM: the record it is in, then where in that
 * record it sits.
 *
 * **It is exactly as unique as the record's UUID is.** Two blocks of one
 * record can never collide, because an index is a place in one list. But two
 * records can arrive under one UUID — a fork copies a record's prefix — and
 * then their blocks share ids too. Both records and both blocks still render,
 * with everything they carried: what is duplicated is a name, not a turn, and
 * hiding one would shorten somebody's session to tidy up an identifier.
 *
 * There is no jump API here yet. When one is built, it must require its match
 * to be **unique** and say so otherwise, rather than taking the first node
 * that answers to the name — which is the same rule the adapter already
 * applies to a call identifier used more than once.
 */
export const blockId = (record: string, index: number) => `${record}:${index}`;

/**
 * Why this transcript is not here, in words a reader can act on.
 *
 * Each one says what is true of the text and stops. None of them says the
 * session did not happen: every measurement beside them stands, which is why
 * the page keeps showing those whatever this says.
 */
export function unavailableNote(reason: SessionSourceReason): string {
  switch (reason.reason) {
    case 'missing':
      return 'Nothing on this Mac holds this session’s original text any more.';
    case 'moved':
      return 'This session’s file is no longer where it was read from. Re-index this Mac to read it where it is now.';
    case 'replaced':
      return 'This session’s file has changed since it was measured, so its text and the numbers beside it would not describe the same thing.';
    case 'unreadable':
      return 'This session’s file, or the history holding it, could not be read.';
    case 'ambiguous':
      return `More than one file names this session (${reason.candidates}), so showing either could show the wrong one.`;
    case 'too_large':
      return reason.limit === 'records'
        ? `This session holds more records than this view reads at once (${reason.reached.toLocaleString()} of ${reason.ceiling.toLocaleString()}).`
        : `This session is larger than this view reads at once (${bytes(reason.reached)} of ${bytes(reason.ceiling)}).`;
    case 'cancelled':
      return 'Reading this transcript was stopped, so none of it was kept.';
    case 'invalid_identifier':
      return 'This session’s recorded identity cannot name a file.';
    case 'not_indexed':
      return 'This Mac’s index holds no local file for this session, so there is nothing to reopen.';
    case 'prerequisite_unavailable':
      return 'Reading this host’s transcripts needs XTrace’s bundled reader, and this read was not given one, so only what was measured is shown.';
    case 'unsupported_host':
      return 'XTrace cannot read this host’s transcripts.';
    case 'reader_unavailable':
      return readerUnavailable[reason.cause];
    case 'reader_limit':
      return isIdentificationLimit(reason.limit)
        ? `XTrace could not confirm which file holds this session (${identificationLimit[reason.limit]}), so none of its text is shown.`
        : `This session is larger than XTrace reads at once (${bodyLimit[reason.limit]}), so none of its text is shown.`;
    case 'reader_deadline':
      return 'Reading this transcript took longer than XTrace allows, so it was stopped and none of it was kept.';
    case 'store_unsupported':
      return 'This Cursor session is kept in a database XTrace does not read transcripts from, so only what was measured is shown.';
    case 'reader_protocol':
      return 'The transcript reader’s answer was not exactly this session, so none of it is shown.';
  }
}

/** Which prerequisite of the transcript reader was missing for this read. */
const readerUnavailable: Record<ReaderUnavailableCause, string> = {
  interpreter:
    'Reading this host’s transcripts needs Python 3.10 or newer, which XTrace could not find on this Mac.',
  readers:
    'XTrace’s bundled transcript reader is not the version this app was built with, so it was not run.',
  index:
    'This app is not reading local history, so there is no reader to open this transcript with.',
};

/**
 * The reader's ceilings split by what reached them. A body ceiling is the
 * selected session's own size. An identification ceiling is the work of
 * finding which file claims the session among every other one in the history,
 * and says nothing about this session's size, so it is never worded as if it
 * did.
 */
type IdentificationLimit = 'discovery_entries' | 'header_probe_bytes' | 'probe_bytes' | 'probes';
type BodyLimit = Exclude<ReaderLimit, IdentificationLimit>;

/** Which of the reader's body ceilings the session passed. */
const bodyLimit: Record<BodyLimit, string> = {
  source_bytes: 'its files hold too many bytes',
  files: 'it spans too many files',
  native_rows: 'its files hold too many lines',
  records: 'it holds too many records',
  line_bytes: 'one of its lines is too long',
  output_bytes: 'its text is too long',
  header_bytes: 'its session header is too long',
};

/** Which identification ceiling stopped the search for the session's file. */
const identificationLimit: Record<IdentificationLimit, string> = {
  discovery_entries: 'the history holds too many entries to search',
  header_probe_bytes: 'another file’s header was too long to check',
  probe_bytes: 'checking the other files’ headers read too much',
  probes: 'the history holds too many files to check',
};

const isIdentificationLimit = (limit: ReaderLimit): limit is IdentificationLimit =>
  Object.hasOwn(identificationLimit, limit);

/** A byte figure a reader can compare, at the scale the ceilings are set in. */
function bytes(value: number): string {
  const mib = value / (1024 * 1024);
  return mib >= 1 ? `${Math.round(mib)} MB` : `${Math.max(1, Math.round(value / 1024))} KB`;
}

/**
 * What this text is missing, said once and plainly.
 *
 * Two different absences, and both make the text partial:
 *
 * - **Gaps** — a file of the session that could not be read, listed or found.
 *   A gap list can hold several kinds and several of one kind; a reader needs
 *   to know what is absent, not how many files it was absent from, so each
 *   kind is said once in a fixed order and the same session always reads the
 *   same way.
 * - **Dropped records** — lines the read accepted as records and could not
 *   identify. They are inside the file, they are not shown as turns, and the
 *   measurements beside the text still count them. A transcript missing them
 *   is not the whole record, so it must not be offered as one; the count is
 *   given because it is the only thing known about them.
 */
export function partialNote(
  gaps: readonly SessionSourceGap[],
  droppedRecords: number,
): string | null {
  const kinds = new Set(gaps.map((gap) => gap.gap));
  const said: string[] = [];
  if (kinds.has('missing')) said.push('a sub-agent transcript it counted is no longer on this Mac');
  if (kinds.has('unreadable')) said.push('one of its files could not be read');
  if (kinds.has('replaced')) said.push('one of its files changed while it was being read');
  if (kinds.has('discovery_incomplete'))
    said.push('part of its sub-agent history could not be listed');
  const stopped = gaps.find((gap) => gap.gap === 'stopped');
  if (stopped) said.push(`one of its files stops being readable at line ${stopped.line}`);
  if (droppedRecords > 0) {
    said.push(
      droppedRecords === 1
        ? 'one of its records could not be identified and is not shown'
        : `${droppedRecords.toLocaleString()} of its records could not be identified and are not shown`,
    );
  }
  if (said.length === 0) return null;
  return `This is part of this session’s record: ${said.join('; ')}.`;
}

/** A recorded instant, worded in the reader's own zone. */
function at(iso: string | null): TranscriptRecord['at'] {
  if (!iso) return null;
  const when = new Date(iso);
  // A record carries what it carries. Rather than print `Invalid Date`, a time
  // this app cannot read is no time at all — the record still renders.
  if (Number.isNaN(when.getTime())) return null;
  return {
    iso,
    label: when.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }),
  };
}

function payload(value: SourcePayload): TranscriptPayload {
  return value.kind === 'text' ? { kind: 'text', text: value.text } : value;
}

/**
 * A result's parts, as one payload.
 *
 * The text parts are joined in the order the record listed them, and a part
 * that is not text keeps its place as a line naming what is not shown. The
 * wording is this module's; the transport carried a label and nothing else,
 * which is what kept a screenshot's bytes off the wire in the first place.
 */
function resultPayload(parts: readonly SourceResultPart[]): TranscriptPayload | undefined {
  if (parts.length === 0) return undefined;
  const lines = parts.map((part) =>
    part.kind === 'text'
      ? part.text
      : part.label
        ? `[a ${part.label} in this result is not shown here]`
        : '[part of this result is not shown here]',
  );
  return { kind: 'text', text: lines.join('\n') };
}

/**
 * How a call ended, from the result that names it.
 *
 * `unrecorded` means exactly what the view says it means: no result for this
 * call is in the transcript at all. Deciding that needs the whole session,
 * because a host writes the result in the record *after* the call — which is
 * why the pairing is built over every record before any of them is mapped.
 *
 * **Nothing is inferred unless the record states it once.** The tie between a
 * call and its result is an identifier, and a transcript can use one twice —
 * two calls under one id, or two results naming one call. Which result then
 * answered which call is stated nowhere, and reading it as a success, a
 * failure or a missing result would each be this adapter's claim rather than
 * the record's. It says so instead, and every call and every result still
 * renders with everything it carried.
 *
 * The result's own payload is deliberately **not** copied onto the call. It
 * renders in the record that carried it, so the transcript reads in the order
 * it happened and nothing appears twice.
 */
function callOutcome(callId: string | null, paired: Paired): ToolOutcome {
  if (callId === null) return { state: 'unrecorded' };
  // More than one call under this identifier: even a single result cannot say
  // which of them it answered.
  if ((paired.calls.get(callId)?.count ?? 0) > 1) return { state: 'ambiguous' };
  const answer = paired.results.get(callId);
  if (answer === undefined) return { state: 'unrecorded' };
  if (answer.count > 1) return { state: 'ambiguous' };
  return answer.failed ? { state: 'failed' } : { state: 'ok' };
}

/**
 * The tool a result came from, when the record ties it to exactly one call.
 *
 * With no call under that identifier there is nothing to take a name from;
 * with more than one there is no single name to take. Either way the result
 * keeps all of its own content, and the view calls it "Tool" — which is what
 * is actually known about it.
 */
function resultName(callId: string | null, paired: Paired): string | null {
  if (callId === null) return null;
  const call = paired.calls.get(callId);
  return call && call.count === 1 ? call.name : null;
}

function block(record: SourceRecord, source: SourceBlock, paired: Paired): TranscriptBlock {
  const id = blockId(record.id, source.index);
  switch (source.kind) {
    case 'text':
      return { kind: 'text', id, text: source.text };
    case 'thinking':
      return { kind: 'thinking', id, text: source.text };
    case 'tool_call':
      return {
        kind: 'tool_call',
        id,
        name: source.name,
        callId: source.call_id,
        input: source.input ? payload(source.input) : undefined,
        outcome: callOutcome(source.call_id, paired),
      };
    case 'tool_result':
      return {
        kind: 'tool_result',
        id,
        callId: source.call_id,
        // The call this answers said what tool it was. Carrying that name here
        // is the record's own link, and it lets a result read as the tool that
        // produced it rather than as an anonymous "Tool" — when the record
        // ties it to exactly one call.
        name: resultName(source.call_id, paired),
        // A result's own state is its own statement, not an inference, so it
        // is carried whatever else names this identifier.
        outcome: source.failed
          ? { state: 'failed', output: resultPayload(source.parts) }
          : { state: 'ok', output: resultPayload(source.parts) },
      };
    case 'unsupported':
      return { kind: 'unsupported', id, label: source.label };
  }
}

interface Paired {
  /** Call id → how many calls claimed it, and the first one's tool. */
  readonly calls: Map<string, { count: number; name: string }>;
  /** Call id → how many results named it, and whether the first one failed. */
  readonly results: Map<string, { count: number; failed: boolean }>;
}

/**
 * What this session states about each call identifier, counted once over every
 * record.
 *
 * **The counts are the point.** An identifier used once ties exactly one call
 * to at most one result, and that tie is the record's own statement. Used more
 * than once on either side it ties nothing in particular, and this view says
 * so rather than picking one. The first mention's name and state are kept only
 * so the unique case can read them; nothing consults them when the count is
 * not one.
 */
function pairs(records: readonly SourceRecord[]): Paired {
  const calls = new Map<string, { count: number; name: string }>();
  const results = new Map<string, { count: number; failed: boolean }>();
  for (const record of records) {
    for (const source of record.blocks) {
      if (source.kind === 'tool_call' && source.call_id !== null) {
        const seen = calls.get(source.call_id);
        if (seen) seen.count += 1;
        else calls.set(source.call_id, { count: 1, name: source.name });
      }
      if (source.kind === 'tool_result' && source.call_id !== null) {
        const seen = results.get(source.call_id);
        if (seen) seen.count += 1;
        else results.set(source.call_id, { count: 1, failed: source.failed });
      }
    }
  }
  return { calls, results };
}

/**
 * What to call a record on screen.
 *
 * The transport says `user` or `assistant`, because that is what the canonical
 * record says. But a host writes a tool's results as a record of its own, and
 * files it under `user` — nobody typed it. Labelling that turn "Human" puts a
 * person's name on a machine's reply, and the strip beneath it then reads as
 * though they ran the tool.
 *
 * So a record that carries **only** results is labelled as the tool it is.
 * That is read off the record's own blocks and nothing else: it is a caption,
 * not a classification. Whether a turn counts as a human message is M-02's
 * question, measured elsewhere, and nothing here consults or restates it.
 */
function role(source: SourceRecord): TranscriptRecord['role'] {
  const onlyResults =
    source.blocks.length > 0 && source.blocks.every((block) => block.kind === 'tool_result');
  return source.role === 'user' && onlyResults ? 'tool' : source.role;
}

/** One record, with its blocks in the order the record states them. */
function record(source: SourceRecord, paired: Paired): TranscriptRecord {
  return {
    id: source.id,
    role: role(source),
    // No ordinal. The records are the session's own files in read order —
    // its transcript, then its sub-agent transcripts — so counting them would
    // number turns that were never numbered, and number them wrongly.
    at: at(source.at),
    blocks: source.blocks.map((block_) => block(source, block_, paired)),
  };
}

/**
 * The whole answer, as the transcript screen takes it.
 *
 * A read that was cancelled is **not** the view's `cancelled` state: that one
 * says the *session* was cancelled and shows what it recorded before it
 * stopped, which is a fact about somebody's work. A cancelled read is a fact
 * about this app, and it has nothing to show, so it is unavailable and says
 * why in its own words.
 */
export function transcriptProps(status: SessionSourceStatus): TranscriptProps {
  if (status.state === 'unavailable') {
    return { records: [], state: { kind: 'unavailable', note: unavailableNote(status.reason) } };
  }
  const paired = pairs(status.records);
  const note = partialNote(status.gaps, status.dropped_records);
  return {
    records: status.records.map((source) => record(source, paired)),
    state: note ? { kind: 'incomplete', note } : { kind: 'ready' },
  };
}
