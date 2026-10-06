import { useQuery } from '@tanstack/react-query';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { Link, useParams, useSearchParams } from 'react-router';
import { useData } from '../../data/DataProvider';
import { queryKeys } from '../../data/query-client';
import type { MetricSessionStretches } from '../../data/generated/MetricSessionStretches';
import type { SessionPrLink } from '../../data/generated/SessionPrLink';
import type { SessionRow } from '../../data/generated/SessionRow';
import { EvidenceDot } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import { count, tokens as formatTokens } from '../../kit/format';
import { HostGlyph } from '../../kit/HostGlyph';
import { MetricCell } from '../../kit/MetricCell';
import { RulePopover } from '../../kit/RulePopover';
import type { RuleId } from '../../kit/rules';
import { SectionCard } from '../../kit/SectionCard';
import { plainReason, rangeDays } from '../dashboard/present';
import { DefinitionInfo } from '../dashboard/DefinitionInfo';
import { useSelectedRange } from '../dashboard/range';
import { continuous } from '../metric-format';
import { evidenceWords } from '../session-cells';
import { contextLead, contextTitle } from '../session-context';
import { listState } from '../session-search';
import { SessionTimeline, type TimelineQuery } from './SessionTimeline';
import { SessionTranscript } from './SessionTranscript';
import { type TranscriptProps, transcriptProps } from './transcript-adapter';
import { jumpUnavailableText, type JumpUnavailable, resolveJump } from './timeline-jump';
import type { RevealOutcome, RevealRequest } from './transcript-reveal';
import { type TranscriptRead, useSessionTranscript } from './useSessionTranscript';
import '../../styles/sessions.css';
import './session-detail.css';

/**
 * What the read said about the text's relationship to the numbers beside it.
 *
 * Kept apart from the transcript's own notices on purpose: a gap says part of
 * the text is missing, and this says the file has moved on since it was
 * measured. They are different facts and a reader can be told both.
 */
/** The read itself failed, which is this app's problem and not the file's. */
const FAILED = 'This session’s text could not be read just now.';

const GENERATION = {
  // Says what is true of the *measurements*, and nothing about whether the
  // text is whole. Whether it is whole is the transcript's own statement, made
  // beside this one — and a session can perfectly well have grown since it was
  // indexed *and* be missing part of itself, in which case a sentence here
  // calling the text complete would contradict the notice underneath it.
  appended:
    'This session has been written to since it was last indexed. The measurements beside it describe the part that was indexed.',
  unrecorded:
    'This text is not tied to a measurement: at least one of this session’s files has no recorded reading.',
} as const;

/** The measured columns, with the same definitions the list states (S2). */
const MEASURES: {
  key: string;
  label: string;
  rule: RuleId;
  value: (row: SessionRow) => number | null | undefined;
  format: (value: number) => string;
  reason: string;
}[] = [
  {
    key: 'human',
    label: 'Human messages',
    rule: 'M-02',
    value: (row) => indexed(row)?.human_messages,
    format: count,
    reason: plainReason('Human classification is unmeasured'),
  },
  {
    key: 'tokens',
    label: 'Tokens',
    rule: 'M-04',
    value: (row) => indexed(row)?.tokens.counters.total_tokens,
    format: formatTokens,
    reason: plainReason('Selected usage counters are absent or incomplete'),
  },
  {
    key: 'agent',
    label: 'Agent minutes',
    rule: 'M-05',
    value: (row) => {
      const metrics = indexed(row);
      return metrics ? metrics.agent_ms / 60000 : null;
    },
    format: continuous,
    reason: 'This session is not indexed, so nothing was measured',
  },
];

/**
 * The reader's last press on a stretch, and everything it was made against.
 *
 * A press means "the stretch at this index, in this list". It is only that
 * while all four still hold: the session, the window, the read of the
 * transcript it was resolved against, and **the accepted stretches response
 * itself** — an index is a place in one particular list, and a refetch that
 * returns a different or reordered list makes it a place in a list nobody is
 * looking at any more.
 *
 * It is checked twice, and both are needed. During render, so a replaced scope
 * never paints one frame of the old selection. And in an effect that **clears**
 * it, so it is gone rather than hidden: a press merely hidden while the window
 * is 30d comes back the moment the reader returns to 7d, pointing at whatever
 * now sits at that index.
 */
