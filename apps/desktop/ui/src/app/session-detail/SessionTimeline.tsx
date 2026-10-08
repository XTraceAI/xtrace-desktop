import type { MetricExcludedSurface } from '../../data/generated/MetricExcludedSurface';
import type { MetricRepeatThresholds } from '../../data/generated/MetricRepeatThresholds';
import type { MetricSessionStretch } from '../../data/generated/MetricSessionStretch';
import type { MetricUnknownRepeats } from '../../data/generated/MetricUnknownRepeats';
import type { MetricSessionStretches } from '../../data/generated/MetricSessionStretches';
import { Button } from '../../kit/Button';
import { clock, isInstant } from '../../kit/clock';
import { surfaceLabel } from '../../kit/hosts';
import { agentDuration } from '../agent-duration';
import { handsOffSpoken, handsOffTime } from '../metric-format';
import { jumpUnavailableText, type JumpTarget, type JumpUnavailable } from './timeline-jump';
import './session-detail.css';

/**
 * A stretch's length, from M-09's own duration, written as all hands-off time
 * is (`3.2 min`); `spoken` is the same value in words, for a label read aloud.
 *
 * Only ever from `duration_ms`. The endpoints are the native spellings as
 * stored, and a length re-derived from them could disagree with the one the
 * Dashboard's hands-off number was built from — which is exactly the number
 * this segment is meant to be one piece of.
 */
export function stretchDuration(ms: number, spoken = false): string {
  const minutes = ms / 60_000;
  return spoken ? handsOffSpoken(minutes) : handsOffTime(minutes);
}

/** When a stretch began, in the reader's own zone, or nothing if it cannot be read. */
export function stretchStart(iso: string): string | null {
  const when = new Date(iso).getTime();
  return isInstant(when) ? clock(when, { date: 'day' }) : null;
}

/** Why M-09 says nothing about this session, without claiming more than that. */
export function unmeasuredText(surface: MetricExcludedSurface | null): string {
  if (surface) {
    return (
      `Hands-off stretches are not measured for sessions on ${surfaceLabel(surface.host, surface.surface)} ` +
      `in this range: too many of its sessions record several turns at one instant to put them ` +
      `in order (${surface.degenerate_sessions} of ${surface.qualifying_sessions}). ` +
      'That is about how the surface records time, not about whether this session’s work happened.'
    );
  }
  // Both of the core's reasons for an unnamed surface, because it does not say
  // which one applied: a boundary it could not place, or a segment whose tool
  // use it could not establish.
  return (
    'Hands-off stretches are not measured for this session in this range: one of its turns ' +
    'could not be classified, so where a stretch begins or ends is not known, or whether a ' +
    'segment used a tool at all could not be established.'
  );
}

/**
 * Why a stretch's repeated calls were not counted (M-20), in a reader's words.
 *
 * Each names evidence that was missing. None of them is "no repeats": a count
 * made from part of a stretch would understate exactly what M-20 looks for.
 */
export function unknownRepeatsText(reason: MetricUnknownRepeats): string {
  switch (reason) {
    case 'call_count':
      return 'A turn in this stretch did not record how many tool calls it made.';
    case 'missing_blocks':
      return 'Not every tool call this stretch made is in the index.';
    case 'missing_key':
      return 'Some of its tool calls were indexed before this app compared calls, or their arguments could not be compared.';
    case 'unsupported_version':
      return 'Some of its tool calls were compared by a different version of this app.';
    case 'conflicting_key':
      return 'Two readings of this session disagree about which tool call was made at one place.';
  }
}

/**
 * A stretch's active time (M-05's fold over its records), written as all agent
 * time is (`0 h 5 m`). Unlike an elapsed length it can honestly be zero — every
 * gap in the stretch was a long wait — and a zero is said as one (`0 h 0 m`).
 */
export function activeDuration(ms: number, spoken = false): string {
  const duration = agentDuration(ms);
  return spoken ? duration.spoken : duration.visible;
}

/** Singular or plural, for a count a reader sees. */
const plural = (count: number, one: string, many: string) =>
  `${count.toLocaleString()} ${count === 1 ? one : many}`;

/**
 * What M-20 said about one stretch, in a few words under its length.
 *
 * Active time is labelled as active, because it is a different measurement
 * from the length above it. An unknown count says it is unknown — never 0.
 */
export function repeatSummary(stretch: MetricSessionStretch): string {
  const active = `${activeDuration(stretch.active_duration_ms)} active`;
  const repeats = stretch.repeats;
  if (repeats.state === 'unknown') return `${active} · repeats unknown`;
  if (repeats.repeats === 0) return `${active} · no repeats`;
  // A no-break space holds the tool and its count together when the line wraps.
  const worst = repeats.worst ? ` · ${repeats.worst.tool_name}\u00a0×${repeats.worst.count}` : '';
  return `${active} · ${plural(repeats.repeats, 'repeat', 'repeats')}${worst}`;
}

/**
 * The circling rule, from the thresholds the answer was judged against —
 * not from constants of this view — and without calling anybody wrong.
 */
