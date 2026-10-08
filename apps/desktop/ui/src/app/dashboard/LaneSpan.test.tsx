import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { SpanDetailControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import { events } from '../../data/ipc-names';
import type { DashboardAutomaticText } from '../../data/generated/DashboardAutomaticText';
import type { DashboardSpanAutomatic } from '../../data/generated/DashboardSpanAutomatic';
import type { DashboardSpanDetail } from '../../data/generated/DashboardSpanDetail';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { ActivityLanes } from './ActivityLanes';
import { spanDuration } from './LaneSpan';

class Observer {
  observe() {}
  unobserve() {}
  disconnect() {}
}
beforeEach(() => vi.stubGlobal('IntersectionObserver', Observer));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const exported = fixture as FixtureExport;
const lane = exported.dashboards[0].lanes[0];

/**
 * F1's Dashboard, whose one lane span is 12:00–12:23 UTC, read through the
 * fixture source. `answer` replaces the exported detail when given.
 */
function setup(answer?: () => Promise<DashboardSpanDetail>) {
  const source = new FixtureDataSource(structuredClone(exported));
  const read = vi.fn<SpanDetailControls['read']>(answer ?? source.spanDetails.read);
  const cancel = vi.fn(async () => {});
  Object.assign(source, { spanDetails: { read, cancel } });
  const view = (report: FixtureExport['dashboards'][number]) => (
    <MemoryRouter>
      <DataProvider source={source}>
        <ActivityLanes report={report} range="7d" />
      </DataProvider>
    </MemoryRouter>
  );
  const { rerender } = render(view(structuredClone(exported.dashboards[0])));
  const bar = () => screen.getByRole('img', { name: /^Active span / });
  return {
    source,
    read,
    cancel,
    bar,
    show: (report: FixtureExport['dashboards'][number]) => rerender(view(report)),
  };
}

const indexed = (
  overrides: Partial<Extract<DashboardSpanDetail, { state: 'indexed' }>>,
): DashboardSpanDetail => ({
  state: 'indexed',
  tool: { state: 'called', name: 'WebFetch', calls: 3 },
  output_tokens: 3_150_000,
  cost: {
    total_usd: 1.25,
    priced_subtotal_usd: 1.25,
    selected_observations: 2,
    priced_observations: 2,
    unpriced_observations: 0,
    assumed_tier_observations: 0,
    unpriced: [],
  },
  prompt: {
    state: 'found',
    at_ms: lane.start_ms,
    in_span: true,
    text: { state: 'stored', text: 'make the PR link exact, not inferred', truncated: false },
  },
  automatic: { state: 'no_notification' },
  ...overrides,
});

const notification = (text: DashboardAutomaticText): DashboardSpanAutomatic => ({
  state: 'found',
  at_ms: lane.start_ms + 60_000,
  text,
});

describe('spanDuration', () => {
  it('states a length as all agent time is written, and a single event as one', () => {
    expect(spanDuration(0, 0)).toBe('single event');
    expect(spanDuration(0, 2_000)).toBe('<1 m');
    expect(spanDuration(0, 30_000)).toBe('0 h 1 m');
    expect(spanDuration(0, 23 * 60_000)).toBe('0 h 23 m');
    expect(spanDuration(0, 150 * 60_000)).toBe('2 h 30 m');
  });
});

it('opens a bubble on hover with the span’s name, length, start, tool, output and prompt', async () => {
  const { read, bar } = setup();
  const span = await waitFor(bar);
  // The bar is a named mark with no native title doubling the bubble.
  expect(span.getAttribute('title')).toBeNull();
  expect(span.getAttribute('aria-label')).toBe(
    'Active span Sep 7, 12:00 PM – Sep 7, 12:23 PM, 23 minutes',
  );
  fireEvent.mouseEnter(span);
  fireEvent.mouseMove(span);
  const bubble = await screen.findByRole('tooltip');
  expect(bubble.querySelector('.xt-span-bubble-name')!.textContent).toBe('Session 00000000');
  expect(bubble.querySelector('.xt-span-bubble-duration')!.textContent).toBe('0 h 23 m');
  expect(bubble.querySelector('time')!.textContent).toBe('12:00 PM');
  // The fixture's own answer, read through the native command at export.
  await waitFor(() =>
    expect(bubble.querySelector('.xt-span-bubble-tool')?.textContent).toBe('Read'),
  );
  expect(bubble.querySelector('.xt-span-bubble-output')!.textContent).toBe('150 output tokens');
  expect(bubble.querySelector('.xt-span-bubble-prompt')!.textContent).toBe(
    '› Implement synthetic task 5.',
  );
  expect(read).toHaveBeenCalledExactlyOnceWith(
    lane.session_id,
    lane.start_ms,
    lane.end_ms,
    expect.stringMatching(/^span-/),
  );
});

it('says it is reading until the detail answers, then keeps the answer for the next open', async () => {
  let resolve!: (detail: DashboardSpanDetail) => void;
  const { read, bar } = setup(() => new Promise((done) => (resolve = done)));
  const span = await waitFor(bar);
  fireEvent.focus(span);
  const bubble = await screen.findByRole('tooltip');
  expect(bubble.textContent).toContain('reading…');
  // Nothing is shown for a value not read yet: no tool, no zero output.
  expect(bubble.querySelector('.xt-span-bubble-tool')).toBeNull();
  expect(bubble.querySelector('.xt-span-bubble-output')).toBeNull();
  expect(read).toHaveBeenCalledTimes(1);
  await act(async () => resolve(indexed({})));
  await waitFor(() => expect(bubble.textContent).toContain('WebFetch'));
  expect(bubble.querySelector('.xt-span-bubble-output')!.textContent).toBe('3.15M output tokens');
  expect(bubble.querySelector('.xt-span-bubble-prompt')!.textContent).toBe(
    '› make the PR link exact, not inferred',
  );
  // Committed-data invalidation (here, the first index status the app reads)
  // may re-read it once; closing and opening again reads nothing.
  await act(async () => resolve(indexed({})));
  const settled = read.mock.calls.length;
  fireEvent.blur(span);
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  fireEvent.focus(span);
  const again = await screen.findByRole('tooltip');
  expect(again.textContent).toContain('WebFetch');
  expect(read).toHaveBeenCalledTimes(settled);
});

it.each<[string, DashboardSpanDetail, Record<string, string | null>]>([
  [
    'unknown tool, unmeasured output and words missing from the source',
    indexed({
      tool: { state: 'unknown' },
      output_tokens: null,
      prompt: {
        state: 'found',
        at_ms: lane.start_ms - 3_600_000,
        in_span: false,
        text: { state: 'not_found' },
      },
    }),
    {
      tool: 'tools unknown',
      output: 'output unmeasured',
      prompt: '› before this span · last message’s words not found in the session file',
    },
  ],
  [
    'words whose source changed since it was indexed',
    indexed({
      prompt: {
        state: 'found',
        at_ms: lane.start_ms,
        in_span: true,
        text: { state: 'unavailable', reason: { reason: 'replaced' } },
      },
    }),
    {
      tool: 'WebFetch',
      output: '3.15M output tokens',
      prompt: '› last message’s words unavailable: the session file changed since it was indexed',
    },
  ],
  [
    'a wrapped input, whose person’s part nothing locates',
    indexed({
      prompt: { state: 'found', at_ms: lane.start_ms, in_span: true, text: { state: 'wrapped' } },
    }),
    {
      tool: 'WebFetch',
      output: '3.15M output tokens',
      prompt: '› last message’s words not shown (wrapped input)',
    },
  ],
  [
    'differing copies of the message in the session file',
    indexed({
      prompt: { state: 'found', at_ms: lane.start_ms, in_span: true, text: { state: 'ambiguous' } },
    }),
    {
      tool: 'WebFetch',
      output: '3.15M output tokens',
      prompt: '› last message’s words not shown: the session file holds differing copies',
    },
  ],
  [
    'words still waiting for a read slot, beside the measured detail',
    indexed({
      prompt: { state: 'found', at_ms: lane.start_ms, in_span: true, text: { state: 'busy' } },
    }),
    {
      tool: 'WebFetch',
      output: '3.15M output tokens',
      prompt: '› last message’s words not read yet: other session files are being read',
    },
  ],
  [
    'no tool calls and no person’s message',
    indexed({ tool: { state: 'no_calls' }, output_tokens: 0, prompt: { state: 'no_message' } }),
    { tool: 'no tools', output: '0 output tokens', prompt: null },
  ],
  [
    'an unclassified last message',
    indexed({ prompt: { state: 'unclassified' } }),
    {
      tool: 'WebFetch',
      output: '3.15M output tokens',
      prompt: '› last message unknown: not classified',
    },
  ],
])('states %s honestly', async (_, detail, expected) => {
  const { bar } = setup(async () => detail);
  fireEvent.focus(await waitFor(bar));
  const bubble = await screen.findByRole('tooltip');
  await waitFor(() =>
    expect(bubble.querySelector('.xt-span-bubble-tool')?.textContent).toBe(expected.tool),
  );
  expect(bubble.querySelector('.xt-span-bubble-output')!.textContent).toBe(expected.output);
  expect(bubble.querySelector('.xt-span-bubble-prompt')?.textContent ?? null).toBe(expected.prompt);
  if (expected.prompt === null) expect(bubble.querySelector('hr')).toBeNull();
});

it('says when the session is not indexed or the read failed', async () => {
  const { bar } = setup(async () => ({ state: 'missing' }));
  fireEvent.focus(await waitFor(bar));
  expect(await screen.findByText('session not in the index')).toBeTruthy();
  cleanup();
  const failed = setup(async () => {
    throw new Error('read failed');
  });
  fireEvent.focus(await waitFor(failed.bar));
  expect(await screen.findByText('detail could not be read')).toBeTruthy();
});

it('cancels a read still running when its bar goes away', async () => {
  const { read, cancel, bar } = setup(() => new Promise(() => {}));
  fireEvent.focus(await waitFor(bar));
  await screen.findByText('reading…');
  const readId = read.mock.calls[0][3];
  cleanup();
  await waitFor(() => expect(cancel).toHaveBeenCalledExactlyOnceWith(readId));
});

it('cancels a read still running when its bubble closes, and reads again on the next open', async () => {
  const { read, cancel, bar } = setup(() => new Promise(() => {}));
  const span = await waitFor(bar);
  fireEvent.focus(span);
  await screen.findByText('reading…');
  const readId = read.mock.calls[0][3];
  fireEvent.blur(span);
  await waitFor(() => expect(cancel).toHaveBeenCalledExactlyOnceWith(readId));
  fireEvent.focus(span);
  await screen.findByText('reading…');
  await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(read.mock.calls[1][3]).not.toBe(readId);
});

it('asks again for words that only waited for a read slot', async () => {
  const busy = indexed({
    prompt: { state: 'found', at_ms: lane.start_ms, in_span: true, text: { state: 'busy' } },
  });
  const { read, bar } = setup(async () => busy);
  const span = await waitFor(bar);
  fireEvent.focus(span);
  await screen.findByText(/not read yet/);
  const settled = read.mock.calls.length;
  fireEvent.blur(span);
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  fireEvent.focus(span);
  await screen.findByRole('tooltip');
  await waitFor(() => expect(read.mock.calls.length).toBeGreaterThan(settled));
});

it('drops every span answer read before stored content was deleted', async () => {
  const saying = (text: string) =>
    indexed({
      prompt: {
        state: 'found',
        at_ms: lane.start_ms,
        in_span: true,
        text: { state: 'stored', text, truncated: false },
      },
    });
  let hold = false;
  const pending: ((detail: DashboardSpanDetail) => void)[] = [];
  const { source, read, cancel, bar } = setup(() =>
    hold
      ? new Promise((done) => pending.push(done))
      : Promise.resolve(saying('words read before the delete')),
  );
  const span = await waitFor(bar);
  fireEvent.focus(span);
  await screen.findByText('› words read before the delete');
  fireEvent.blur(span);
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  // A closed bubble's kept answer is gone after the purge: the next open
  // reads again and never shows it meanwhile.
  hold = true;
  act(() => source.emit(events.contentPurged));
  const before = read.mock.calls.length;
  fireEvent.focus(span);
  const bubble = await screen.findByRole('tooltip');
  expect(bubble.textContent).toContain('reading…');
  expect(bubble.textContent).not.toContain('words read before the delete');
  await waitFor(() => expect(read.mock.calls.length).toBeGreaterThan(before));
  // A read still running when the next purge lands is cancelled, and its
  // late answer is never shown; a fresh read answers instead.
  const running = read.mock.calls.at(-1)![3];
  const late = pending.length;
  act(() => source.emit(events.contentPurged));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith(running));
  await waitFor(() => expect(pending.length).toBeGreaterThan(late));
  await act(async () => pending[late - 1](saying('a snapshot from before the delete')));
  expect(screen.getByRole('tooltip').textContent).not.toContain('a snapshot from before');
  await act(async () => pending.at(-1)!(saying('words read after the delete')));
  await screen.findByText('› words read after the delete');
  expect(screen.getByRole('tooltip').textContent).not.toContain('a snapshot from before');
});

