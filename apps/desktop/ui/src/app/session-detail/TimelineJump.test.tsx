import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import { events, type DataEvent } from '../../data/ipc-names';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { MetricSessionStretch } from '../../data/generated/MetricSessionStretch';
import type { MetricSessionStretches } from '../../data/generated/MetricSessionStretches';
import type { MetricToolBlock } from '../../data/generated/MetricToolBlock';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { SourceRecord } from '../../data/generated/SourceRecord';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

/**
 * A stretch's first tool call, from the timeline to the transcript, on the real
 * page. Every identity, record and stretch here is written by the test.
 */
const exported = fixture as FixtureExport;
const template = exported.sessions[0].rows[0];
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));

const SESSION = '11111111-0000-4000-8000-000000000001';
const OTHER_SESSION = '11111111-0000-4000-8000-000000000002';
const CALLER = '22222222-2222-4222-8222-222222222222';
const LATER = '33333333-3333-4333-8333-333333333333';

let scrolled: { element: Element; options: unknown }[];
beforeEach(() => {
  scrolled = [];
  // jsdom lays nothing out and has no scrolling; record what would have been
  // scrolled, and on which element.
  Element.prototype.scrollIntoView = function (this: Element, options?: unknown) {
    scrolled.push({ element: this, options });
  } as Element['scrollIntoView'];
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
  delete (Element.prototype as Partial<Element>).scrollIntoView;
  delete (window as Partial<Window>).matchMedia;
});

const records = (caller = CALLER): SourceRecord[] => [
  {
    id: caller,
    role: 'assistant',
    at: null,
    blocks: [
      { kind: 'text', index: 0, text: 'Reading and running.' },
      { kind: 'tool_call', index: 1, name: 'Read', call_id: 'toolu_1', input: null },
      { kind: 'tool_call', index: 2, name: 'Bash', call_id: 'toolu_2', input: null },
    ],
  },
  {
    id: LATER,
    role: 'assistant',
    at: null,
    blocks: [{ kind: 'tool_call', index: 0, name: 'Grep', call_id: 'toolu_3', input: null }],
  },
];

const loaded = (list: SourceRecord[] = records()): SessionSourceStatus => ({
  state: 'loaded',
  generation: { generation: 'indexed', appended: false },
  sources: 1,
  dropped_records: 0,
  gaps: [],
  records: list,
});

const stretch = (
  minute: number,
  first_tool: MetricSessionStretch['first_tool'],
): MetricSessionStretch => ({
  start_uuid: `start-${minute}`,
  end_uuid: `end-${minute}`,
  start: `2026-09-07T12:${String(minute).padStart(2, '0')}:00Z`,
  end: `2026-09-07T12:${String(minute + 3).padStart(2, '0')}:00Z`,
  duration_ms: 180_000,
  first_tool,
  // M-20 over the same stretch: measured, nothing repeated.
  active_duration_ms: 180_000,
  repeats: { state: 'measured', repeats: 0, worst: null },
  circling: false,
});

/** The metric's default thresholds, as the answer carries them. */
const THRESHOLDS = { active_ms: 240_000, repeats: 5 };

/** One valid target, each negative case, and a second valid target elsewhere. */
const measured: MetricSessionStretches = {
  state: 'measured',
  stretches: [
    stretch(0, { record_uuid: CALLER, block_index: 1 }), // Read, in a closed row
    stretch(5, null), // no locator
    stretch(10, { record_uuid: CALLER, block_index: 0 }), // a text block
    stretch(15, { record_uuid: '44444444-4444-4444-8444-444444444444', block_index: 0 }), // absent
    stretch(20, { record_uuid: LATER, block_index: 0 }), // Grep, in another row
  ],
  repeat_thresholds: THRESHOLDS,
};