type Press =
  | {
      readonly kind: 'reveal';
      readonly index: number;
      readonly call: PressedCall;
      readonly tool: string;
      readonly request: RevealRequest;
      readonly scope: PressScope;
    }
  | {
      readonly kind: 'unavailable';
      readonly index: number;
      readonly call: PressedCall;
      readonly reason: JumpUnavailable;
      readonly scope: PressScope;
    };

/**
 * Which of a stretch's calls a press asked for: the one it started with (M-09),
 * or the earliest of its most repeated calls (M-20). Both are positions from
 * the same answer, confirmed against the same read, revealed the same way.
 */
type PressedCall = 'first' | 'repeated';

/** What a press on one call of one stretch did, in words, or nothing yet. */
function pressText(
  pressed: Press,
  result: { token: number; outcome: RevealOutcome } | null,
): string | null {
  const stretch = `stretch ${pressed.index + 1}`;
  const which = pressed.call === 'first' ? 'The first tool call' : 'The repeated call';
  if (pressed.kind === 'unavailable') {
    return `${which} of ${stretch} cannot be shown. ${jumpUnavailableText(pressed.reason)}`;
  }
  if (result?.token !== pressed.request.token) return null;
  if (result.outcome !== 'revealed') {
    return `${which} of ${stretch} could not be found in the transcript.`;
  }
  const tool = pressed.tool ? `: ${pressed.tool}` : '';
  return pressed.call === 'first'
    ? `Showing the first tool call of ${stretch}${tool}.`
    : `Showing the earliest of the repeated calls in ${stretch}${tool}.`;
}

/** What a press was made against. Replace any of it and the press is void. */
interface PressScope {
  readonly sessionId: string | undefined;
  readonly days: number;
  readonly read: TranscriptRead;
  /** The accepted answer the pressed index was an index into. */
  readonly stretches: MetricSessionStretches | undefined;
}

/** Whether a press still means what it meant when it was made. */
const holds = (press: Press | null, scope: PressScope) =>
  press !== null &&
  press.scope.sessionId === scope.sessionId &&
  press.scope.days === scope.days &&
  press.scope.read === scope.read &&
  press.scope.stretches === scope.stretches;

/** A row's measurements, or nothing when its session is not indexed (M-01). */
const indexed = (row: SessionRow) => (row.metrics.state === 'indexed' ? row.metrics : null);

/** A link's identity, its evidence and any stored title, in that order. */
const prName = (link: SessionPrLink) =>
  `${link.repository}#${link.number}, linked pull request, ${evidenceWords[link.confidence]} evidence` +
  (link.title ? `: ${link.title}` : '');

/**
 * Every pull request the index recorded for this session, with the list's own
 * badge. "Linked" is all a link says: that the session and the pull request
 * were tied together, on the evidence each one names — not who wrote it, not
 * that it merged, and not how much of it this session made. Nothing here
 * opens the network; the app has no page of its own for one pull request.
 */
