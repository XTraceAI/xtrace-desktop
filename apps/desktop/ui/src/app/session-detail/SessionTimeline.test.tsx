import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { MetricSessionStretch } from '../../data/generated/MetricSessionStretch';
import {
  activeDuration,
  circlingText,
  SessionTimeline,
  stretchDuration,
  stretchStart,
  TIMELINE_TEXT,
  type TimelineQuery,
  unknownRepeatsText,
  unmeasuredText,
} from './SessionTimeline';
import type { JumpTarget } from './timeline-jump';

afterEach(cleanup);

/** Synthetic throughout. */
const stretch = (
  start: string,
  duration_ms: number,
  end = start,
  first_tool: MetricSessionStretch['first_tool'] = null,
): MetricSessionStretch => ({
  start_uuid: `start-${start}`,
  end_uuid: `end-${start}`,
  start,
  end,
  duration_ms,
  first_tool,
  // M-20 over the same stretch: measured, nothing repeated.
  active_duration_ms: duration_ms,
  repeats: { state: 'measured', repeats: 0, worst: null },
  circling: false,
});

/** The metric's default thresholds, as the answer carries them. */
const THRESHOLDS = { active_ms: 240_000, repeats: 5 };

const target = (blockId: string): JumpTarget => ({ kind: 'target', blockId, tool: 'Read' });

function show(query: TimelineQuery, jumps: JumpTarget[] = [], selected: number | null = null) {
  const onSelect = vi.fn();
  render(
    <SessionTimeline
      query={query}
      jumps={jumps}
      selected={selected}
      onSelect={onSelect}
      announcement={null}
    />,
  );
  return onSelect;
}

it('says it is reading, and draws no segment in the meantime', () => {
  show({ phase: 'loading' });
  expect(screen.getByText(TIMELINE_TEXT.loading)).toBeTruthy();
  expect(screen.queryAllByRole('button')).toHaveLength(0);
});

it('says a failed read failed, and offers to ask again', () => {
  const retry = vi.fn();
  show({ phase: 'failed', retry });
  expect(screen.getByRole('alert').textContent).toContain(TIMELINE_TEXT.failed);
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  expect(retry).toHaveBeenCalledOnce();
});

it('says an identifier no session owns has no stretches, not that it had none', () => {
  show({ phase: 'ready', stretches: { state: 'missing' } });
  expect(screen.getByText(TIMELINE_TEXT.missing)).toBeTruthy();
  expect(screen.queryByText(TIMELINE_TEXT.empty)).toBeNull();
});

it('names an excluded surface as the reason, without claiming the session’s times are absent', () => {
  show({
    phase: 'ready',
    stretches: {
      state: 'unmeasured',
      excluded_surface: {
        host: 'claude',
        surface: 'raw-batched',
        qualifying_sessions: 5,
        degenerate_sessions: 3,
      },
    },
  });
  const said = unmeasuredText({
    host: 'claude',
    surface: 'raw-batched',
    qualifying_sessions: 5,
    degenerate_sessions: 3,
  });
  expect(screen.getByText(said)).toBeTruthy();
  expect(said).toContain('claude raw-batched surface');
  expect(said).toContain('(3 of 5)');
  // The surface's health is what excluded it. Nothing here says this session's
  // own timestamps are missing, absent or unrecorded.
  expect(said).not.toMatch(/missing|absent|no timestamp|unrecorded/i);
  // An unlabelled surface is named as unlabelled rather than as nothing.
  expect(
    unmeasuredText({
      host: 'codex',
      surface: null,
      qualifying_sessions: 3,
      degenerate_sessions: 3,
    }),
  ).toContain('codex unlabelled surface');
});

it('says a session unmeasured on its own records is unmeasured for either reason', () => {
  // The core does not say which of the two applied — a boundary it could not
  // place, or a segment whose tool use it could not establish — so the
  // sentence must not pick one.
  show({ phase: 'ready', stretches: { state: 'unmeasured', excluded_surface: null } });
  const said = unmeasuredText(null);
  expect(screen.getByText(said)).toBeTruthy();
  expect(said).toContain('could not be classified');
  expect(said).toContain('used a tool');
});

it('says a measured session with no stretch in the range had none, as a measurement', () => {
  show({
    phase: 'ready',
    stretches: { state: 'measured', stretches: [], repeat_thresholds: THRESHOLDS },
  });
  expect(screen.getByText(TIMELINE_TEXT.empty)).toBeTruthy();
  expect(screen.queryAllByRole('button')).toHaveLength(0);
});