function source(options: {
  transcript?: SessionSourceStatus | 'hold' | 'fail';
  transcripts?: Record<string, SessionSourceStatus>;
  stretches?:
    | MetricSessionStretches
    | ((days: number, call: number) => MetricSessionStretches | Promise<MetricSessionStretches>);
}) {
  const opened: string[] = [];
  const stretchReads: [string, number][] = [];
  const listeners = new Map<DataEvent, Set<() => void>>();
  let settle: (status: SessionSourceStatus) => void = () => {};
  const data: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
    dashboard: async (days) => exported.dashboards.find((d) => d.window.days === days)!,
    tokensByHost: async (days) => {
      const report = exported.dashboards.find((d) => d.window.days === days)!;
      return { window: report.window, hosts: report.tokens_by_host };
    },
    environment: async (days) => exported.environments.find((e) => e.window.days === days)!,
    today: async () => exported.today,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    sessionRow: async (sessionId) => ({
      ...template,
      id: sessionId,
      host: 'claude',
      repo: '/Users/example/code/atlas',
      branch: 'feat/timeline',
    }),
    sessionStretches: async (sessionId, days) => {
      stretchReads.push([sessionId, days]);
      const given = options.stretches;
      return typeof given === 'function' ? given(days, stretchReads.length) : (given ?? measured);
    },
    sessionTranscript: (sessionId) => {
      opened.push(sessionId);
      const given = options.transcripts?.[sessionId] ?? options.transcript ?? loaded();
      if (given === 'fail') return Promise.reject(new Error('the command failed'));
      if (given === 'hold') return new Promise((resolve) => (settle = resolve));
      return Promise.resolve(given);
    },
    cancelSessionTranscript: async () => {},
    // Pull-request refresh is not exercised by this test.
    pullRequests: async () => Promise.reject(new Error('not used by this test')),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    refreshPullRequests: async () => Promise.reject(new Error('not used by this test')),
    cancelPullRequestRefresh: async () => Promise.reject(new Error('not used by this test')),
    subscribe: async (event, listener) => {
      const set = listeners.get(event) ?? new Set();
      set.add(listener);
      listeners.set(event, set);
      return () => void set.delete(listener);
    },
  };
  return {
    data,
    opened,
    stretchReads,
    settle: (status: SessionSourceStatus) => settle(status),
    /** The app's own refresh path: an ingest event invalidates its queries. */
    emit: (event: DataEvent) => listeners.get(event)?.forEach((listener) => listener()),
  };
}

function Go({ to }: { to: string }) {
  const navigate = useNavigate();
  return (
    <button type="button" onClick={() => void navigate(to)}>
      go {to}
    </button>
  );
}

