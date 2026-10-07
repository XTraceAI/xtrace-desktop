import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { SessionRow } from '../../data/generated/SessionRow';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { MetricSessionStretches } from '../../data/generated/MetricSessionStretches';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
const template = exported.sessions[0].rows[0];
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

/** Synthetic throughout: an identity, a repository and a turn, all invented. */
const SESSION = '11111111-0000-4000-8000-000000000001';
const row: SessionRow = {
  ...template,
  id: SESSION,
  host: 'claude',
  repo: '/Users/example/code/atlas',
  branch: 'feat/detail',
  model: 'model-a',
  record_count: 42,
  metrics: {
    state: 'indexed',
    events: 12,
    human_messages: 3,
    tool_calls: 7,
    tokens: { ...(template.metrics.state === 'indexed' ? template.metrics.tokens : never()) },
    agent_ms: registeredMinutes(4),
  },
};

function never(): never {
  throw new Error('the fixture row must be indexed');
}
function registeredMinutes(minutes: number) {
  return minutes * 60_000;
}

const readable: SessionSourceStatus = {
  state: 'loaded',
  generation: { generation: 'indexed', appended: false },
  sources: 1,
  dropped_records: 0,
  gaps: [],
  records: [
    {
      id: 'aaaaaaaa-0000-4000-8000-00000000000a',
      role: 'user',
      at: null,
      blocks: [{ kind: 'text', index: 0, text: 'a synthetic question' }],
    },
  ],
};

interface Harness {
  source: DataSource;
  /** Every stretches lookup, as `[sessionId, windowDays]`. */
  stretchReads: [string, number][];
  /** The window each row lookup asked for, in order. */
  windows: number[];
  opened: { sessionId: string; readId: string }[];
  cancelled: string[];
  settle: (status: SessionSourceStatus) => void;
  fail: () => void;
}

/**
 * A source whose transcript read is held open until the test settles it, so
 * the order of an open, a replacement and a cancel is the test's to choose
 * rather than the scheduler's.
 */
function harness(
  options: {
    rows?: SessionRow[];
    auto?: SessionSourceStatus | null;
    stretches?: MetricSessionStretches | ((days: number) => Promise<MetricSessionStretches>);
  } = {},
): Harness {
  const stretchReads: [string, number][] = [];
  const opened: { sessionId: string; readId: string }[] = [];
  const cancelled: string[] = [];
  const windows: number[] = [];
  let settle: (status: SessionSourceStatus) => void = () => {};
  let fail: () => void = () => {};
  const source: DataSource = {
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
    dashboard: async () => exported.dashboards[0],
    tokensByHost: async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    }),
    environment: async () => exported.environments[0],
    today: async () => exported.today,
    sessionsList: async () => ({
      window: exported.sessions[0].window,
      next: null,
      rows: options.rows ?? [row],
    }),
    // Exactly as the native command answers: the identity is compared, never
    // searched for, so a prefix of another session is not this one.
    sessionRow: async (sessionId, windowDays) => {
      windows.push(windowDays);
      return (options.rows ?? [row]).find((candidate) => candidate.id === sessionId) ?? null;
    },
    sessionStretches: async (sessionId, days) => {
      stretchReads.push([sessionId, days]);
      const given: NonNullable<typeof options.stretches> = options.stretches ?? {
        state: 'measured',
        stretches: [],
        repeat_thresholds: { active_ms: 240_000, repeats: 5 },
      };
      return typeof given === 'function' ? given(days) : given;
    },
    sessionTranscript: (sessionId, readId) => {
      opened.push({ sessionId, readId });
      if (options.auto !== undefined && options.auto !== null) {
        return Promise.resolve(options.auto);
      }
      return new Promise<SessionSourceStatus>((resolve, reject) => {
        settle = resolve;
        fail = () => reject(new Error('the command failed'));
      });
    },
    cancelSessionTranscript: async (readId) => void cancelled.push(readId),
    // Pull-request refresh is not exercised by this test.
    pullRequests: async () => Promise.reject(new Error('not used by this test')),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    refreshPullRequests: async () => Promise.reject(new Error('not used by this test')),
    cancelPullRequestRefresh: async () => Promise.reject(new Error('not used by this test')),
    subscribe: async () => () => {},
  };
  return {
    source,
    stretchReads,
    opened,
    cancelled,
    windows,
    settle: (s) => settle(s),
    fail: () => fail(),
  };
}