it('lays the stretches out in the order M-09 stated them, never re-sorted', () => {
  // Given out of time order on purpose: the order is the metric's to state.
  const stretches = [
    stretch('2026-09-07T12:30:00Z', 60_000),
    stretch('2026-09-07T12:00:00Z', 120_000),
    stretch('2026-09-07T12:15:00Z', 180_000),
  ];
  show(
    { phase: 'ready', stretches: { state: 'measured', stretches, repeat_thresholds: THRESHOLDS } },
    [target('a:0'), target('b:0'), target('c:0')],
  );
  const labels = screen.getAllByRole('button').map((button) => button.getAttribute('aria-label'));
  expect(labels.map((label) => label?.match(/lasted (\d+ min)/)?.[1])).toEqual([
    '1 min',
    '2 min',
    '3 min',
  ]);
});

it('states each stretch’s length from its own duration, never from its endpoints', () => {
  // The endpoints are ten minutes apart; M-09's duration is ninety seconds.
  // The segment says ninety seconds, because that is the number the
  // Dashboard's hands-off figure was built from.
  show(
    {
      phase: 'ready',
      stretches: {
        state: 'measured',
        stretches: [stretch('2026-09-07T12:00:00Z', 90_000, '2026-09-07T12:10:00Z')],
        repeat_thresholds: THRESHOLDS,
      },
    },
    [target('a:0')],
  );
  const button = screen.getByRole('button');
  expect(within(button).getByText('1 min 30 s')).toBeTruthy();
  expect(button.getAttribute('aria-label')).toContain('lasted 1 min 30 s');
  expect(button.textContent).not.toContain('10 min');
});

it('makes every segment a native button that says when, how long, and what pressing it does', () => {
  const stretches = [
    stretch('2026-09-07T12:00:00Z', 60_000),
    stretch('2026-09-07T12:05:00Z', 60_000),
  ];
  const onSelect = show(
    { phase: 'ready', stretches: { state: 'measured', stretches, repeat_thresholds: THRESHOLDS } },
    [target('a:0'), { kind: 'unavailable', reason: 'no_locator' }],
    1,
  );
  const [first, second] = screen.getAllByRole('button');
  for (const button of [first, second]) {
    expect(button.tagName).toBe('BUTTON');
    expect(button.getAttribute('type')).toBe('button');
    expect(button.getAttribute('tabindex')).toBeNull();
  }
  expect(first.getAttribute('aria-pressed')).toBe('false');
  expect(second.getAttribute('aria-pressed')).toBe('true');
  expect(first.getAttribute('aria-label')).toMatch(
    /^Stretch 1 of 2, started .+, lasted 1 min, 1 min of it active, no repeats, not circling, shows its first tool call in the transcript$/,
  );
  expect(first.getAttribute('aria-label')).toContain(stretchStart('2026-09-07T12:00:00Z')!);
  // A stretch whose call cannot be shown is still a measured stretch, still a
  // button, and says why.
  expect(second.getAttribute('aria-label')).toContain(
    'its first tool call cannot be shown: Which tool call this stretch started with was not recorded.',
  );
  fireEvent.click(second);
  expect(onSelect).toHaveBeenCalledWith(1);
});

it('sizes each bar by its share of the longest stretch, from the durations alone', () => {
  const stretches = [
    stretch('2026-09-07T12:00:00Z', 30_000),
    stretch('2026-09-07T12:05:00Z', 120_000),
  ];
  const { container } = render(
    <SessionTimeline
      query={{
        phase: 'ready',
        stretches: { state: 'measured', stretches, repeat_thresholds: THRESHOLDS },
      }}
      jumps={[target('a:0'), target('b:0')]}
      selected={null}
      onSelect={() => {}}
      announcement={null}
    />,
  );
  const bars = [...container.querySelectorAll<HTMLElement>('.xt-timeline-bar')];
  expect(bars.map((bar) => bar.style.inlineSize)).toEqual(['25%', '100%']);
  // Decorative: the length is in the label, not in a shape a reader cannot see.
  expect(bars.every((bar) => bar.getAttribute('aria-hidden') === 'true')).toBe(true);
});