it('keeps an open bubble open while a live span grows', async () => {
  const { read, bar, show } = setup(async () => indexed({}));
  const span = await waitFor(bar);
  fireEvent.focus(span);
  await screen.findByText('WebFetch');
  // The next report: the same span, five minutes longer.
  const grown = structuredClone(exported.dashboards[0]);
  grown.lanes[0].end_ms += 5 * 60_000;
  show(grown);
  // The very same bar, still open and now 28 minutes long.
  expect(bar()).toBe(span);
  expect(span.hasAttribute('data-popup-open')).toBe(true);
  const bubble = screen.getByRole('tooltip');
  expect(bubble.querySelector('.xt-span-bubble-duration')!.textContent).toBe('0 h 28 m');
  // The grown span is read for itself, with the earlier answer shown meanwhile.
  expect(bubble.textContent).toContain('WebFetch');
  await waitFor(() =>
    expect(read).toHaveBeenCalledWith(
      lane.session_id,
      lane.start_ms,
      lane.end_ms + 5 * 60_000,
      expect.any(String),
    ),
  );
});

describe('the automatic line', () => {
  const open = async (detail: DashboardSpanDetail) => {
    const { bar } = setup(async () => detail);
    fireEvent.focus(await waitFor(bar));
    const bubble = await screen.findByRole('tooltip');
    await waitFor(() => expect(bubble.querySelector('.xt-span-bubble-tool')).not.toBeNull());
    return bubble;
  };

  it('follows the person’s message, labelled automatic, muted and never marked ›', async () => {
    const bubble = await open(
      indexed({
        automatic: notification({
          state: 'stored',
          text: 'Agent "Map prompt classification sources" finished',
          truncated: false,
        }),
      }),
    );
    const prompt = bubble.querySelector('.xt-span-bubble-prompt')!;
    const automatic = bubble.querySelector('.xt-span-bubble-automatic')!;
    expect(prompt.textContent).toBe('› make the PR link exact, not inferred');
    expect(automatic.textContent).toBe(
      '(Automatic) Agent "Map prompt classification sources" finished',
    );
    expect(automatic.textContent).not.toContain('›');
    expect(automatic.hasAttribute('data-quiet')).toBe(true);
    // Divider, the person's message, then the automatic line.
    const lines = [...bubble.querySelectorAll('hr, p')].slice(-3);
    expect(lines.map((line) => line.className)).toEqual([
      'xt-span-bubble-rule',
      'xt-span-bubble-prompt',
      'xt-span-bubble-automatic',
    ]);
  });

  it('stands alone under the divider when the session has no person’s message', async () => {
    const bubble = await open(
      indexed({
        prompt: { state: 'no_message' },
        automatic: notification({
          state: 'stored',
          text: 'Background command "pnpm check" completed (exit code 0)',
          truncated: true,
        }),
      }),
    );
    expect(bubble.querySelector('hr')).not.toBeNull();
    expect(bubble.querySelector('.xt-span-bubble-prompt')).toBeNull();
    expect(bubble.querySelector('.xt-span-bubble-automatic')!.textContent).toBe(
      '(Automatic) Background command "pnpm check" completed (exit code 0)…',
    );
  });

  it.each<[string, DashboardAutomaticText]>([
    ['no summary', { state: 'no_summary' }],
    ['a record missing from the source', { state: 'not_found' }],
    ['differing copies', { state: 'ambiguous' }],
    ['a busy read', { state: 'busy' }],
    ['an unreadable source', { state: 'unavailable', reason: { reason: 'replaced' } }],
  ])('is omitted, adding no error line, for %s', async (_, text) => {
    const bubble = await open(indexed({ automatic: notification(text) }));
    expect(bubble.querySelector('.xt-span-bubble-automatic')).toBeNull();
    expect(bubble.querySelector('.xt-span-bubble-prompt')!.textContent).toBe(
      '› make the PR link exact, not inferred',
    );
    expect(bubble.textContent).not.toContain('Automatic');
  });

  it('adds no divider when there is neither a person’s message nor a summary', async () => {
    const bubble = await open(
      indexed({ prompt: { state: 'no_message' }, automatic: notification({ state: 'not_found' }) }),
    );
    expect(bubble.querySelector('hr')).toBeNull();
  });
});