/** Drives the router's own history, the way a link in the app does. */
function Go({ to }: { to: string }) {
  const navigate = useNavigate();
  return (
    <button type="button" onClick={() => void navigate(to)}>
      go
    </button>
  );
}

function mount(source: DataSource, path = `/sessions/${SESSION}`, to?: string) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          {to && <Go to={to} />}
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

it('shows what was measured even when the text cannot be shown at all', async () => {
  // The whole point of the two halves being independent: a file that has been
  // deleted, moved or rewritten takes nothing away from what was measured.
  const { source } = harness({
    auto: { state: 'unavailable', reason: { reason: 'replaced' } },
  });
  mount(source);
  await screen.findByRole('heading', { name: 'atlas · feat/detail' });
  expect(await screen.findByText('This transcript is not available to show.')).toBeTruthy();
  expect(screen.getByText(/has changed since it was measured/)).toBeTruthy();
  // Every measured column is still here, under its own rule.
  expect(screen.getByRole('button', { name: 'Human messages definition' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Tokens definition' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Agent time definition' })).toBeTruthy();
  expect(screen.getByText('3')).toBeTruthy();
  expect(screen.getByText('Records 42 · measured over the last 7 days')).toBeTruthy();
  // And it is never worded as a session that recorded nothing.
  expect(screen.queryByText('This view was given no turns to show.')).toBeNull();
});

it('says a Cursor session kept in a database is unavailable, and keeps what was measured', async () => {
  const { source } = harness({
    rows: [{ ...row, id: SESSION, host: 'cursor' }],
    auto: { state: 'unavailable', reason: { reason: 'store_unsupported' } },
  });
  mount(source);
  expect(await screen.findByText(/kept in a database XTrace does not read/)).toBeTruthy();
  expect(screen.getByText('Records 42 · measured over the last 7 days')).toBeTruthy();
  expect(screen.queryByText('This view was given no turns to show.')).toBeNull();
});

it('says which reader ceiling a Codex session passed, and keeps what was measured', async () => {
  const { source } = harness({
    rows: [{ ...row, id: SESSION, host: 'codex' }],
    auto: { state: 'unavailable', reason: { reason: 'reader_limit', limit: 'records' } },
  });
  mount(source);
  expect(
    await screen.findByText(/larger than XTrace reads at once \(it holds too many records\)/),
  ).toBeTruthy();
  expect(screen.getByText('Records 42 · measured over the last 7 days')).toBeTruthy();
});

it('shows a Codex session the reader returned, like any other transcript', async () => {
  const { source } = harness({
    rows: [{ ...row, id: SESSION, host: 'codex' }],
    auto: readable,
  });
  mount(source);
  expect(await screen.findByText('a synthetic question')).toBeTruthy();
  expect(screen.getByText('Records 42 · measured over the last 7 days')).toBeTruthy();
});

it('says what a loaded transcript is missing rather than reading as complete', async () => {
  const { source } = harness({
    auto: { ...readable, gaps: [{ gap: 'missing' }, { gap: 'discovery_incomplete' }] },
  });
  mount(source);
  expect(await screen.findByText('a synthetic question')).toBeTruthy();
  expect(
    screen.getByText(
      /This is part of this session’s record: a sub-agent transcript it counted is no longer on this Mac; part of its sub-agent history could not be listed\./,
    ),
  ).toBeTruthy();
});

it('says when the file has moved on since it was measured', async () => {
  const { source } = harness({
    auto: { ...readable, generation: { generation: 'indexed', appended: true } },
  });
  mount(source);
  expect(await screen.findByText(/has been written to since it was last indexed/)).toBeTruthy();
  // The text is complete; it is the relationship to the numbers that is stated.
  expect(screen.queryByText(/This is part of this session’s record/)).toBeNull();
});

it('says when the text is not tied to a measurement at all', async () => {
  const { source } = harness({ auto: { ...readable, generation: { generation: 'unrecorded' } } });
  mount(source);
  expect(await screen.findByText(/not tied to a measurement/)).toBeTruthy();
});