function LinkedPrs({ links }: { links: SessionPrLink[] }) {
  const label = useId();
  if (links.length === 0) return null;
  return (
    <div className="xt-session-detail-prs">
      <span id={label} className="xt-session-detail-prs-label">
        {links.length === 1 ? 'Linked PR' : 'Linked PRs'}
      </span>
      <ul aria-labelledby={label}>
        {links.map((link) => (
          <li key={link.url}>
            <span
              className="xt-session-pr"
              data-confidence={link.confidence}
              role="img"
              aria-label={prName(link)}
              title={`${link.repository}#${link.number} · ${evidenceWords[link.confidence]} evidence${
                link.title ? ` · ${link.title}` : ''
              }`}
            >
              <EvidenceDot evidence={link.confidence} />
              <span className="xt-session-detail-pr-name">
                {link.repository}#{link.number}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * One session: what was measured about it, and what it actually said.
 *
 * The two halves are independent on purpose. The measurements come from the
 * index and are always shown; the text is read from the session's original
 * file on demand and may not be available at all. A file that has been
 * deleted, moved or rewritten takes nothing away from what was already
 * measured, and this page never lets one stand in for the other.
 */
export function SessionDetailPage() {
  const { sessionId } = useParams();
  const { source } = useData();
  const range = useSelectedRange();
  const days = rangeDays[range];
  // The list's filters travel in this page's own address, so going back
  // restores the list the reader left — the same search, hosts, pull-request
  // filter and range — rather than an unfiltered one. Nothing else is read.
  const [params] = useSearchParams();
  const back = listState(params);
  const backHref = back.size > 0 ? `/sessions?${back}` : '/sessions';

  // This session's own row, named exactly. It is not the list's search: that
  // matches a substring of the identity and answers with a bounded page of
  // whatever it matched, so a session whose identity is contained in enough
  // others could not be reached through it at all. Same read, same rules and
  // same snapshot as a listed row, so this page and the list it was opened
  // from cannot disagree about what the session measured.
  const listed = useQuery({
    queryKey: queryKeys.session(days, sessionId ?? ''),
    queryFn: () => source.sessionRow(sessionId ?? '', days),
    enabled: source.kind !== 'preview' && !!sessionId,
  });
  const row = listed.data ?? null;
  // Only an answer that is current says which pull requests are linked: while
  // the row is being read or read again, after a read failed or when there is
  // no such session, no badge is drawn rather than one that might not hold. A
  // re-read after ingest or a pull-request refresh can remove a link or change
  // its title, so the cached answer's links are not shown while it is out.
  const prLinks = listed.isSuccess && !listed.isFetching && row ? row.pr_links : [];
  const { read, retry } = useSessionTranscript(sessionId);

  // M-09's stretches for this session over this window. Metadata, cached like
  // the row above and invalidated with it; a change of range re-reads this and
  // never the transcript, whose read is keyed by the session alone.
  const stretchesQuery = useQuery({
    queryKey: queryKeys.stretches(days, sessionId ?? ''),
    queryFn: () => source.sessionStretches(sessionId ?? '', days),
    enabled: source.kind !== 'preview' && !!sessionId,
  });
  const timeline: TimelineQuery = stretchesQuery.isError
    ? { phase: 'failed', retry: () => void stretchesQuery.refetch() }
    : stretchesQuery.data
      ? { phase: 'ready', stretches: stretchesQuery.data }
      : { phase: 'loading' };
  const stretches = useMemo(
    () => (stretchesQuery.data?.state === 'measured' ? stretchesQuery.data.stretches : []),
    [stretchesQuery.data],
  );
  // Each stretch's first call, confirmed against the transcript as it was read
  // for this open — or the reason it cannot be.
  const jumps = useMemo(
    () => stretches.map((stretch) => resolveJump(stretch.first_tool, read)),
    [stretches, read],
  );
  // Each stretch's most repeated call (M-20), by the earliest of its calls, put
  // through the very same check — or `null` when M-20 names no repeated call.
  const repeatJumps = useMemo(
    () =>
      stretches.map((stretch) =>
        stretch.repeats.state === 'measured' && stretch.repeats.worst
          ? resolveJump(stretch.repeats.worst.representative, read)
          : null,
      ),
    [stretches, read],
  );

  const tokens = useRef(0);
  const [press, setPress] = useState<Press | null>(null);
  const [result, setResult] = useState<{ token: number; outcome: RevealOutcome } | null>(null);
  const answer = stretchesQuery.data;
  // Read during render, so a press whose scope has been replaced is never
  // painted, not even for the frame before the effect below runs.
  const pressed = holds(press, { sessionId, days, read, stretches: answer }) ? press : null;
  const selected = pressed?.index ?? null;
  const reveal = pressed?.kind === 'reveal' ? pressed.request : null;
  // And destroyed, so it cannot come back. Returning `current` unchanged when
  // it still holds leaves the state identical and re-renders nothing.
  useEffect(() => {
    setPress((current) =>
      holds(current, { sessionId, days, read, stretches: answer }) ? current : null,
    );
    // A result is only ever read beside a live press of the same token; with
    // the scope replaced there is none, so it goes with it.
    setResult(null);
  }, [sessionId, days, read, answer]);
  const pressCall = useCallback(
    (index: number, call: PressedCall) => {
      const jump = call === 'first' ? jumps[index] : repeatJumps[index];
      if (!jump) return;
      tokens.current += 1;
      // The scope this render resolved both lists against, recorded with the press.
      const scope: PressScope = { sessionId, days, read, stretches: answer };
      setPress(
        jump.kind === 'target'
          ? {
              kind: 'reveal',
              index,
              call,
              tool: jump.tool,
              request: { token: tokens.current, blockId: jump.blockId },
              scope,
            }
          : { kind: 'unavailable', index, call, reason: jump.reason, scope },
      );
    },
    [jumps, repeatJumps, sessionId, days, read, answer],
  );
  const onSelect = useCallback((index: number) => pressCall(index, 'first'), [pressCall]);
  const onShowRepeat = useCallback((index: number) => pressCall(index, 'repeated'), [pressCall]);
  // Stable, as the transcript asks. A late report for a replaced request is
  // recorded and then matches nothing below.
  const onReveal = useCallback(
    (token: number, outcome: RevealOutcome) => setResult({ token, outcome }),
    [],
  );
  const announcement = pressed ? pressText(pressed, result) : null;
  // A command that failed is not a session that cannot be shown, but it looks
  // the same to a reader, so it is said in the same place — with its own
  // sentence, and with the one thing a reader can do about it beside it.
  const transcript: TranscriptProps =
    read.phase === 'read'
      ? transcriptProps(read.status)
      : read.phase === 'failed'
        ? { records: [], state: { kind: 'unavailable', note: FAILED } }
        : { records: [], state: { kind: 'loading' } };
  const generation =
    read.phase === 'read' && read.status.state === 'loaded'
      ? read.status.generation.generation === 'unrecorded'
        ? GENERATION.unrecorded
        : read.status.generation.appended
          ? GENERATION.appended
          : null
      : null;

  if (source.kind === 'preview') {
    return (
      <section className="xt-session-detail">
        <p>Open the desktop app to read one session.</p>
      </section>
    );
  }

  return (
    <section className="xt-session-detail">
      <div className="xt-session-detail-heading">
        <Link className="xt-session-back" to={backHref}>
          ← All sessions
        </Link>
        <div className="xt-session-detail-title">
          {row && <HostGlyph host={row.host} size={20} />}
          <div>
            <h1 title={row ? contextTitle(row.repo, row.branch) : undefined}>
              {row ? contextLead(row.repo, row.branch) : 'Session'}
            </h1>
            <p className="xt-session-detail-meta">
              <span className="xt-session-mono">{sessionId}</span>
              {row && (
                <>
                  <span aria-hidden="true"> · </span>
                  <span>{row.model ?? 'Unknown model'}</span>
                </>
              )}
            </p>
          </div>
        </div>
        <LinkedPrs links={prLinks} />
      </div>

      {listed.isError ? (
        <p role="alert" className="xt-session-detail-notice">
          This session’s measurements could not be loaded.{' '}
          <button type="button" onClick={() => void listed.refetch()}>
            Try again
          </button>
        </p>
      ) : listed.isPending ? (
        <p role="status" className="xt-session-detail-notice">
          Reading this session’s measurements…
        </p>
      ) : !row ? (
        // Said plainly and without guessing: a session this list does not hold
        // is not a session with no measurements.
        <p role="status" className="xt-session-detail-notice">
          This Mac’s index holds no session with this identifier.{' '}
          <Link to={backHref}>Back to all sessions</Link>
        </p>
      ) : (
        <SectionCard
          title="Measured in this range"
          meta={`Records ${row.record_count.toLocaleString()} · measured over the last ${range.replace('d', ' days')}`}
        >
          <dl className="xt-session-measures">
            {MEASURES.map((measure) => (
              <div key={measure.key} className="xt-session-measure">
                <dt>
                  <RulePopover ruleId={measure.rule}>
                    <button
                      type="button"
                      className="xt-table-metric-header"
                      aria-label={`${measure.label} definition`}
                    >
                      {measure.label}
                    </button>
                  </RulePopover>
                </dt>
                <dd>
                  <MetricCell
                    value={measure.value(row)}
                    format={measure.format}
                    reason={measure.reason}
                  />
                </dd>
              </div>
            ))}
          </dl>
        </SectionCard>
      )}

      <SectionCard
        title="Hands-off stretches"
        meta={`Measured over the last ${range.replace('d', ' days')}`}
        // The definition sits behind the same compact control as the
        // Dashboard's cards; the rule ID is read only inside it.
        right={<DefinitionInfo ruleId="M-09" name="Hands-off stretches" />}
      >
        <SessionTimeline
          query={timeline}
          jumps={jumps}
          selected={selected}
          onSelect={onSelect}
          announcement={announcement}
          reason={pressed?.kind === 'unavailable' ? pressed.reason : undefined}
          repeatJumps={repeatJumps}
          onShowRepeat={onShowRepeat}
        />
      </SectionCard>

      <SectionCard
        title="Transcript"
        meta="Read from this session’s own file when you open it. Nothing is stored."
        right={
          read.phase === 'failed' ? (
            <Button variant="outline" height={28} onClick={retry}>
              Try again
            </Button>
          ) : undefined
        }
      >
        {generation && (
          <p className="xt-session-detail-notice" role="note">
            {generation}
          </p>
        )}
        <SessionTranscript
          records={transcript.records}
          state={transcript.state}
          reveal={reveal}
          onReveal={onReveal}
        />
      </SectionCard>
    </section>
  );
}
