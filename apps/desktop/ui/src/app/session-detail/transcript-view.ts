/**
 * What this view is handed, and what it derives from it.
 *
 * The transcript screen is presentation and nothing else: it renders the turns a caller
 * passes and never reads a file, a database, a network or a clock. This module is the whole
 * input surface — a small readonly shape, deliberately separate from the generated transport
 * DTOs in `src/data/generated`.
 *
 * Separate on purpose. Those types are the wire, they are regenerated from the backend, and
 * they carry fields (usage, identities, measurement) this view must not grow an opinion
 * about. An adapter maps one to the other in one place; when the wire changes, the adapter
 * changes and the rendering does not.
 *
 * **Identity is carried, never re-derived.** A record's `id` and a block's `id` are the
 * caller's exact strings and are put on the DOM verbatim, so a later "jump to this tool call"
 * adapter can find the node it means. Nothing here composes, normalises, parses or renumbers
 * an id, and no position in this array is ever treated as one.
 */

/** Who a record is from. The label under it is fixed — this view names no person. */
export type TranscriptRole = 'user' | 'assistant' | 'system' | 'tool';

/**
 * A recorded instant, already worded by the caller.
 *
 * The caller owns the clock and the zone: which timezone a session is read in is a decision
 * this app makes from the data it loaded, and a presentation component that re-derived one
 * would quietly disagree with every other screen. `iso` is what was recorded, for
 * `<time dateTime>`; `label` is what the reader sees. A record with no recorded time simply
 * has none — nothing here fabricates one from its neighbours.
 */
export interface TranscriptTime {
  readonly iso: string;
  readonly label: string;
}

/**
 * One side of a tool call — what it was given, or what came back.
 *
 * `omitted` is the case this app has to state honestly: an unconfigured database keeps
 * metadata only, so the payload that was recorded is not in the store to show. The caller
 * says so in its own words through `note` (this view invents no reason of its own, and an
 * absent note simply reads as "not shown here"). It is inert either way — there is nothing
 * behind it to open, load or retry.
 */
export type TranscriptPayload =
  | { readonly kind: 'text'; readonly text: string }
  | { readonly kind: 'json'; readonly value: unknown }
  | { readonly kind: 'omitted'; readonly note?: string | null };

/**
 * How a call ended, as the record states it.
 *
 * `unrecorded` is terminal, not pending: a transcript is a finished record, so a call with no
 * result in it will never get one. Anything that read as "still running" would be a promise
 * the page cannot keep.
 *
 * `ambiguous` is the fourth fact, and it exists because the first three would each be a
 * claim. A call is tied to its result by an identifier the record states, and a transcript
 * can state that identifier more than once — two calls under one id, or two results naming
 * one call. Which result answered which call is then not stated anywhere, and `ok`,
 * `failed` and `unrecorded` would each answer a question the record left open. Nothing is
 * hidden by it: every call and every result still renders, with everything it carried.
 */
export type ToolOutcome =
  | { readonly state: 'ok'; readonly output?: TranscriptPayload }
  | {
      readonly state: 'failed';
      readonly error?: string | null;
      readonly output?: TranscriptPayload;
    }
  | { readonly state: 'unrecorded' }
  | { readonly state: 'ambiguous' };

export interface TextBlock {
  readonly kind: 'text';
  readonly id: string;
  readonly text: string;
}

/** Code the record carried. `label` is the language or filename the record stated, verbatim. */
export interface CodeBlock {
  readonly kind: 'code';
  readonly id: string;
  readonly text: string;
  readonly label?: string | null;
}

/** Extended thinking. Quiet on purpose: it is the agent working, not the agent answering. */
export interface ThinkingBlock {
  readonly kind: 'thinking';
  readonly id: string;
  readonly text: string;
}

export interface ToolCallBlock {
  readonly kind: 'tool_call';
  readonly id: string;
  /** The tool's name exactly as recorded — an open set, including MCP names. */
  readonly name: string;
  /** The call's own id, kept so a result and a jump target can both find this call. */
  readonly callId?: string | null;
  readonly input?: TranscriptPayload;
  readonly outcome: ToolOutcome;
}