export function circlingText(thresholds: MetricRepeatThresholds): string {
  return (
    `Circling marks a stretch with at least ${activeDuration(thresholds.active_ms, true)} of ` +
    `active time and at least ${plural(thresholds.repeats, 'repeat', 'repeats')}. A repeat is a ` +
    'call to the same tool with the same command, path or pattern as an earlier call in the ' +
    'stretch. Active time leaves out long waits, so it can be shorter than how long the stretch ' +
    'lasted. It describes the calls; it does not say the agent did anything wrong.'
  );
}

export const TIMELINE_TEXT = {
  loading: 'Reading this session’s hands-off stretches.',
  failed: 'This session’s hands-off stretches could not be read.',
  missing: 'This Mac’s index holds no session with this identifier, so no stretches were measured.',
  empty:
    'No hands-off stretches in this range. A stretch runs from a person’s message through the agent’s work until the next one, and needs at least one tool call.',
} as const;

export type TimelineQuery =
  | { readonly phase: 'loading' }
  | { readonly phase: 'failed'; readonly retry: () => void }
  | { readonly phase: 'ready'; readonly stretches: MetricSessionStretches };

/** What M-20 said about a stretch, spoken. */
function repeatLabel(stretch: MetricSessionStretch): string[] {
  const parts = [`${activeDuration(stretch.active_duration_ms, true)} of it active`];
  const repeats = stretch.repeats;
  if (repeats.state === 'unknown') {
    parts.push(
      `repeats not counted: ${unknownRepeatsText(repeats.reason)}`,
      'whether it was circling is not known',
    );
    return parts;
  }
  parts.push(repeats.repeats === 0 ? 'no repeats' : plural(repeats.repeats, 'repeat', 'repeats'));
  if (repeats.worst) {
    parts.push(
      `most repeated: ${repeats.worst.tool_name}, ${plural(repeats.worst.count, 'call', 'calls')}`,
    );
  }
  // Only what the metric answered, and never inferred from the count.
  parts.push(
    stretch.circling === null
      ? 'whether it was circling is not known'
      : stretch.circling
        ? 'circling'
        : 'not circling',
  );
  return parts;
}

/** What one segment is, spoken: where it is, when, how long, and what pressing it does. */
function segmentLabel(
  stretch: MetricSessionStretch,
  index: number,
  total: number,
  jump: JumpTarget,
): string {
  const start = stretchStart(stretch.start);
  const parts = [
    `Stretch ${index + 1} of ${total}`,
    start ? `started ${start}` : 'start time not readable',
    `lasted ${stretchDuration(stretch.duration_ms, true)}`,
    ...repeatLabel(stretch),
  ];
  parts.push(
    jump.kind === 'target'
      ? 'shows its first tool call in the transcript'
      : `its first tool call cannot be shown: ${jumpUnavailableText(jump.reason)}`,
  );
  return parts.join(', ');
}

/**
 * One session's M-09 hands-off stretches, above its transcript.
 *
 * Each segment is one of the segments the Dashboard's hands-off number was made
 * of, in the order M-09 states them, sized by its own share of the longest —
 * from the supplied durations only, with no time axis re-derived from the
 * endpoints. A segment is a native button: pressing it selects it and, when
 * its first tool call can be confirmed in the loaded transcript, reveals that
 * call. When it cannot, the segment stays exactly where it is and says why: a
 * measured stretch is still a measured stretch.
 *
 * The four states that are not a list are four different facts, and none of
 * them is worded as another.
 *
 * Each segment also says what M-20 measured about it: its active time, how
 * many of its calls repeated an earlier one, and which tool repeated most.
 * Only a stretch M-20 judged circling is set apart; one whose repeats were not
 * counted says so and is drawn like any other, because not knowing is not a
 * finding. The order stays M-09's — nothing is ranked by repeats. When the
 * selected stretch has a most-repeated call, a reader can go to the earliest
 * of those calls through the same confirmed-position reveal as its first call.
 */
