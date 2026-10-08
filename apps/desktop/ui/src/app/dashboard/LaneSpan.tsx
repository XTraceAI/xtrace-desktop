import { Tooltip } from '@base-ui/react/tooltip';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useId, useState } from 'react';
import { usePurgeEpoch } from '../../data/content-purges';
import { useData } from '../../data/DataProvider';
import type { DashboardLane } from '../../data/generated/DashboardLane';
import type { DashboardSpanAutomatic } from '../../data/generated/DashboardSpanAutomatic';
import type { DashboardSpanDetail } from '../../data/generated/DashboardSpanDetail';
import type { DashboardSpanPrompt } from '../../data/generated/DashboardSpanPrompt';
import type { DashboardSpanTool } from '../../data/generated/DashboardSpanTool';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { SessionSourceReason } from '../../data/generated/SessionSourceReason';
import { queryKeys } from '../../data/query-client';
import { tokens as formatTokens } from '../../kit/format';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { agentDuration } from '../agent-duration';
import { clockTime, costAmount, plural, recordedTime } from './present';
import '../../styles/span-bubble.css';

let reads = 0;

/**
 * A span's length from its two endpoints, as the bar draws it, written as all
 * agent time is (`3 h 13 m`; `spoken`: in words). A span of one event has no length, and says so
 * rather than reading as zero minutes.
 */
export function spanDuration(startMs: number, endMs: number, spoken = false): string {
  const length = Math.max(0, endMs - startMs);
  if (length === 0) return 'single event';
  return spoken ? agentDuration(length).spoken : agentDuration(length).visible;
}

const toolText = (tool: DashboardSpanTool) =>
  tool.state === 'called' ? tool.name : tool.state === 'no_calls' ? 'no tools' : 'tools unknown';
const toolTitle = (tool: DashboardSpanTool) =>
  tool.state === 'called'
    ? `Most-used tool in this span: ${tool.name}, ${plural(tool.calls, 'call')}`
    : tool.state === 'no_calls'
      ? 'No tool was called in this span'
      : 'Whether a tool was called in this span is not recorded';

/** Words that only waited for a read slot, or whose read was cancelled. */
const waited = (text: { state: string; reason?: { reason: string } }) =>
  text.state === 'busy' || (text.state === 'unavailable' && text.reason?.reason === 'cancelled');

/** An answer whose words may be there next time: kept only until then. */
const retryable = (detail: DashboardSpanDetail | undefined) =>
  detail?.state === 'indexed' &&
  ((detail.prompt.state === 'found' && waited(detail.prompt.text)) ||
    (detail.automatic.state === 'found' && waited(detail.automatic.text)));

/** Why a message's words could not be read from its session's source. */
const unreadable: Record<SessionSourceReason['reason'], string> = {
  missing: 'the session file is gone',
  moved: 'the session file moved',
  replaced: 'the session file changed since it was indexed',
  unreadable: 'the session file could not be read',
  ambiguous: 'more than one file names this session',
  too_large: 'the session file is too large to read',
  cancelled: 'the read was cancelled',
  invalid_identifier: 'no session file is recorded for it',
  not_indexed: 'no session file is recorded for it',
  prerequisite_unavailable: 'the session reader is unavailable',
  reader_unavailable: 'the session reader is unavailable',
  unsupported_host: 'no reader exists for this host',
  reader_limit: 'the session file is too large to read',
  reader_deadline: 'the session reader ran out of time',
  store_unsupported: 'this session is not stored in a readable file',
  reader_protocol: 'the session reader gave no usable answer',
};

