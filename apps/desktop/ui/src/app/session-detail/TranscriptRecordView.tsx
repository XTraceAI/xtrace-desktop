import { ToolActivityStrip } from './ToolStrip';
import { TranscriptMarkdown } from './TranscriptMarkdown';
import { CodeSurface, TranscriptText } from './TranscriptText';
import {
  type TranscriptBlock,
  type TranscriptRecord,
  type TranscriptRole,
  recordSegments,
} from './transcript-view';
import './transcript.css';

/**
 * What the header calls each side.
 *
 * Fixed wording, and no name: who a session belonged to is attribution, which this view does
 * not do and is not given the data for. `User` is the role the record was saved under, not a
 * claim that a person typed it — another agent's prompt is saved under the same role.
 */
const ROLE_LABEL: Record<TranscriptRole, string> = {
  user: 'User',
  assistant: 'Agent',
  system: 'System',
  tool: 'Tool',
};

/**
 * One non-tool block, drawn as itself.
 *
 * Only the agent's text is formatted. It is Markdown because the model wrote Markdown; a
 * person's prompt, a system line, thinking and code are shown as the characters they are —
 * see `TranscriptMarkdown` for why the line falls there.
 */
function TranscriptBlockView({
  block,
  role,
}: {
  block: Exclude<TranscriptBlock, { kind: 'tool_call' | 'tool_result' }>;
  role: TranscriptRole;
}) {
  return (
    <div data-block-id={block.id} data-block={block.kind}>
      {block.kind === 'text' || block.kind === 'thinking' ? (
        <TranscriptText
          text={block.text}
          kind={block.kind}
          formatted={
            role === 'assistant' && block.kind === 'text' ? (
              <TranscriptMarkdown text={block.text} />
            ) : undefined
          }
        />
      ) : block.kind === 'code' ? (
        <CodeSurface code={block.text} label={block.label ?? undefined} />
      ) : (
        // The honest fallback. A block this view cannot draw is named, not guessed at and not
        // dumped as JSON — and a media block is named rather than loaded, because this view
        // fetches nothing.
        <p className="xt-tx-unsupported">
          {block.label?.trim()
            ? `A block of type “${block.label.trim()}” in this turn is not shown here.`
            : 'A block in this turn is not shown here.'}
        </p>
      )}
    </div>
  );
}

/**
 * One record, full width and left aligned.
 *
 * Every turn is the same shape whoever it is from: a transcript is a record being read, not a
 * conversation being held, and chat bubbles would put the person and the agent on opposite
 * sides of a column that is only ever scanned top to bottom.
 *
 * The id goes onto the element untouched, and the header states a time only when the record
 * carries one — a turn with no recorded timestamp is shown without one rather than borrowing
 * a neighbour's.
 */
export function TranscriptRecordView({ record }: { record: TranscriptRecord }) {
  const segments = recordSegments(record);

  return (
    <article className="xt-turn" data-role={record.role} data-record-id={record.id}>
      <header className="xt-turn-head">
        <span>{ROLE_LABEL[record.role]}</span>
        {typeof record.ordinal === 'number' ? <span>turn {record.ordinal}</span> : null}
        {record.at ? (
          <time className="xt-turn-time" dateTime={record.at.iso}>
            {record.at.label}
          </time>
        ) : null}
      </header>

      <div className="xt-turn-body">
        {segments.length === 0 ? (
          // The turn happened; what it carried is not in this view. Dropping the record
          // instead would quietly shorten the session.
          <p className="xt-tx-unsupported">No content from this turn is shown here.</p>
        ) : (
          segments.map((segment) =>
            segment.kind === 'tools' ? (
              <ToolActivityStrip key={segment.key} blocks={segment.blocks} />
            ) : (
              <TranscriptBlockView key={segment.key} block={segment.block} role={record.role} />
            ),
          )
        )}
      </div>
    </article>
  );
}