export function SessionTimeline({
  query,
  jumps,
  selected,
  onSelect,
  announcement,
  reason,
  repeatJumps = [],
  onShowRepeat,
}: {
  query: TimelineQuery;
  /** Whether each measured stretch's first call can be shown, in stretch order. */
  jumps: readonly JumpTarget[];
  selected: number | null;
  onSelect: (index: number) => void;
  /**
   * Whether each stretch's most repeated call can be shown, in stretch order;
   * `null` for a stretch that names no repeated call.
   */
  repeatJumps?: readonly (JumpTarget | null)[];
  /** Go to the selected stretch's most repeated call. */
  onShowRepeat?: (index: number) => void;
  /** What the last press did, shown and announced. */
  announcement: string | null;
  /** Why the selected stretch's call cannot be shown, when it cannot. */
  reason?: JumpUnavailable;
}) {
  // One node, seen and announced. What a press did — and above all why
  // nothing happened — belongs on the screen: a reason carried only in a
  // segment's accessible name is a reason a sighted reader never receives.
  // It is polite, so it is spoken when it changes rather than interrupting.
  // Always mounted, so a screen reader is already watching it when its text
  // changes; a region that appears together with its words is announced far
  // less reliably. Empty, it takes no room — see `.xt-timeline-note:empty`.
  const live = (
    <p className="xt-timeline-note" role="status" aria-live="polite" data-reason={reason}>
      {announcement ?? ''}
    </p>
  );

  if (query.phase === 'loading') {
    return (
      <div className="xt-timeline" data-state="loading">
        <p className="xt-session-detail-notice" role="status">
          {TIMELINE_TEXT.loading}
        </p>
      </div>
    );
  }
  if (query.phase === 'failed') {
    return (
      <div className="xt-timeline" data-state="failed">
        <p className="xt-session-detail-notice" role="alert">
          {TIMELINE_TEXT.failed}{' '}
          <Button variant="outline" height={28} onClick={query.retry}>
            Try again
          </Button>
        </p>
      </div>
    );
  }

  const answer = query.stretches;
  if (answer.state === 'missing') {
    return (
      <div className="xt-timeline" data-state="missing">
        <p className="xt-session-detail-notice">{TIMELINE_TEXT.missing}</p>
      </div>
    );
  }
  if (answer.state === 'unmeasured') {
    return (
      <div className="xt-timeline" data-state="unmeasured">
        <p className="xt-session-detail-notice">{unmeasuredText(answer.excluded_surface)}</p>
      </div>
    );
  }
  if (answer.stretches.length === 0) {
    return (
      <div className="xt-timeline" data-state="empty">
        <p className="xt-session-detail-notice">{TIMELINE_TEXT.empty}</p>
      </div>
    );
  }

  const stretches = answer.stretches;
  const longest = Math.max(...stretches.map((stretch) => stretch.duration_ms), 1);
  const chosen = selected === null ? undefined : stretches[selected];
  const worst =
    chosen?.repeats.state === 'measured' && chosen.repeats.worst ? chosen.repeats.worst : null;
  const repeatJump = selected === null ? null : (repeatJumps[selected] ?? null);
  return (
    <div className="xt-timeline" data-state="measured">
      <ol className="xt-timeline-segments" aria-label="Hands-off stretches, in order">
        {stretches.map((stretch, index) => {
          const jump = jumps[index] ?? { kind: 'unavailable', reason: 'transcript_loading' };
          const start = stretchStart(stretch.start);
          return (
            // Position-keyed: two stretches can share a start record in principle, and a
            // duplicate key would silently drop one.
            <li key={`${index}:${stretch.start_uuid}`}>
              <button
                type="button"
                className="xt-timeline-segment"
                aria-pressed={selected === index}
                aria-label={segmentLabel(stretch, index, stretches.length, jump)}
                data-jump={jump.kind === 'target' ? 'available' : jump.reason}
                // Only the metric's own `true` sets a stretch apart. Unknown is
                // not "not circling" and is not drawn as either.
                data-circling={stretch.circling === true ? 'true' : undefined}
                data-repeats={stretch.repeats.state}
                onClick={() => onSelect(index)}
              >
                <span className="xt-timeline-start">{start ?? 'Time not readable'}</span>
                <span className="xt-timeline-duration">{stretchDuration(stretch.duration_ms)}</span>
                <span className="xt-timeline-repeats">{repeatSummary(stretch)}</span>
                {stretch.circling === true && (
                  <span className="xt-timeline-circling">Circling</span>
                )}
                <span
                  className="xt-timeline-bar"
                  aria-hidden="true"
                  // Share of the longest stretch, from the supplied durations alone.
                  style={{ inlineSize: `${Math.max((stretch.duration_ms / longest) * 100, 2)}%` }}
                />
              </button>
            </li>
          );
        })}
      </ol>
      {selected !== null && chosen && (
        <div className="xt-timeline-repeat" data-repeats={chosen.repeats.state}>
          <p>
            {chosen.repeats.state === 'unknown'
              ? `Stretch ${selected + 1}: repeats not counted. ${unknownRepeatsText(chosen.repeats.reason)}`
              : worst
                ? `Stretch ${selected + 1}: the most repeated call was ${worst.tool_name}, ${plural(worst.count, 'call', 'calls')} with the same argument.`
                : `Stretch ${selected + 1}: no call repeated an earlier one.`}
          </p>
          {worst && onShowRepeat && (
            <Button
              variant="outline"
              height={28}
              aria-label={
                repeatJump?.kind === 'target'
                  ? `Show repeated call: ${worst.tool_name}, stretch ${selected + 1}`
                  : `Show repeated call: ${worst.tool_name}, stretch ${selected + 1}. It cannot be shown: ${jumpUnavailableText(repeatJump?.reason ?? 'transcript_loading')}`
              }
              data-jump={repeatJump?.kind === 'target' ? 'available' : repeatJump?.reason}
              onClick={() => onShowRepeat(selected)}
            >
              Show repeated call
            </Button>
          )}
        </div>
      )}
      {live}
      <p className="xt-timeline-rule">{circlingText(answer.repeat_thresholds)}</p>
    </div>
  );
}
