/**
 * Whether a stretch's first tool call can be shown, and exactly where.
 *
 * M-09 names the call a stretch started with by **position**:
 * `{record_uuid, block_index}` — the record that carried it and the block's
 * place in that record's content. That is not an identity. The host's own call
 * id is not stored, and the store's `tool_uses.id` is a row number, so the
 * position has to be matched against content that was actually read. This
 * module does that match, against the session's structured payload as it was
 * loaded for this open, and nothing else:
 *
 * - **Exactly one block, and it must be a tool call.** The record UUID is
 *   compared as the bytes it was recorded with, and the index as the number it
 *   is. No block there, more than one block there, or one block of another kind
 *   are each their own answer, and none of them is a jump.
 * - **No second guess.** There is no fallback to the host's call id, to the
 *   nearest call, or to the record the call would have been in. A position the
 *   content does not confirm is a position this view does not show; the stretch
 *   itself is still shown, because it was measured whatever the text says.
 * - **Not against the page.** The document is where a confirmed target is
 *   revealed, not where one is looked for. Matching here means nothing is ever
 *   found by building a selector out of an identifier the history supplied.
 */
import type { MetricToolBlock } from '../../data/generated/MetricToolBlock';
import type { SourceBlock } from '../../data/generated/SourceBlock';
import { blockId } from './transcript-adapter';
import type { TranscriptRead } from './useSessionTranscript';

/** Why a stretch's first call cannot be shown. Each is a different fact. */
export type JumpUnavailable =
  /** M-09 could not establish which call the stretch started with. */
  | 'no_locator'
  /** The transcript is still being read. */
  | 'transcript_loading'
  /** The transcript could not be read, or may not be shown. */
  | 'transcript_unavailable'
  /** No block sits at that position in the transcript that was read. */
  | 'not_in_transcript'
  /** More than one block sits at that position, so which is meant is unknown. */
  | 'ambiguous'
  /** The one block at that position is not a tool call. */
  | 'not_a_tool_call';

export type JumpTarget =
  | {
      readonly kind: 'target';
      /** The presentation id the transcript puts on that block. */
      readonly blockId: string;
      /** The tool the record says was called there, verbatim. */
      readonly tool: string;
    }
  | { readonly kind: 'unavailable'; readonly reason: JumpUnavailable };

const unavailable = (reason: JumpUnavailable): JumpTarget => ({ kind: 'unavailable', reason });

export function resolveJump(locator: MetricToolBlock | null, read: TranscriptRead): JumpTarget {
  if (locator === null) return unavailable('no_locator');
  if (read.phase === 'loading') return unavailable('transcript_loading');
  if (read.phase === 'failed' || read.status.state !== 'loaded') {
    return unavailable('transcript_unavailable');
  }
  // Every block at that position, in any record that carries that identity: a
  // forked history can put two records under one UUID, and then the position
  // names two blocks and neither can be chosen.
  const at: SourceBlock[] = [];
  for (const record of read.status.records) {
    // Byte-exact: an identity that differs only in whitespace is another one.
    if (record.id !== locator.record_uuid) continue;
    for (const block of record.blocks) {
      if (block.index === locator.block_index) at.push(block);
    }
  }
  if (at.length === 0) return unavailable('not_in_transcript');
  if (at.length > 1) return unavailable('ambiguous');
  const [only] = at;
  if (only.kind !== 'tool_call') return unavailable('not_a_tool_call');
  return {
    kind: 'target',
    blockId: blockId(locator.record_uuid, locator.block_index),
    tool: only.name,
  };
}

/** What a reader is told, in this view's words, when a jump is not possible. */
export function jumpUnavailableText(reason: JumpUnavailable): string {
  switch (reason) {
    case 'no_locator':
      return 'Which tool call this stretch started with was not recorded.';
    case 'transcript_loading':
      return 'The transcript is still being read.';
    case 'transcript_unavailable':
      return 'The transcript is not available to show.';
    case 'not_in_transcript':
      return 'That tool call is not in the transcript that was read.';
    case 'ambiguous':
      return 'More than one block in the transcript sits where that call was recorded, so which one is meant is not known.';
    case 'not_a_tool_call':
      return 'The block recorded at that place in the transcript is not a tool call.';
  }
}