it('shows the reason a call cannot be jumped to, as well as announcing it', () => {
  // A reason carried only in a segment's accessible name is a reason a sighted
  // reader never receives.
  render(
    <SessionTimeline
      query={{
        phase: 'ready',
        stretches: {
          state: 'measured',
          stretches: [stretch('2026-09-07T12:00:00Z', 60_000)],
          repeat_thresholds: THRESHOLDS,
        },
      }}
      jumps={[{ kind: 'unavailable', reason: 'no_locator' }]}
      selected={0}
      onSelect={() => {}}
      announcement="The first tool call of stretch 1 cannot be shown. Which tool call this stretch started with was not recorded."
      reason="no_locator"
    />,
  );
  const note = screen.getByRole('status');
  expect(note.textContent).toContain('was not recorded');
  expect(note.getAttribute('aria-live')).toBe('polite');
  expect(note.getAttribute('data-reason')).toBe('no_locator');
  // Not hidden from sight, and not hidden from the accessibility tree either.
  expect(note.className).toBe('xt-timeline-note');
  expect(note.getAttribute('aria-hidden')).toBeNull();
});

it('keeps the region in the document with nothing to say, so it is watched before it speaks', () => {
  render(
    <SessionTimeline
      query={{
        phase: 'ready',
        stretches: {
          state: 'measured',
          stretches: [stretch('2026-09-07T12:00:00Z', 60_000)],
          repeat_thresholds: THRESHOLDS,
        },
      }}
      jumps={[target('a:0')]}
      selected={null}
      onSelect={() => {}}
      announcement={null}
    />,
  );
  const note = screen.getByRole('status');
  expect(note.textContent).toBe('');
  expect(note.getAttribute('data-reason')).toBeNull();
});

it('announces what the last press did in a polite live region', () => {
  render(
    <SessionTimeline
      query={{
        phase: 'ready',
        stretches: {
          state: 'measured',
          stretches: [stretch('2026-09-07T12:00:00Z', 60_000)],
          repeat_thresholds: THRESHOLDS,
        },
      }}
      jumps={[target('a:0')]}
      selected={0}
      onSelect={() => {}}
      announcement="Showing the first tool call of stretch 1: Read."
    />,
  );
  const live = screen.getByRole('status');
  expect(live.getAttribute('aria-live')).toBe('polite');
  expect(live.textContent).toBe('Showing the first tool call of stretch 1: Read.');
});

it('words a duration at the scale it has, and never as zero', () => {
  expect(stretchDuration(400)).toBe('1 s');
  expect(stretchDuration(45_000)).toBe('45 s');
  expect(stretchDuration(60_000)).toBe('1 min');
  expect(stretchDuration(90_000)).toBe('1 min 30 s');
  expect(stretchDuration(3_600_000)).toBe('1 h');
  expect(stretchDuration(5_400_000)).toBe('1 h 30 min');
});

it('says a start time it cannot read is unreadable rather than inventing one', () => {
  expect(stretchStart('not-a-time')).toBe(null);
  show(
    {
      phase: 'ready',
      stretches: {
        state: 'measured',
        stretches: [stretch('not-a-time', 60_000)],
        repeat_thresholds: THRESHOLDS,
      },
    },
    [target('a:0')],
  );
  expect(screen.getByText('Time not readable')).toBeTruthy();
  expect(screen.getByRole('button').getAttribute('aria-label')).toContain(
    'start time not readable',
  );
});

/* M-20 on each segment. What the answer says, and nothing it did not. */

const measuredWith = (
  stretches: MetricSessionStretch[],
  repeat_thresholds = THRESHOLDS,
): TimelineQuery => ({
  phase: 'ready',
  stretches: { state: 'measured', stretches, repeat_thresholds },
});

const with20 = (
  base: MetricSessionStretch,
  m20: Pick<MetricSessionStretch, 'active_duration_ms' | 'repeats' | 'circling'>,
): MetricSessionStretch => ({ ...base, ...m20 });

const worst = (tool_name: string, count: number) => ({
  tool_name,
  count,
  representative: { record_uuid: 'rec', block_index: 0 },
});

it('says a stretch whose calls all differed had no repeats, as a measurement', () => {
  show(measuredWith([stretch('2026-09-07T12:00:00Z', 300_000)]), [target('a:0')]);
  const [segment] = screen.getAllByRole('button');
  expect(within(segment).getByText('5 min active · no repeats')).toBeTruthy();
  expect(segment.getAttribute('aria-label')).toContain(', no repeats, not circling,');
  expect(segment.getAttribute('data-circling')).toBeNull();
  expect(within(segment).queryByText('Circling')).toBeNull();
});