/** The footer line, or nothing when the session has no person's message. */
function PromptLine({ prompt, window }: { prompt: DashboardSpanPrompt; window: DashboardWindow }) {
  if (prompt.state === 'no_message') return null;
  if (prompt.state === 'unclassified')
    return (
      <p className="xt-span-bubble-prompt" data-quiet>
        › last message unknown: not classified
      </p>
    );
  const when = recordedTime(prompt.at_ms, window);
  const earlier = prompt.in_span ? '' : 'before this span · ';
  if (prompt.text.state !== 'stored')
    return (
      <p className="xt-span-bubble-prompt" data-quiet title={`Typed ${when}`}>
        › {earlier}last message’s words{' '}
        {prompt.text.state === 'not_found'
          ? 'not found in the session file'
          : prompt.text.state === 'ambiguous'
            ? 'not shown: the session file holds differing copies'
            : prompt.text.state === 'wrapped'
              ? 'not shown (wrapped input)'
              : prompt.text.state === 'busy'
                ? 'not read yet: other session files are being read'
                : `unavailable: ${unreadable[prompt.text.reason.reason]}`}
      </p>
    );
  return (
    <p
      className="xt-span-bubble-prompt"
      title={`${prompt.in_span ? 'Typed' : 'Typed before this span,'} ${when}`}
    >
      › {earlier}
      {prompt.text.text}
      {prompt.text.truncated ? '…' : ''}
    </p>
  );
}

/**
 * The span's latest task notification: a line Claude Code wrote itself when a
 * background agent, command or monitor finished. It is automatic, so it is
 * labelled so, muted, and never marked `›` like the person's own message.
 * Only words that were read are shown: a notification whose summary is
 * unknown adds no line rather than another error.
 */
function AutomaticLine({
  automatic,
  window,
}: {
  automatic: DashboardSpanAutomatic;
  window: DashboardWindow;
}) {
  if (automatic.state !== 'found' || automatic.text.state !== 'stored') return null;
  return (
    <p
      className="xt-span-bubble-automatic"
      data-quiet
      title={`Written automatically by the agent, ${recordedTime(automatic.at_ms, window)}`}
    >
      (Automatic) {automatic.text.text}
      {automatic.text.truncated ? '…' : ''}
    </p>
  );
}

/** Whether the bubble has an automatic line to show. */
const hasAutomaticLine = (automatic: DashboardSpanAutomatic) =>
  automatic.state === 'found' && automatic.text.state === 'stored';

/** The meta row's right-hand fields, and the footer, in each load state. */
function Detail({
  detail,
  pending,
  failed,
}: {
  detail: DashboardSpanDetail | undefined;
  pending: boolean;
  failed: boolean;
}) {
  if (failed || (!pending && !detail))
    return <span className="xt-span-bubble-quiet">detail could not be read</span>;
  if (pending || !detail) return <span className="xt-span-bubble-quiet">reading…</span>;
  if (detail.state === 'missing')
    return <span className="xt-span-bubble-quiet">session not in the index</span>;
  return (
    <>
      <span
        className="xt-span-bubble-tool"
        data-quiet={detail.tool.state === 'called' ? undefined : ''}
        title={toolTitle(detail.tool)}
      >
        {toolText(detail.tool)}
      </span>
      <span
        className="xt-span-bubble-output"
        data-quiet={detail.output_tokens === null ? '' : undefined}
        title={
          detail.output_tokens === null
            ? 'No selected response in this span stated its output tokens'
            : `${detail.output_tokens.toLocaleString('en-US')} output tokens in this span`
        }
      >
        {detail.output_tokens === null ? (
          'output unmeasured'
        ) : (
          <>
            {formatTokens(detail.output_tokens)}
            <span className="sr-only"> output tokens</span>
          </>
        )}
      </span>
      <span
        className="xt-span-bubble-cost"
        data-quiet={costAmount(detail.cost) === null ? '' : undefined}
        title={`API-equivalent cost of recorded response usage in this span at public API prices: ${plural(detail.cost.priced_observations, 'response')} priced of ${plural(detail.cost.selected_observations, 'response')}${detail.cost.assumed_tier_observations > 0 ? `; ${plural(detail.cost.assumed_tier_observations, 'Codex response')} priced at the standard tier` : ''}${detail.cost.total_usd === null && detail.cost.priced_observations > 0 ? '; + means some responses could not be priced' : ''}`}
      >
        {costAmount(detail.cost) ?? 'cost unknown'}
      </span>
    </>
  );
}

/**
 * One active span drawn on a lane, which floats and opens a detail bubble
 * when pointed at or focused.
 *
 * The bubble's head is what the lane already knows: the session's name, the
 * span's length and its start. The rest — the span's most-used tool, its
 * output tokens, the last message a person typed in it or before it, and the
 * latest notification the agent wrote automatically inside it — is
 * read once, on first open, for exactly this span, and kept until committed
 * data changes; a read that has not answered says so, and an unknown is never
 * shown as zero.
 *
 * The bar is a named, focusable graphic like the compaction ticks drawn over
 * the same track, so a keyboard reaches the same bubble; the row's own
 * description still lists every span's times.
 */