function mount(data: DataSource, path = `/sessions/${SESSION}`, to?: string) {
  return render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={[path]}>
          {to && <Go to={to} />}
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

const segments = () =>
  within(screen.getByRole('list', { name: 'Hands-off stretches, in order' })).getAllByRole(
    'button',
  );
const segment = async (index: number) => {
  await screen.findByRole('list', { name: 'Hands-off stretches, in order' });
  return segments()[index];
};
/** Wait until the transcript's turns are on the page. */
const transcriptReady = () => screen.findByRole('button', { name: /Read 1 file/ });
const row = (name: RegExp) => screen.getByRole('button', { name });
const highlighted = () => [...document.querySelectorAll('.xt-tool-card[data-highlighted]')];
const live = () =>
  screen
    .getAllByRole('status')
    .find((node) => node.getAttribute('aria-live') === 'polite' && node.closest('.xt-timeline'));

it('reveals a stretch’s first call by opening its closed row, and leaves focus where it was', async () => {
  const { data } = source({});
  mount(data);
  await transcriptReady();
  const readRow = row(/Read 1 file/);
  expect(readRow.getAttribute('aria-expanded')).toBe('false');

  const first = await segment(0);
  first.focus();
  fireEvent.click(first);

  // The row that holds the call opened, and only that one.
  expect(readRow.getAttribute('aria-expanded')).toBe('true');
  expect(row(/Ran 1 command/).getAttribute('aria-expanded')).toBe('false');
  // The card now standing where the anchor stood is the one highlighted.
  const [card] = highlighted();
  expect(highlighted()).toHaveLength(1);
  expect(card.getAttribute('data-block-id')).toBe(`${CALLER}:1`);
  expect(card.getAttribute('data-tool')).toBe('Read');
  // Scrolled to, gently, and nothing else was.
  expect(scrolled).toEqual([{ element: card, options: { block: 'center', behavior: 'smooth' } }]);
  // Focus stays on the segment the reader pressed.
  expect(document.activeElement).toBe(first);
  expect(first.getAttribute('aria-pressed')).toBe('true');
  // And it is said out loud.
  expect(live()?.textContent).toBe('Showing the first tool call of stretch 1: Read.');
});

it('respects a reader who asked for less motion', async () => {
  // Only the reduced-motion query answers yes; anything else the app asks
  // (the theme, say) answers no, as it would on a real machine.
  window.matchMedia = vi.fn((query: string) => ({
    matches: query === '(prefers-reduced-motion: reduce)',
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as Window['matchMedia'];
  const { data } = source({});
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(scrolled.map((entry) => entry.options)).toEqual([{ block: 'center', behavior: 'auto' }]);
  expect(window.matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
});

it('leaves an already-open row open rather than toggling it shut', async () => {
  const { data } = source({});
  mount(data);
  await transcriptReady();
  fireEvent.click(row(/Read 1 file/));
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(await segment(0));
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('true');
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);
});

it.each([
  [1, 'Which tool call this stretch started with was not recorded.'],
  [2, 'The block recorded at that place in the transcript is not a tool call.'],
  [3, 'That tool call is not in the transcript that was read.'],
])(
  'keeps stretch %i measured and says why its first call cannot be shown',
  async (index, reason) => {
    const { data } = source({});
    mount(data);
    await transcriptReady();
    const pressed = await segment(index);
    expect(pressed.getAttribute('aria-label')).toContain(reason);
    fireEvent.click(pressed);
    // Still a segment, still pressed, still showing its duration.
    expect(pressed.getAttribute('aria-pressed')).toBe('true');
    expect(within(pressed).getByText('3 min')).toBeTruthy();
    // Nothing opened, nothing highlighted, nothing scrolled.
    expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('false');
    expect(highlighted()).toHaveLength(0);
    expect(scrolled).toHaveLength(0);
    expect(live()?.textContent).toBe(
      `The first tool call of stretch ${index + 1} cannot be shown. ${reason}`,
    );
  },
);

it('refuses a position two records share, even when one of them is a tool call', async () => {
  // A fork can carry two records under one UUID; the id on the page is then
  // shared, and neither node can be chosen.
  const forked = [
    ...records(),
    {
      id: CALLER,
      role: 'assistant' as const,
      at: null,
      blocks: [{ kind: 'text' as const, index: 1, text: 'copied' }],
    },
  ];
  const { data } = source({ transcript: loaded(forked) });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  expect(live()?.textContent).toMatch(/more than one block/i);
});

it('moves the highlight when another stretch is pressed', async () => {
  const { data } = source({});
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);
  fireEvent.click(segments()[4]);
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${LATER}:0`]);
  expect(live()?.textContent).toBe('Showing the first tool call of stretch 5: Grep.');
});

it('reveals without building a selector out of an identifier from the history', async () => {
  // The document is where a confirmed target is shown, never where one is
  // looked for: no query anywhere during a reveal names the block.
  const { data } = source({});
  mount(data);
  await transcriptReady();
  const onDocument = vi.spyOn(Document.prototype, 'querySelector');
  const onDocumentAll = vi.spyOn(Document.prototype, 'querySelectorAll');
  const onElement = vi.spyOn(Element.prototype, 'querySelector');
  const onElementAll = vi.spyOn(Element.prototype, 'querySelectorAll');
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);
  const queried = [onDocument, onDocumentAll, onElement, onElementAll]
    .flatMap((spy) => spy.mock.calls)
    .map(([selector]) => String(selector));
  expect(queried.filter((selector) => selector.includes(CALLER))).toEqual([]);
});

it('does not act later on a press made while the transcript was still being read', async () => {
  const { data, settle } = source({ transcript: 'hold' });
  mount(data);
  const first = await segment(0);
  expect(first.getAttribute('aria-label')).toContain('The transcript is still being read.');
  fireEvent.click(first);
  expect(live()?.textContent).toContain('The transcript is still being read.');
  // The transcript arrives. The earlier press does not quietly fire now: it
  // was answered, and a jump the reader did not ask for again is not theirs.
  await act(async () => settle(loaded()));
  await transcriptReady();
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('false');
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  // Pressing it again, now, does.
  fireEvent.click(segments()[0]);
  expect(highlighted()).toHaveLength(1);
});

it('never carries a reveal into another session that happens to share the position', async () => {
  // The other session's transcript has a record under the same UUID with a
  // tool call at the same index — the one place a leaked request could land.
  const { data } = source({
    transcripts: { [OTHER_SESSION]: loaded(records(CALLER)) },
  });
  mount(data, `/sessions/${SESSION}`, `/sessions/${OTHER_SESSION}`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);
  scrolled.length = 0;

  fireEvent.click(screen.getByRole('button', { name: `go /sessions/${OTHER_SESSION}` }));
  await waitFor(() => expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0));
  await transcriptReady();
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('false');
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
});

it('re-reads the stretches for a new range and never re-reads the transcript', async () => {
  const { data, opened, stretchReads } = source({});
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);

  // The page owns the range control, because what it shows is measured over it.
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(stretchReads).toContainEqual([SESSION, 30]));
  // One read of the text, however many windows its measurements are taken over.
  expect(opened).toEqual([SESSION]);
  // The press belonged to the old window's stretches, so it is cleared with it.
  await waitFor(() => expect(highlighted()).toHaveLength(0));
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
});

it('keeps the stretches and the measurements when the transcript cannot be read', async () => {
  const { data } = source({ transcript: 'fail' });
  mount(data);
  const first = await segment(0);
  await waitFor(() =>
    expect(first.getAttribute('aria-label')).toContain('The transcript is not available to show.'),
  );
  expect(segments()).toHaveLength(5);
  expect(screen.getByText(/^Records .* measured over the last 7 days$/)).toBeTruthy();
  fireEvent.click(first);
  expect(live()?.textContent).toContain('The transcript is not available to show.');
});

it('reveals the right call when identities differ only in whitespace and carry punctuation', async () => {
  const odd = ' a:b "c" ';
  const bare = 'a:b "c"';
  const list: SourceRecord[] = [
    {
      id: bare,
      role: 'assistant',
      at: null,
      blocks: [
        { kind: 'text', index: 0, text: 'bare' },
        { kind: 'tool_call', index: 1, name: 'Glob', call_id: 'toolu_bare', input: null },
      ],
    },
    {
      id: odd,
      role: 'assistant',
      at: null,
      blocks: [
        { kind: 'text', index: 0, text: 'padded' },
        { kind: 'tool_call', index: 1, name: 'Read', call_id: 'toolu_odd', input: null },
      ],
    },
  ];
  const { data } = source({
    transcript: loaded(list),
    stretches: {
      state: 'measured',
      stretches: [stretch(0, { record_uuid: odd, block_index: 1 })],
      repeat_thresholds: THRESHOLDS,
    },
  });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${odd}:1`]);
  expect(highlighted()[0].getAttribute('data-tool')).toBe('Read');
});

it('keeps focus on the segment across Enter and Space', async () => {
  // jsdom does not turn a key into a click; a native button does, which is
  // why every segment is one. What is held here is that pressing it from the
  // keyboard's own element leaves the reader on that element.
  const { data } = source({});
  mount(data);
  await transcriptReady();
  const first = await segment(0);
  first.focus();
  fireEvent.click(document.activeElement as HTMLElement);
  expect(document.activeElement).toBe(first);
  const last = segments()[4];
  last.focus();
  fireEvent.click(document.activeElement as HTMLElement);
  expect(document.activeElement).toBe(last);
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${LATER}:0`]);
});

it('does not bring a press back when the reader returns to the window it was made in', async () => {
  // A press is an index into one list. Hidden while another window is
  // selected, it would come back pointing at whatever now sits at that index;
  // it has to be gone, not hidden.
  const { data, stretchReads } = source({});
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);

  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(stretchReads).toContainEqual([SESSION, 30]));
  await waitFor(() => expect(highlighted()).toHaveLength(0));

  // Back to the window the press was made in. The answer is still cached, so
  // the very same list comes back — which is exactly the case where a merely
  // hidden press would reappear, pointing at a stretch nobody selected.
  fireEvent.click(screen.getByRole('radio', { name: '7d' }));
  await screen.findByText('Measured over the last 7 days');
  await waitFor(() => expect(segments()).toHaveLength(5));
  expect(highlighted()).toHaveLength(0);
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(live()?.textContent).toBe('');
});

it('does not keep a press across a replaced answer for the same window', async () => {
  // The same window, asked again, comes back reordered. The index the reader
  // pressed is now a different stretch, so the press cannot survive it.
  const reordered: MetricSessionStretches = {
    state: 'measured',
    stretches: [...(measured.state === 'measured' ? measured.stretches : [])].reverse(),
    repeat_thresholds: THRESHOLDS,
  };
  const { data, stretchReads, emit } = source({
    stretches: (_days, call) => (call === 1 ? measured : reordered),
  });
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);

  // The app's own refresh: an ingest event invalidates the session queries and
  // the answer comes back different for the very same window.
  act(() => emit(events.importReceived));
  await waitFor(() => expect(stretchReads.length).toBeGreaterThan(1), { timeout: 3000 });
  await waitFor(() => expect(segments()[0].getAttribute('aria-label')).toContain('Stretch 1 of 5'));
  // Nothing selected, nothing highlighted, and nothing said about a stretch
  // that is no longer the one at that index.
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(highlighted()).toHaveLength(0);
  expect(live()?.textContent).toBe('');
});

it.each([
  [1, 'Which tool call this stretch started with was not recorded.'],
  [2, 'The block recorded at that place in the transcript is not a tool call.'],
  [3, 'That tool call is not in the transcript that was read.'],
])(
  'shows stretch %i’s reason on the screen, not only to a screen reader',
  async (index, reason) => {
    const { data } = source({});
    mount(data);
    await transcriptReady();
    fireEvent.click(await segment(index));
    const note = live()!;
    expect(note.textContent).toContain(reason);
    // Seen as well as announced: jsdom applies no stylesheet, so what is checked
    // is that the note is not the class this app hides things with, and is not
    // taken out of the accessibility tree either.
    expect(note.className).toBe('xt-timeline-note');
    expect(note.className).not.toContain('sr-only');
    expect(note.getAttribute('aria-hidden')).toBeNull();
    expect(note.getAttribute('data-reason')).toBeTruthy();
  },
);

it('shows why a position naming two blocks cannot be jumped to', async () => {
  const forked = [
    ...records(),
    {
      id: CALLER,
      role: 'assistant' as const,
      at: null,
      blocks: [{ kind: 'text' as const, index: 1, text: 'copied' }],
    },
  ];
  const { data } = source({ transcript: loaded(forked) });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  const note = live()!;
  expect(note.textContent).toMatch(/more than one block/i);
  expect(note.className).toBe('xt-timeline-note');
  expect(note.getAttribute('data-reason')).toBe('ambiguous');
});

it('shows why nothing can be jumped to while the transcript is unavailable', async () => {
  const { data } = source({ transcript: 'fail' });
  mount(data);
  const first = await segment(0);
  await waitFor(() =>
    expect(first.getAttribute('aria-label')).toContain('The transcript is not available to show.'),
  );
  fireEvent.click(first);
  const note = live()!;
  expect(note.textContent).toContain('The transcript is not available to show.');
  expect(note.className).toBe('xt-timeline-note');
  expect(note.getAttribute('data-reason')).toBe('transcript_unavailable');
});

/*
 * M-20's most repeated call, from the timeline to the transcript. The position
 * is the earliest call of the stretch's largest repeated group; it goes through
 * the very same confirmation and the very same reveal as a first call.
 */

/** A measured M-20 answer for one stretch, naming its most repeated call. */
const repeating = (
  base: MetricSessionStretch,
  tool: string,
  count: number,
  representative: MetricToolBlock,
  circling: boolean,
): MetricSessionStretch => ({
  ...base,
  active_duration_ms: 300_000,
  repeats: {
    state: 'measured',
    repeats: count - 1,
    worst: { tool_name: tool, count, representative },
  },
  circling,
});

const withRepeats: MetricSessionStretches = {
  state: 'measured',
  stretches: [
    // Starts with Read; Bash, the call after it, repeated seven times.
    repeating(
      stretch(0, { record_uuid: CALLER, block_index: 1 }),
      'Bash',
      7,
      {
        record_uuid: CALLER,
        block_index: 2,
      },
      true,
    ),
    // Its most repeated call is the very call it started with.
    repeating(
      stretch(5, { record_uuid: LATER, block_index: 0 }),
      'Grep',
      3,
      {
        record_uuid: LATER,
        block_index: 0,
      },
      false,
    ),
    // Not counted: an older index never compared these calls.
    {
      ...stretch(10, { record_uuid: CALLER, block_index: 1 }),
      repeats: { state: 'unknown', reason: 'missing_key' },
      circling: null,
    },
  ],
  repeat_thresholds: THRESHOLDS,
};

const showRepeat = () => screen.getByRole('button', { name: /^Show repeated call/ });

it('shows a stretch’s repeated call through the same reveal, and keeps focus on the button', async () => {
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  // Nothing is offered until a stretch is chosen.
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);
  expect(screen.getByText(/the most repeated call was Bash, 7 calls/)).toBeTruthy();

  scrolled.length = 0;
  const button = showRepeat();
  expect(button.tagName).toBe('BUTTON');
  button.focus();
  fireEvent.click(button);
  // The Bash row opened, its call is the one highlighted, and it was scrolled to.
  expect(row(/Ran 1 command/).getAttribute('aria-expanded')).toBe('true');
  const [card] = highlighted();
  expect(highlighted()).toHaveLength(1);
  expect(card.getAttribute('data-block-id')).toBe(`${CALLER}:2`);
  expect(card.getAttribute('data-tool')).toBe('Bash');
  expect(scrolled.map((entry) => entry.element)).toEqual([card]);
  // The stretch stays the selected one, focus stays where the reader pressed,
  // and what happened is said.
  expect(segments()[0].getAttribute('aria-pressed')).toBe('true');
  expect(document.activeElement).toBe(button);
  expect(live()?.textContent).toBe(
    'Showing the earliest of the repeated calls in stretch 1: Bash.',
  );
});

it('reveals the same call again when the repeated call is the one the stretch started with', async () => {
  // The two positions coincide. Each press is its own request, so the second
  // is answered rather than taken for the first.
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(1));
  expect(live()?.textContent).toBe('Showing the first tool call of stretch 2: Grep.');
  scrolled.length = 0;
  fireEvent.click(showRepeat());
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${LATER}:0`]);
  expect(scrolled).toHaveLength(1);
  expect(live()?.textContent).toBe(
    'Showing the earliest of the repeated calls in stretch 2: Grep.',
  );
});