/** A result whose call is not in this view — carried rather than dropped. */
export interface ToolResultBlock {
  readonly kind: 'tool_result';
  readonly id: string;
  readonly callId?: string | null;
  readonly name?: string | null;
  readonly outcome: ToolOutcome;
}

/**
 * A block this view does not draw.
 *
 * The honest fallback, and the only one: not a placeholder pretending to be content, not
 * `JSON.stringify` of whatever arrived, not a throw. `label` is what the record called the
 * block ("image", "video", a type that ships next month) and is printed as text.
 *
 * Media lands here deliberately. This view loads nothing remote — no `<img>`, no `<video>`,
 * no fetch — so a picture is named, not shown, and opening it is a job for a screen that has
 * the bytes locally.
 */
export interface UnsupportedBlock {
  readonly kind: 'unsupported';
  readonly id: string;
  readonly label?: string | null;
}

export type TranscriptBlock =
  TextBlock | CodeBlock | ThinkingBlock | ToolCallBlock | ToolResultBlock | UnsupportedBlock;

export type ToolBlock = ToolCallBlock | ToolResultBlock;

export interface TranscriptRecord {
  /** The record's exact id. Rendered as `data-record-id`, never parsed. */
  readonly id: string;
  readonly role: TranscriptRole;
  /** The session ordinal the caller recorded, if it has one. Never counted from the array. */
  readonly ordinal?: number | null;
  readonly at?: TranscriptTime | null;
  /** In the order they were recorded. This view never reorders them. */
  readonly blocks: readonly TranscriptBlock[];
}

/**
 * What the caller can say about the transcript as a whole.
 *
 * Five distinct things, because they are five different facts and collapsing any two of them
 * would put a claim on screen that nobody made:
 *
 * - `loading` — the answer has not arrived. It is not "empty".
 * - `unavailable` — it cannot be shown. It is **not** an empty history, and is never worded
 *   as one; `note` is the caller's own sentence, because the reasons live behind this view
 *   and inventing an enum for them here would fix a vocabulary this view cannot honour.
 * - `cancelled` / `incomplete` — the records that *are* here are real, and there is something
 *   true to add about the record they came from. Both render with the turns, not instead of
 *   them.
 * - `ready` — what is passed is what there is. With no records that reads as "this view was
 *   given no turns", which is a statement about this view and not about the session.
 */
export type TranscriptState =
  | { readonly kind: 'ready' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'unavailable'; readonly note?: string | null }
  | { readonly kind: 'cancelled'; readonly note?: string | null }
  | { readonly kind: 'incomplete'; readonly note?: string | null };

/**
 * One drawable run inside a record: a single block, or a stretch of tool work that collapses
 * into one strip.
 *
 * Grouping is **within a record only**. Folding tool work across records would be an account
 * of the session that this view is not entitled to give — records arrive as the caller
 * ordered them, and what belongs with what across them is the adapter's question, not the
 * renderer's.
 */
export type TranscriptSegment =
  | {
      readonly kind: 'block';
      readonly key: string;
      readonly block: Exclude<TranscriptBlock, ToolBlock>;
    }
  | { readonly kind: 'tools'; readonly key: string; readonly blocks: ToolBlock[] };

export function isToolBlock(block: TranscriptBlock): block is ToolBlock {
  return block.kind === 'tool_call' || block.kind === 'tool_result';
}

/**
 * A block with something to draw.
 *
 * A blank text block is the placeholder a tool-only turn leaves behind, and drawing it would
 * put an empty paragraph where the record has nothing. This filters the VIEW: the blocks the
 * caller passed are never mutated, and a record left with no drawable block says so in one
 * line rather than disappearing — a turn that happened is not a turn to hide.
 */
export function isDrawableBlock(block: TranscriptBlock): boolean {
  if (block.kind === 'text' || block.kind === 'thinking' || block.kind === 'code') {
    return block.text.trim().length > 0;
  }
  return true;
}