it.each([
  ['complete', { total_usd: 1.25, priced_observations: 2, unpriced_observations: 0 }, '$1.25'],
  ['partial', { total_usd: null, priced_observations: 1, unpriced_observations: 1 }, '$1.25+'],
  [
    'unpriced',
    { total_usd: null, priced_observations: 0, unpriced_observations: 2 },
    'cost unknown',
  ],
  [
    'no responses',
    { total_usd: null, selected_observations: 0, priced_observations: 0, unpriced_observations: 0 },
    'cost unknown',
  ],
  ['measured zero', { total_usd: 0, priced_subtotal_usd: 0 }, '$0.00'],
])(
  'shows %s cost beside output tokens without assigning it to the prompt',
  async (_, overrides, expected) => {
    const detail = indexed({});
    if (detail.state !== 'indexed') throw new Error('indexed');
    Object.assign(detail.cost, overrides);
    const { bar } = setup(async () => detail);
    fireEvent.focus(await waitFor(bar));
    const bubble = await screen.findByRole('tooltip');
    await waitFor(() =>
      expect(bubble.querySelector('.xt-span-bubble-cost')?.textContent).toBe(expected),
    );
    expect(bubble.querySelector('.xt-span-bubble-output')?.nextElementSibling?.className).toBe(
      'xt-span-bubble-cost',
    );
    expect(bubble.querySelector('.xt-span-bubble-cost')?.getAttribute('title')).toContain(
      'recorded response usage in this span at public API prices',
    );
    expect(bubble.querySelector('.xt-span-bubble-prompt')?.textContent).toBe(
      '› make the PR link exact, not inferred',
    );
  },
);