export function LaneSpan({
  span,
  name,
  left,
  width,
  window,
}: {
  span: DashboardLane;
  /** The row's visible session name. */
  name: string;
  /** Position and width on the lane, 0–1. */
  left: number;
  width: number;
  window: DashboardWindow;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  const { source } = useData();
  const [open, setOpen] = useState(false);
  // Read only while the bubble is open, then kept: opening the same span
  // again reads nothing unless committed data invalidated `metrics` since,
  // and a closed bubble is never re-read in the background. A read still
  // running when the bubble closes, or its bar unmounts, is abandoned: the
  // native side may be reading the session's source for the words, and stops
  // at its next check. An answer whose words only waited for a read slot, or
  // were cancelled, is not kept, so the next open asks again.
  const controls = source.spanDetails;
  const client = useQueryClient();
  // Keyed by the purge epoch, so nothing read before "Delete stored content"
  // is shown after it, even an answer that lands later.
  const purges = usePurgeEpoch();
  const key = queryKeys.spanDetail(purges, span.session_id, span.start_ms, span.end_ms);
  const query = useQuery<DashboardSpanDetail>({
    queryKey: key,
    enabled: open && Boolean(controls),
    staleTime: (query) => (retryable(query.state.data) ? 0 : Infinity),
    // A live span grows: its new end is a new read, and until it answers the
    // open bubble keeps what the same span said a moment ago.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === purges ? previous : undefined,
    queryFn: async ({ signal }) => {
      const readId = `span-${Date.now().toString(36)}-${++reads}`;
      const cancel = () => {
        void controls!.cancel(readId).catch(() => {});
      };
      signal.addEventListener('abort', cancel, { once: true });
      try {
        return await controls!.read(span.session_id, span.start_ms, span.end_ms, readId);
      } finally {
        signal.removeEventListener('abort', cancel);
      }
    },
  });
  const onOpenChange = (next: boolean) => {
    setOpen(next);
    // Cancelling reverts the query to what it held before this read began.
    if (!next) void client.cancelQueries({ queryKey: key, exact: true });
  };
  const duration = spanDuration(span.start_ms, span.end_ms);
  const start = clockTime(span.start_ms, window, false);
  // Read aloud in words, as the timeline's stretches are; the bubble shows the short form.
  const label = `Active span ${clockTime(span.start_ms, window)} – ${clockTime(span.end_ms, window)}, ${spanDuration(span.start_ms, span.end_ms, true)}`;
  return (
    <Tooltip.Root open={open} onOpenChange={onOpenChange} disableHoverablePopup>
      <Tooltip.Trigger
        render={
          <span
            className="xt-lane-span"
            style={{ left: `${left * 100}%`, width: `${width * 100}%` }}
          />
        }
        tabIndex={0}
        // A named graphic: the bar has no text of its own for a name to label.
        role="img"
        aria-label={label}
        aria-describedby={open ? id : undefined}
        delay={80}
      />
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="top"
          align="center"
          sideOffset={8}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-span-bubble">
            <p className="xt-span-bubble-head">
              <i className="xt-span-bubble-swatch" aria-hidden="true" />
              <span className="xt-span-bubble-name">{name}</span>
              <span className="xt-span-bubble-duration">{duration}</span>
            </p>
            <p className="xt-span-bubble-meta">
              <time
                dateTime={new Date(span.start_ms).toISOString()}
                title={`Started ${recordedTime(span.start_ms, window)}`}
              >
                {start}
              </time>
              <Detail
                detail={query.data}
                pending={!controls ? false : query.isPending}
                failed={query.isError}
              />
            </p>
            {query.data?.state === 'indexed' &&
              (query.data.prompt.state !== 'no_message' ||
                hasAutomaticLine(query.data.automatic)) && (
                <>
                  <hr className="xt-span-bubble-rule" />
                  <PromptLine prompt={query.data.prompt} window={window} />
                  <AutomaticLine automatic={query.data.automatic} window={window} />
                </>
              )}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