it.each([
  ['call_count', 'did not record how many tool calls it made'],
  ['missing_blocks', 'Not every tool call this stretch made is in the index'],
  ['missing_key', 'indexed before this app compared calls'],
  ['unsupported_version', 'compared by a different version of this app'],
  ['conflicting_key', 'disagree about which tool call was made at one place'],
] as const)(
  'says repeats are unknown for %s — never zero, never “not circling”',
  (reason, words) => {
    expect(unknownRepeatsText(reason)).toContain(words);
    show(
      measuredWith([
        with20(stretch('2026-09-07T12:00:00Z', 600_000), {
          active_duration_ms: 600_000,
          repeats: { state: 'unknown', reason },
          circling: null,
        }),
      ]),
      [target('a:0')],
      0,
    );
    const [segment] = screen.getAllByRole('button', { name: /^Stretch/ });
    expect(within(segment).getByText('10 min active · repeats unknown')).toBeTruthy();
    const label = segment.getAttribute('aria-label')!;
    expect(label).toContain(`repeats not counted: ${unknownRepeatsText(reason)}`);
    expect(label).toContain('whether it was circling is not known');
    expect(label).not.toMatch(/no repeats|not circling|\b0 repeats/);
    expect(segment.textContent).not.toMatch(/no repeats|\b0\b/);
    expect(segment.getAttribute('data-circling')).toBeNull();
    // The selected stretch says so in words, and offers no call to go to.
    expect(
      screen.getByText(`Stretch 1: repeats not counted. ${unknownRepeatsText(reason)}`),
    ).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  },
);

it('sets apart only the stretches the metric judged circling, and keeps M-09’s order', () => {
  // The metric's own answer is the only thing read. The second stretch has
  // more repeats and more active time than the first yet was answered `false`
  // — the view neither recomputes nor second-guesses it — and the third's
  // `null` is drawn like any other stretch.
  const stretches = [
    with20(stretch('2026-09-07T12:00:00Z', 240_000), {
      active_duration_ms: 240_000,
      repeats: { state: 'measured', repeats: 5, worst: worst('Edit', 6) },
      circling: true,
    }),
    with20(stretch('2026-09-07T11:00:00Z', 900_000), {
      active_duration_ms: 900_000,
      repeats: { state: 'measured', repeats: 20, worst: worst('Bash', 21) },
      circling: false,
    }),
    with20(stretch('2026-09-07T13:00:00Z', 900_000), {
      active_duration_ms: 900_000,
      repeats: { state: 'unknown', reason: 'missing_key' },
      circling: null,
    }),
  ];
  show(measuredWith(stretches), [target('a:0'), target('b:0'), target('c:0')]);
  const segments = screen.getAllByRole('button');
  expect(segments.map((segment) => segment.getAttribute('data-circling'))).toEqual([
    'true',
    null,
    null,
  ]);
  expect(segments.map((segment) => within(segment).queryByText('Circling') !== null)).toEqual([
    true,
    false,
    false,
  ]);
  // Never re-ranked by repeats: the order is the one the metric stated.
  expect(
    segments.map((segment) => segment.getAttribute('aria-label')?.match(/lasted ([^,]+)/)?.[1]),
  ).toEqual(['4 min', '15 min', '15 min']);
  expect(segments[0].getAttribute('aria-label')).toContain(
    '4 min of it active, 5 repeats, most repeated: Edit, 6 calls, circling,',
  );
  expect(segments[1].getAttribute('aria-label')).toContain(', not circling,');
});

it('names the most repeated call by its tool and count only', () => {
  show(
    measuredWith([
      with20(stretch('2026-09-07T12:00:00Z', 360_000), {
        active_duration_ms: 360_000,
        repeats: { state: 'measured', repeats: 8, worst: worst('Edit', 9) },
        circling: true,
      }),
    ]),
    [target('a:0')],
  );
  const [segment] = screen.getAllByRole('button');
  expect(within(segment).getByText('6 min active · 8 repeats · Edit ×9')).toBeTruthy();
  // The representative is a position for the reveal, not something to show.
  expect(segment.textContent).not.toContain('rec');
  expect(segment.getAttribute('aria-label')).not.toContain('rec');
});