it('refuses a repeated call whose position two records share', async () => {
  const forked = [
    ...records(),
    {
      id: CALLER,
      role: 'assistant' as const,
      at: null,
      blocks: [{ kind: 'text' as const, index: 2, text: 'copied' }],
    },
  ];
  const { data } = source({ stretches: withRepeats, transcript: loaded(forked) });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  scrolled.length = 0;
  const button = showRepeat();
  expect(button.getAttribute('aria-label')).toMatch(/cannot be shown: More than one block/);
  fireEvent.click(button);
  // Nothing moved to a guess: the earlier highlight went with the earlier press.
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  expect(live()?.textContent).toBe(
    'The repeated call of stretch 1 cannot be shown. More than one block in the transcript sits where that call was recorded, so which one is meant is not known.',
  );
  expect(live()?.getAttribute('data-reason')).toBe('ambiguous');
});

it('keeps what M-20 measured and says why a repeated call cannot be shown without the transcript', async () => {
  const { data } = source({ stretches: withRepeats, transcript: 'fail' });
  mount(data);
  const first = await segment(0);
  await waitFor(() =>
    expect(first.getAttribute('aria-label')).toContain('The transcript is not available to show.'),
  );
  // Measured from the index, so shown whatever the text says.
  expect(within(first).getByText('0 h 5 m active · 6 repeats · Bash ×7')).toBeTruthy();
  expect(within(first).getByText('Circling')).toBeTruthy();
  fireEvent.click(first);
  fireEvent.click(showRepeat());
  expect(highlighted()).toHaveLength(0);
  expect(live()?.textContent).toBe(
    'The repeated call of stretch 1 cannot be shown. The transcript is not available to show.',
  );
});