it('reads one session per open, and cancels the read it started when it leaves', async () => {
  const { source, opened, cancelled, settle } = harness();
  const view = mount(source);
  await waitFor(() => expect(opened).toHaveLength(1));
  expect(opened[0].sessionId).toBe(SESSION);
  settle(readable);
  expect(await screen.findByText('a synthetic question')).toBeTruthy();
  expect(cancelled).toEqual([]);
  view.unmount();
  await waitFor(() => expect(cancelled).toEqual([opened[0].readId]));
});

it('cancels the read it replaced, and shows only the open that is current', async () => {
  // The reader opened one session and then another before the first answered.
  const second = '11111111-0000-4000-8000-000000000002';
  const { source, opened, cancelled } = harness({
    rows: [row, { ...row, id: second, repo: '/Users/example/code/beacon', branch: 'main' }],
  });
  mount(source, `/sessions/${SESSION}`, `/sessions/${second}`);
  await waitFor(() => expect(opened).toHaveLength(1));
  const first = opened[0];
  fireEvent.click(screen.getByRole('button', { name: 'go' }));
  await waitFor(() => expect(opened).toHaveLength(2));
  // The read that was replaced is cancelled by its own name, and the new one
  // is not: a cancel names one open, never every read.
  await waitFor(() => expect(cancelled).toEqual([first.readId]));
  expect(opened[1].sessionId).toBe(second);
  expect(opened[1].readId).not.toBe(first.readId);
});

it('discards an answer that arrives after the open it belonged to ended', async () => {
  // The backend read is bounded and may finish after the cancel crosses. What
  // it says then is about a session the reader has left.
  const { source, opened, settle } = harness();
  const view = mount(source);
  await waitFor(() => expect(opened).toHaveLength(1));
  view.unmount();
  settle(readable);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByText('a synthetic question')).toBeNull();
});