/**
 * A record's blocks → what it draws, in the order given.
 *
 * Contiguous tool blocks become one strip; anything the agent actually said closes the run,
 * so a record shaped text → tool → text → tool draws two strips, each where it happened.
 *
 * Keys are position-prefixed, and only because React needs uniqueness: two blocks may
 * legitimately arrive carrying the same id, and a duplicate key silently drops a turn. The
 * exact id still reaches the DOM untouched — that is what `data-block-id` is for.
 */
export function recordSegments(record: TranscriptRecord): TranscriptSegment[] {
  const segments: TranscriptSegment[] = [];

  record.blocks.forEach((block, index) => {
    if (!isDrawableBlock(block)) return;

    if (isToolBlock(block)) {
      const open = segments.at(-1);
      if (open?.kind === 'tools') open.blocks.push(block);
      else segments.push({ kind: 'tools', key: `${index}:${block.id}`, blocks: [block] });
      return;
    }
    segments.push({ kind: 'block', key: `${index}:${block.id}`, block });
  });

  return segments;
}

/** Contiguous calls of the same tool name collapse into one row of the strip. */
export function toolGroups(blocks: readonly ToolBlock[]): ToolBlock[][] {
  const groups: ToolBlock[][] = [];
  const indexByName = new Map<string, number>();

  for (const block of blocks) {
    const name = toolLabel(block);
    const index = indexByName.get(name);
    if (index === undefined) {
      indexByName.set(name, groups.length);
      groups.push([block]);
    } else groups[index].push(block);
  }

  return groups;
}

/** A result with no call in view still has to be called something. */
export function toolLabel(block: ToolBlock): string {
  const name = block.kind === 'tool_call' ? block.name : (block.name ?? '');
  return name.trim() || 'Tool';
}

/**
 * How each well-known tool counts itself. `n` is always ≥ 1.
 *
 * **A `Map`, because the key is a tool name off the record and the tool set is open.** An
 * object literal answers `toString`, `constructor`, `valueOf` and `__proto__` out of its
 * prototype, so a call by any of those names came back with something inherited instead of
 * `undefined`: `PHRASE.__proto__` is not callable at all, `PHRASE.hasOwnProperty(1)` throws,
 * and the two that happened to return a string put nonsense in the summary. A `Map` has no
 * prototype chain to fall through, so an unknown name is unknown whatever it is called, and
 * the default branch below is reached the way it was written to be. The same is true of
 * {@link PRIMARY_ARGUMENT}, which failed harder — an inherited value there is not iterable.
 */
const PHRASE = new Map<string, (n: number) => string>([
  ['Read', (n) => `read ${n} ${plural(n, 'file')}`],
  ['Edit', (n) => `edited ${n} ${plural(n, 'file')}`],
  ['MultiEdit', (n) => `edited ${n} ${plural(n, 'file')}`],
  ['Write', (n) => `wrote ${n} ${plural(n, 'file')}`],
  ['NotebookEdit', (n) => `edited ${n} ${plural(n, 'notebook')}`],
  ['Bash', (n) => `ran ${n} ${plural(n, 'command')}`],
  ['BashOutput', (n) => `read ${n} command ${plural(n, 'output')}`],
  ['Glob', (n) => `ran ${n} ${plural(n, 'search', 'searches')}`],
  ['Grep', (n) => `ran ${n} ${plural(n, 'search', 'searches')}`],
  ['Task', (n) => `ran ${n} ${plural(n, 'sub-agent')}`],
  ['TodoWrite', (n) => (n === 1 ? 'updated the plan' : `updated the plan ${n} times`)],
  ['WebFetch', (n) => `fetched ${n} ${plural(n, 'page')}`],
  ['WebSearch', (n) => `searched the web ${n} ${plural(n, 'time')}`],
]);

function plural(n: number, one: string, many = `${one}s`): string {
  return n === 1 ? one : many;
}