it('offers no repeated call for a stretch whose repeats were not counted, and says why', async () => {
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  const unknown = await segment(2);
  expect(within(unknown).getByText('0 h 3 m active · repeats unknown')).toBeTruthy();
  expect(unknown.getAttribute('data-circling')).toBeNull();
  fireEvent.click(unknown);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  expect(
    screen.getByText(
      'Stretch 3: repeats not counted. Some of its tool calls were indexed before this app compared calls, or their arguments could not be compared.',
    ),
  ).toBeTruthy();
});

it('drops a repeated-call press when the range changes', async () => {
  const { data, stretchReads } = source({ stretches: withRepeats });
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  fireEvent.click(showRepeat());
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:2`]);

  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(stretchReads).toContainEqual([SESSION, 30]));
  await waitFor(() => expect(highlighted()).toHaveLength(0));
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  // And back: the cached answer returns, the press does not.
  fireEvent.click(screen.getByRole('radio', { name: '7d' }));
  await screen.findByText('Measured over the last 7 days');
  await waitFor(() => expect(segments()).toHaveLength(3));
  expect(highlighted()).toHaveLength(0);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  expect(live()?.textContent).toBe('');
});

it('drops a repeated-call press when the answer for the same window is replaced', async () => {
  // Refreshed, the same window says the stretches in another order. The press
  // named a place in the old list, and its call is not re-found in the new one.
  const reordered: MetricSessionStretches = {
    ...withRepeats,
    stretches: [...(withRepeats.state === 'measured' ? withRepeats.stretches : [])].reverse(),
  };
  const { data, stretchReads, emit } = source({
    stretches: (_days, call) => (call === 1 ? withRepeats : reordered),
  });
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  fireEvent.click(showRepeat());
  expect(highlighted()).toHaveLength(1);

  act(() => emit(events.importReceived));
  await waitFor(() => expect(stretchReads.length).toBeGreaterThan(1), { timeout: 3000 });
  await waitFor(() => expect(segments()[0].getAttribute('data-repeats')).toBe('unknown'));
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(highlighted()).toHaveLength(0);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  expect(live()?.textContent).toBe('');
});