it('offers a way to ask again when the read itself fails', async () => {
  const { source, opened, fail, settle } = harness();
  mount(source);
  await waitFor(() => expect(opened).toHaveLength(1));
  fail();
  expect(await screen.findByText(/could not be read just now/)).toBeTruthy();
  // A failed command is not a session with no turns either.
  expect(screen.queryByText('This view was given no turns to show.')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await waitFor(() => expect(opened).toHaveLength(2));
  // A new open, with a name of its own: a retry must not inherit a cancel.
  expect(opened[1].readId).not.toBe(opened[0].readId);
  settle(readable);
  expect(await screen.findByText('a synthetic question')).toBeTruthy();
});

it('says plainly when the index holds no session with this identifier', async () => {
  const { source } = harness({ rows: [], auto: readable });
  mount(source);
  expect(await screen.findByText(/holds no session with this identifier/)).toBeTruthy();
  // No measurements are invented for it, and no zero stands in for one.
  expect(screen.queryByRole('button', { name: /Human messages/ })).toBeNull();
});

it('does not mistake a session whose identifier merely starts the same way', async () => {
  // The list search is a substring match; this page is about one session.
  const { source } = harness({ rows: [{ ...row, id: `${SESSION}-fork` }], auto: readable });
  mount(source);
  expect(await screen.findByText(/holds no session with this identifier/)).toBeTruthy();
});

it('opens a session whose identifier carries path punctuation', async () => {
  const awkward = 'codex-branch a&b/c';
  const { source, opened } = harness({ rows: [{ ...row, id: awkward }], auto: readable });
  mount(source, `/sessions/${encodeURIComponent(awkward)}`);
  await waitFor(() => expect(opened[0]?.sessionId).toBe(awkward));
  expect(await screen.findByText(awkward)).toBeTruthy();
});

it('carries the list’s own filters into its Back link and nothing else', async () => {
  const { source } = harness({ auto: readable });
  mount(source, `/sessions/${SESSION}?q=atlas&host=claude&range=30d&pr=9`);
  const back = await screen.findByRole('link', { name: '← All sessions' });
  expect(back.getAttribute('href')).toBe('/sessions?q=atlas&host=claude&range=30d');
  // The range in the address is the window this page measures over, too.
  // The row and the stretches both say which window they were measured over.
  expect(await screen.findByText(/^Records .* measured over the last 30 days$/)).toBeTruthy();
  expect(screen.getByText('Measured over the last 30 days')).toBeTruthy();
});

it('shows no metric ID as chrome and keeps the M-09 definition one tab stop away', async () => {
  const { source } = harness({ auto: readable });
  mount(source);
  await screen.findByText('Measured over the last 7 days');
  const card = screen.getByText('Hands-off stretches').closest('section') as HTMLElement;
  // Visible text only: accessible names and the closed definition are not chrome.
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  const visible: string[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (!node.parentElement?.closest('.sr-only')) visible.push(node.textContent ?? '');
  }
  expect(visible.join(' ')).not.toMatch(/\b[MRC]-\d{2}\b/);
  const info = within(card).getByRole('button', { name: 'Hands-off stretches definition' });
  expect(info.textContent).toBe('');
  fireEvent.focus(info);
  const tip = await screen.findByRole('tooltip');
  expect(tip.textContent).toMatch(/^How long an agent works on its own/);
  expect(tip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
  expect(info.getAttribute('aria-describedby')).toBe(tip.id);
  fireEvent.keyDown(info, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
});

it('captions a user-role prompt by its saved role, not as a person, and counts it as nothing more', async () => {
  // An agent-written prompt is saved under the same `user` role a person's is, and the index
  // has already left it out of the Human count. The caption names the role and claims no sender.
  const prompt = 'HUMAN: please check M-02 before you merge.';
  const { source } = harness({
    rows: [{ ...row, metrics: { ...row.metrics, human_messages: 0 } as SessionRow['metrics'] }],
    auto: {
      ...readable,
      records: [
        {
          id: 'r-user',
          role: 'user',
          at: null,
          blocks: [{ kind: 'text', index: 0, text: prompt }],
        },
        {
          id: 'r-agent',
          role: 'assistant',
          at: null,
          blocks: [
            { kind: 'text', index: 0, text: 'Checking.' },
            { kind: 'tool_call', index: 1, name: 'Read', call_id: 'toolu_1', input: null },
          ],
        },
        {
          id: 'r-result',
          role: 'user',
          at: null,
          blocks: [{ kind: 'tool_result', index: 0, call_id: 'toolu_1', failed: false, parts: [] }],
        },
      ],
    },
  });
  const { container } = mount(source);
  // Recorded text is shown exactly as written, IDs and all.
  expect(await screen.findByText(prompt)).toBeTruthy();
  const caption = (id: string) =>
    container.querySelector(`[data-record-id="${id}"] .xt-turn-head > span`)?.textContent;
  expect(caption('r-user')).toBe('User');
  expect(container.querySelector('[data-record-id="r-user"]')?.getAttribute('data-role')).toBe(
    'user',
  );
  expect(caption('r-agent')).toBe('Agent');
  expect(caption('r-result')).toBe('Tool');
  // The count is the index's, unchanged by what the caption says.
  const human = screen.getByRole('button', { name: 'Human messages definition' });
  expect(human.closest('.xt-session-measure')?.querySelector('dd')?.textContent).toBe('0');
});

it('names each measure and says why one is missing without a rule ID', async () => {
  const { source } = harness({
    rows: [
      {
        ...row,
        metrics: {
          ...row.metrics,
          human_messages: null,
          tokens: {
            ...(row.metrics.state === 'indexed' ? row.metrics.tokens : never()),
            counters: {
              ...(row.metrics.state === 'indexed' ? row.metrics.tokens.counters : never()),
              total_tokens: null,
            },
          },
        } as SessionRow['metrics'],
      },
    ],
    auto: readable,
  });
  mount(source);
  const card = (await screen.findByText('Measured in this range')).closest('section')!;
  expect(
    within(card).getByText('Unmeasured: Could not tell which messages a person sent.'),
  ).toBeTruthy();
  expect(within(card).getByText('Unmeasured: Some token counts are missing.')).toBeTruthy();
  const said = [...card.querySelectorAll('[aria-label], [title], .sr-only')].map(
    (node) =>
      `${node.getAttribute('aria-label') ?? ''} ${node.getAttribute('title') ?? ''} ${
        node.classList.contains('sr-only') ? node.textContent : ''
      }`,
  );
  expect(said.join(' ')).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);

  // Each definition still opens on focus with the rule's plain summary, and closes on Escape.
  for (const [name, summary] of [
    ['Human messages definition', /^Messages a person sent\./],
    ['Tokens definition', /^Tokens used by each model response/],
    ['Agent time definition', /^Time your agent sessions were active/],
  ] as const) {
    const trigger = within(card).getByRole('button', { name });
    fireEvent.focus(trigger);
    const tip = await screen.findByRole('tooltip');
    expect(tip.textContent).toMatch(summary);
    expect(tip.textContent).not.toMatch(/\b[A-Z]-\d{2}[a-z]?\b/);
    expect(trigger.getAttribute('aria-describedby')).toBe(tip.id);
    fireEvent.keyDown(trigger, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  }
});

it('offers the plain list when there were no filters to carry', async () => {
  const { source } = harness({ auto: readable });
  mount(source);
  const back = await screen.findByRole('link', { name: '← All sessions' });
  expect(back.getAttribute('href')).toBe('/sessions');
});

it('never shows one session’s text under another session’s identity', async () => {
  // Effects run after paint. On the first committed frame with a new session
  // in the address, the read state still holds the previous session's records
  // — the effect that resets it has not run yet. Rendering them there would
  // put one person's transcript under another session's name, for a frame
  // that is painted and clickable.
  const second = '11111111-0000-4000-8000-000000000002';
  const other = 'bbbbbbbb-0000-4000-8000-00000000000b';
  const { source, opened, settle } = harness({
    rows: [row, { ...row, id: second, repo: '/Users/example/code/beacon', branch: 'main' }],
  });
  mount(source, `/sessions/${SESSION}`, `/sessions/${second}`);
  await waitFor(() => expect(opened).toHaveLength(1));
  settle(readable);
  expect(await screen.findByText('a synthetic question')).toBeTruthy();

  fireEvent.click(screen.getByRole('button', { name: 'go' }));
  // The first session's turn is gone the moment the address changes, not once
  // the new read answers.
  expect(screen.queryByText('a synthetic question')).toBeNull();
  await waitFor(() => expect(opened).toHaveLength(2));
  settle({
    ...readable,
    records: [
      {
        id: other,
        role: 'user',
        at: null,
        blocks: [{ kind: 'text', index: 0, text: 'a different question' }],
      },
    ],
  });
  expect(await screen.findByText('a different question')).toBeTruthy();
  expect(screen.queryByText('a synthetic question')).toBeNull();
});

it('names the session it is about, rather than searching for it', async () => {
  // The list's search matches a substring of the identity and answers with a
  // bounded page of whatever it matched. A session whose identity is contained
  // in enough others could not be reached that way; this page asks for it.
  const { source, windows } = harness({ auto: readable });
  const list = vi.spyOn(source, 'sessionsList');
  const exact = vi.spyOn(source, 'sessionRow');
  mount(source, `/sessions/${SESSION}?range=30d`);
  await screen.findByRole('heading', { name: 'atlas · feat/detail' });
  expect(exact).toHaveBeenCalledWith(SESSION, 30);
  expect(list).not.toHaveBeenCalled();
  // The window in the address is the window the row is measured over.
  expect(windows).toEqual([30]);
});

it('says the appended source has grown without calling the text complete', async () => {
  // A session can have grown since it was indexed *and* be missing part of
  // itself. A sentence here calling the text complete would contradict the
  // notice underneath it.
  const { source } = harness({
    auto: {
      ...readable,
      generation: { generation: 'indexed', appended: true },
      gaps: [{ gap: 'missing' }],
      dropped_records: 2,
    },
  });
  mount(source);
  const grown = await screen.findByText(/has been written to since it was last indexed/);
  expect(grown.textContent).not.toMatch(/complete/i);
  expect(screen.getByText(/2 of its records could not be identified/)).toBeTruthy();
  expect(screen.getByText(/no longer on this Mac/)).toBeTruthy();
});

it('shows a call whose identifier is used twice as neither answered nor unanswered', async () => {
  const { source } = harness({
    auto: {
      ...readable,
      records: [
        {
          id: 'aaaaaaaa-0000-4000-8000-00000000000a',
          role: 'assistant',
          at: null,
          blocks: [{ kind: 'tool_call', index: 0, name: 'Read', call_id: 'toolu_1', input: null }],
        },
        {
          id: 'bbbbbbbb-0000-4000-8000-00000000000b',
          role: 'user',
          at: null,
          blocks: [
            {
              kind: 'tool_result',
              index: 0,
              call_id: 'toolu_1',
              failed: false,
              parts: [{ kind: 'text', text: 'the first answer' }],
            },
            {
              kind: 'tool_result',
              index: 1,
              call_id: 'toolu_1',
              failed: true,
              parts: [{ kind: 'text', text: 'the second answer' }],
            },
          ],
        },
      ],
    },
  });
  mount(source);
  // Open every run, so the cards are mounted.
  await screen.findAllByRole('button', { name: /Read/ });
  for (const row of document.querySelectorAll('.xt-tool-row')) {
    fireEvent.click(row);
  }
  const call = document.querySelector(
    '.xt-tool-card[data-block-id="aaaaaaaa-0000-4000-8000-00000000000a:0"]',
  )!;
  expect(call.getAttribute('data-tool-state')).toBe('ambiguous');
  expect(call.textContent).toContain('identifier is used more than once');
  // Not a failure, not a success, and not "no result for this call".
  expect(call.textContent).not.toContain('No result for this call is in the record.');
  // The turn that made the call counts no failure: an unanswerable call is
  // not a failed one. The turn that carried the results counts the one that
  // says it failed, because that is the result's own statement.
  const asked = document.querySelector('[data-record-id="aaaaaaaa-0000-4000-8000-00000000000a"]')!;
  const answered = document.querySelector(
    '[data-record-id="bbbbbbbb-0000-4000-8000-00000000000b"]',
  )!;
  expect(asked.textContent).not.toContain('failed');
  expect(answered.textContent).toContain('1 failed');
  // Both answers are still on the page, whole.
  expect(screen.getByText(/the first answer/)).toBeTruthy();
  expect(screen.getByText(/the second answer/)).toBeTruthy();
});

describe('linked pull requests in the heading', () => {
  const link = (
    number: number,
    confidence: 'exact' | 'sha' | 'inferred',
    title: string | null = null,
  ) => ({
    repository: 'example/atlas',
    number,
    url: `https://github.com/example/atlas/pull/${number}`,
    confidence,
    title,
  });
  const badges = () => screen.queryAllByRole('img', { name: /linked pull request/ });

  it('draws no badge and no label when none was recorded', async () => {
    const { source } = harness({ rows: [{ ...row, pr_links: [] }], auto: readable });
    mount(source);
    await screen.findByRole('heading', { name: 'atlas · feat/detail' });
    expect(badges()).toHaveLength(0);
    expect(screen.queryByText(/^Linked PRs?$/)).toBeNull();
  });

  it('names one link by repository, number, evidence and stored title', async () => {
    const { source } = harness({
      rows: [{ ...row, pr_links: [link(9, 'exact', 'A synthetic title')] }],
      auto: readable,
    });
    mount(source);
    const list = await screen.findByRole('list', { name: 'Linked PR' });
    const badge = within(list).getByRole('img', {
      name: 'example/atlas#9, linked pull request, exact evidence: A synthetic title',
    });
    expect(badge.textContent).toBe('example/atlas#9');
    expect(badge.getAttribute('title')).toBe(
      'example/atlas#9 · exact evidence · A synthetic title',
    );
    // Nothing to follow: not a link, not a button, and no address in it.
    expect(badge.closest('a, button')).toBeNull();
    expect(within(list).queryByRole('link')).toBeNull();
    expect(badge.getAttribute('title')).not.toContain('https://');
  });

  it('shows every recorded link, each on its own evidence', async () => {
    const { source } = harness({
      rows: [{ ...row, pr_links: [link(9, 'exact'), link(3, 'sha'), link(7, 'inferred')] }],
      auto: readable,
    });
    mount(source);
    const list = await screen.findByRole('list', { name: 'Linked PRs' });
    expect(within(list).getAllByRole('listitem')).toHaveLength(3);
    expect(badges().map((badge) => badge.getAttribute('aria-label'))).toEqual([
      'example/atlas#9, linked pull request, exact evidence',
      'example/atlas#3, linked pull request, commit evidence',
      'example/atlas#7, linked pull request, inferred evidence',
    ]);
    // An inferred link is never drawn like an exact one.
    expect(badges().map((badge) => badge.getAttribute('data-confidence'))).toEqual([
      'exact',
      'sha',
      'inferred',
    ]);
    expect(screen.queryByText(/\+\d/)).toBeNull();
    // No wording claims more than a link.
    const words = list.parentElement!.textContent!;
    expect(words).not.toMatch(/contribut|merged|author/i);
    // Back is still the page's way out.
    expect(screen.getByRole('link', { name: '← All sessions' }).getAttribute('href')).toBe(
      '/sessions',
    );
  });

  it('invents no badge while the row is still being read', async () => {
    const { source } = harness({ auto: readable });
    source.sessionRow = () => new Promise(() => {});
    mount(source);
    await screen.findByText('Reading this session’s measurements…');
    expect(badges()).toHaveLength(0);
    expect(screen.queryByRole('list', { name: /Linked PR/ })).toBeNull();
  });

  it('invents no badge when the row cannot be read, and draws it once it is', async () => {
    const { source } = harness({
      rows: [{ ...row, pr_links: [link(9, 'exact')] }],
      auto: readable,
    });
    const answer = source.sessionRow;
    let failing = true;
    source.sessionRow = (id, days) =>
      failing ? Promise.reject(new Error('the command failed')) : answer(id, days);
    mount(source);
    await screen.findByText(/measurements could not be loaded/);
    expect(badges()).toHaveLength(0);
    failing = false;
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(
      await screen.findByRole('img', {
        name: 'example/atlas#9, linked pull request, exact evidence',
      }),
    ).toBeTruthy();
  });

  it('withholds the links while the row is read again, then shows the new answer', async () => {
    const { source } = harness({
      rows: [{ ...row, pr_links: [link(9, 'exact', 'An old title'), link(3, 'sha')] }],
      auto: readable,
    });
    // The runtime's own refresh path: a committed pull-request refresh is heard
    // and the exact row is read again, and that read is held open here.
    const listeners = new Map<string, (payload?: unknown) => void>();
    source.subscribe = async (event, listener) => {
      listeners.set(event, listener);
      return () => {};
    };
    mount(source);
    await screen.findByRole('img', {
      name: 'example/atlas#9, linked pull request, exact evidence: An old title',
    });
    expect(badges()).toHaveLength(2);

    let answer: (row: SessionRow | null) => void = () => {};
    source.sessionRow = () => new Promise((resolve) => (answer = resolve));
    listeners.get('prs://refreshed')!();
    await waitFor(() => expect(badges()).toHaveLength(0), { timeout: 2000 });
    expect(screen.queryByRole('list', { name: /Linked PR/ })).toBeNull();
    // Only the links wait: the rest of the page is still the session's.
    expect(screen.getByRole('heading', { name: 'atlas · feat/detail' })).toBeTruthy();
    expect(screen.getByText('Records 42 · measured over the last 7 days')).toBeTruthy();
    expect(screen.getByRole('link', { name: '← All sessions' }).getAttribute('href')).toBe(
      '/sessions',
    );

    // One link removed and the other retitled: the new answer, nothing older.
    answer({ ...row, pr_links: [link(9, 'exact', 'A new title')] });
    expect(
      await screen.findByRole('img', {
        name: 'example/atlas#9, linked pull request, exact evidence: A new title',
      }),
    ).toBeTruthy();
    expect(badges()).toHaveLength(1);
    expect(screen.getByRole('list', { name: 'Linked PR' })).toBeTruthy();
    expect(screen.queryByText(/An old title/)).toBeNull();
    expect(screen.queryByRole('img', { name: /#3,/ })).toBeNull();
  });

  it('invents no badge for a session the index does not hold', async () => {
    const { source } = harness({ rows: [], auto: readable });
    mount(source);
    await screen.findByText(/holds no session with this identifier/);
    expect(badges()).toHaveLength(0);
  });
});

it('names the same most-used model, with the others counted, as the session’s list row', async () => {
  const { source } = harness({ auto: readable, rows: [{ ...row, other_models: 1 }] });
  mount(source);
  expect(await screen.findByText('model-a +1 more')).toBeTruthy();
});