/**
 * A tool name a reader can read — `mcp__memhub__search_memory` → `Search memory`.
 *
 * The tool set is open: an MCP name or one that ships next week has to come out readable
 * without an entry in the table above, which is why the default branch counts rather than
 * guessing a verb for it.
 */
export function humanizeToolName(name: string): string {
  const stripped = name
    .replace(/^mcp__[^_]+__/, '')
    .replace(/^[a-z0-9-]+__/, '')
    .replace(/[_-]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (!stripped) return 'Tool';
  return stripped.charAt(0).toUpperCase() + stripped.slice(1);
}

/**
 * The strip's headline: every tool in the run, grouped by name, counted and verbed.
 *
 * Insertion-ordered, so the first tool reached for leads — the order the work happened in,
 * and the only one that needs no explanation. This counts the blocks it was handed and
 * nothing else: it is a caption, not a measurement.
 */
export function toolSummary(blocks: readonly ToolBlock[]): string {
  const counts = new Map<string, number>();
  for (const block of blocks) {
    const name = toolLabel(block);
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }

  const phrases = [...counts].map(([name, count]) => {
    const phrase = PHRASE.get(name);
    if (phrase) return phrase(count);
    const label = humanizeToolName(name);
    return count === 1 ? label : `${label} ×${count}`;
  });

  const summary = phrases.join(' · ');
  return summary.charAt(0).toUpperCase() + summary.slice(1);
}

/**
 * The fields that say what a call was *about* — the path it read, the command it ran.
 *
 * A `Map` for the reason given on {@link PHRASE}: keyed by an open tool name, where an
 * inherited answer is worse than no answer.
 */
const PRIMARY_ARGUMENT = new Map<string, string[]>([
  ['Read', ['file_path']],
  ['Edit', ['file_path']],
  ['MultiEdit', ['file_path']],
  ['Write', ['file_path']],
  ['NotebookEdit', ['notebook_path']],
  ['Bash', ['command']],
  ['Glob', ['pattern', 'path']],
  ['Grep', ['pattern']],
  ['Task', ['description', 'prompt']],
  ['Skill', ['skill', 'name']],
  ['WebFetch', ['url']],
  ['WebSearch', ['query']],
]);

const FALLBACK_ARGUMENT = [
  'file_path',
  'path',
  'command',
  'query',
  'pattern',
  'url',
  'name',
  'description',
  'title',
];

/**
 * The one argument worth putting beside the tool's name, or nothing.
 *
 * Only a `json` input can answer: a payload the store did not keep has no argument to show,
 * and a free-text one is not a field lookup. Nothing is guessed from a command string.
 */
export function toolPrimaryArgument(block: ToolBlock): string | null {
  if (block.kind !== 'tool_call') return null;
  const input = block.input;
  if (!input || input.kind !== 'json') return null;
  if (typeof input.value !== 'object' || input.value === null || Array.isArray(input.value)) {
    return null;
  }

  // The payload is the caller's object, so its own prototype is in reach too: `Object.hasOwn`
  // keeps an inherited `constructor` or a `toString` from being read as an argument the tool
  // was given.
  const fields = input.value as Record<string, unknown>;
  for (const key of PRIMARY_ARGUMENT.get(toolLabel(block)) ?? FALLBACK_ARGUMENT) {
    if (!Object.hasOwn(fields, key)) continue;
    const value = fields[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return null;
}

/**
 * The muted detail beside the headline — the paths and commands, in call order.
 *
 * Returned whole and cut by CSS rather than sliced here: how much fits is a question about
 * the column's width, which this module cannot see.
 */
export function toolDetail(blocks: readonly ToolBlock[]): string {
  const seen = new Set<string>();
  for (const block of blocks) {
    const argument = toolPrimaryArgument(block);
    if (argument) seen.add(argument.replace(/\s+/g, ' '));
  }
  return [...seen].join(' · ');
}

/** How many calls in the run failed — the strip says so before it is opened. */
export function toolFailureCount(blocks: readonly ToolBlock[]): number {
  return blocks.filter((block) => block.outcome.state === 'failed').length;
}