it('states active time apart from the stretch’s length, including an honest zero', () => {
  show(
    measuredWith([
      // An hour's wait inside: elapsed is long, active is short.
      with20(stretch('2026-09-07T12:00:00Z', 3_660_000), {
        active_duration_ms: 60_000,
        repeats: { state: 'measured', repeats: 19, worst: worst('Edit', 20) },
        circling: false,
      }),
      // Every gap was a long wait: no active time at all.
      with20(stretch('2026-09-07T14:00:00Z', 2_400_000), {
        active_duration_ms: 0,
        repeats: { state: 'measured', repeats: 0, worst: null },
        circling: false,
      }),
    ]),
    [target('a:0'), target('b:0')],
  );
  const [idle, none] = screen.getAllByRole('button');
  expect(within(idle).getByText('1 h 1 min')).toBeTruthy();
  expect(within(idle).getByText('1 min active · 19 repeats · Edit ×20')).toBeTruthy();
  expect(idle.getAttribute('aria-label')).toContain('lasted 1 h 1 min, 1 min of it active');
  expect(activeDuration(0)).toBe('0 s');
  expect(within(none).getByText('0 s active · no repeats')).toBeTruthy();
});

it('explains circling from the thresholds the answer carried, without calling anyone wrong', () => {
  show(
    measuredWith([stretch('2026-09-07T12:00:00Z', 60_000)], { active_ms: 300_000, repeats: 7 }),
    [target('a:0')],
  );
  const said = circlingText({ active_ms: 300_000, repeats: 7 });
  expect(screen.getByText(said)).toBeTruthy();
  expect(said).toContain('at least 5 min of active time and at least 7 repeats');
  expect(said).toContain('same tool with the same command, path or pattern');
  expect(said).toContain('it does not say the agent did anything wrong');
  expect(circlingText({ active_ms: 240_000, repeats: 1 })).toContain('at least 1 repeat.');
  // The rule is said in words; its ID is not visible chrome.
  expect(said).toMatch(/^Circling marks a stretch/);
  expect(said).not.toMatch(/\b[MRC]-\d{2}\b/);
  // Stated only beside measured stretches.
  cleanup();
  show({ phase: 'ready', stretches: { state: 'missing' } });
  expect(screen.queryByText(/^Circling marks a stretch/)).toBeNull();
});

it('offers the selected stretch’s repeated call, from the keyboard’s own element', () => {
  const onShowRepeat = vi.fn();
  const stretches = [
    with20(stretch('2026-09-07T12:00:00Z', 300_000), {
      active_duration_ms: 300_000,
      repeats: { state: 'measured', repeats: 6, worst: worst('Bash', 7) },
      circling: true,
    }),
    stretch('2026-09-07T13:00:00Z', 300_000),
  ];
  const view = (selected: number | null, repeatJump: JumpTarget | null) =>
    render(
      <SessionTimeline
        query={measuredWith(stretches)}
        jumps={[target('a:0'), target('b:0')]}
        selected={selected}
        onSelect={() => {}}
        announcement={null}
        repeatJumps={[repeatJump, null]}
        onShowRepeat={onShowRepeat}
      />,
    );
  // Nothing selected: nothing offered.
  view(null, target('a:2'));
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  cleanup();

  view(0, target('a:2'));
  expect(
    screen.getByText('Stretch 1: the most repeated call was Bash, 7 calls with the same argument.'),
  ).toBeTruthy();
  const button = screen.getByRole('button', { name: 'Show repeated call: Bash, stretch 1' });
  expect(button.tagName).toBe('BUTTON');
  button.focus();
  fireEvent.click(document.activeElement as HTMLElement);
  expect(onShowRepeat).toHaveBeenCalledWith(0);
  expect(document.activeElement).toBe(button);
  cleanup();

  // A call that cannot be shown is still offered, and says why before it is pressed.
  view(0, { kind: 'unavailable', reason: 'transcript_unavailable' });
  expect(
    screen
      .getByRole('button', { name: /^Show repeated call: Bash, stretch 1\. It cannot be shown/ })
      .getAttribute('aria-label'),
  ).toContain('The transcript is not available to show.');
  cleanup();

  // A stretch whose calls all differed names no call to go to.
  view(1, null);
  expect(screen.getByText('Stretch 2: no call repeated an earlier one.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
});
