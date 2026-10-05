import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { CompactionCount } from '../data/generated/CompactionCount';
import type { CompactionReason } from '../data/generated/CompactionReason';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionCompactions } from '../data/generated/SessionCompactions';
import { events } from '../data/ipc-names';
import { useSessionCompactions } from './session-compactions';

const verified: CompactionCount = {
  state: 'count',
  count: 100,
  events: Array.from({ length: 4 }, (_, index) => ({ at_ms: index + 1, trigger: 'auto' })),
};
const fresh: CompactionCount = {
  state: 'count',
  count: 101,
  events: Array.from({ length: 5 }, (_, index) => ({ at_ms: index + 1, trigger: 'auto' })),
};
const answer = (
  outcome: CompactionCount,
  ids: readonly string[] = ['main'],
): SessionCompactions => ({
  counts: ids.map((id) => ({ id, outcome })),
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
async function setup(ids = ['main'], retryTransient = true) {
  const data = Object.assign(new FixtureDataSource(structuredClone(fixture as FixtureExport)), {
    compactions: {
      read: vi.fn(async (ids: readonly string[], readId: string) => {
        void readId;
        return answer(verified, ids);
      }),
      cancel: vi.fn(async (_readId: string) => {
        void _readId;
      }),
    },
  });
  let currentSource = data;
  const hook = renderHook(
    ({ ids, scope }) =>
      useSessionCompactions(scope, ids, { keepResolved: retryTransient, retryTransient }),
    {
      initialProps: { ids, scope: 'dashboard:7d' },
      wrapper: ({ children }: { children: ReactNode }) => (
        <DataProvider source={currentSource}>{children}</DataProvider>
      ),
    },
  );
  await waitFor(() => expect(hook.result.current(ids[0])).toEqual(verified));
  const initialReads = data.compactions.read.mock.calls.length;
  vi.useFakeTimers();
  const advance = async (ms: number) =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  const refresh = async () => {
    act(() => data.emit(events.turnCompleted));
    await advance(501);
  };
  const replaceSource = (replacement: typeof data) => {
    currentSource = replacement;
    hook.rerender({ ids, scope: 'dashboard:7d' });
  };
  return { ...hook, data, advance, refresh, initialReads, replaceSource };
}

it.each(['transport', 'cancelled', 'limit', 'replaced', 'incomplete'] as const)(
  'keeps the verified count/times through one %s retry and applies the newer answer',
  async (reason) => {
    const { result, data, advance, refresh, initialReads } = await setup();
    if (reason === 'transport')
      data.compactions.read.mockRejectedValueOnce(new Error('read refused'));
    else data.compactions.read.mockResolvedValueOnce(answer({ state: 'unknown', reason }));
    let finish!: (value: SessionCompactions) => void;
    data.compactions.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    await refresh();
    expect(result.current('main')).toEqual(verified);
    expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + 1);
    await advance(300);
    expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + 2);
    expect(result.current('main')).toEqual(verified);
    await act(async () => finish(answer(fresh)));
    await advance(1);
    expect(result.current('main')).toEqual(fresh);
    await advance(1000);
    expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + 2);
  },
);

it.each(['transport', 'replaced'] as const)(
  'clears the count after the final %s failure without a third attempt',
  async (reason) => {
    const { result, data, advance, refresh, initialReads } = await setup();
    if (reason === 'transport')
      data.compactions.read
        .mockRejectedValueOnce(new Error('refused'))
        .mockRejectedValueOnce(new Error('still refused'));
    else
      data.compactions.read
        .mockResolvedValueOnce(answer({ state: 'unknown', reason }))
        .mockResolvedValueOnce(answer({ state: 'unknown', reason }));
    await refresh();
    expect(result.current('main')).toEqual(verified);
    await advance(301);
    expect(result.current('main')).toEqual(
      reason === 'transport' ? undefined : { state: 'unknown', reason },
    );
    await advance(1000);
    expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + 2);
  },
);

it.each([
  'missing',
  'unsupported',
  'identity_mismatch',
  'ownership',
  'ambiguous',
  'not_indexed',
  'unreadable',
] as const)(
  'applies terminal %s immediately while another row awaits its retry',
  async (reason: CompactionReason) => {
    const { result, data, advance, refresh } = await setup(['main', 'terminal']);
    data.compactions.read.mockResolvedValueOnce({
      counts: [
        { id: 'main', outcome: { state: 'unknown', reason: 'incomplete' } },
        { id: 'terminal', outcome: { state: 'unknown', reason } },
      ],
    });
    let finish!: (value: SessionCompactions) => void;
    data.compactions.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    await refresh();
    expect(result.current('main')).toEqual(verified);
    expect(result.current('terminal')).toEqual({ state: 'unknown', reason });
    await advance(300);
    expect(data.compactions.read.mock.calls.at(-1)![0]).toEqual(['main']);
    expect(result.current('terminal')).toEqual({ state: 'unknown', reason });
    await act(async () => finish(answer(fresh)));
    await advance(1);
    expect(result.current('main')).toEqual(fresh);
    expect(result.current('terminal')).toEqual({ state: 'unknown', reason });
  },
);

it.each(['visible', 'range', 'unmount'] as const)(
  'cancels the retry delay on %s replacement',
  async (change) => {
    const { data, rerender, unmount, advance, refresh, initialReads } = await setup();
    data.compactions.read.mockRejectedValueOnce(new Error('refused'));
    await refresh();
    expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + 1);
    if (change === 'unmount') unmount();
    else
      rerender({
        ids: change === 'visible' ? ['other'] : ['main'],
        scope: change === 'range' ? 'dashboard:14d' : 'dashboard:7d',
      });
    await advance(1000);
    expect(data.compactions.read).toHaveBeenCalledTimes(
      initialReads + (change === 'unmount' ? 1 : 2),
    );
  },
);

it('does not retry compaction failures on the other surfaces', async () => {
  const { result, data, advance, refresh, initialReads } = await setup(['main'], false);
  data.compactions.read.mockRejectedValueOnce(new Error('refused'));
  await refresh();
  expect(result.current('main')).toBeUndefined();
  await advance(1000);
  expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + 1);
});

it('cancels the native retry attempt and ignores its late answer after a visible selection change', async () => {
  const { result, data, rerender, advance, refresh } = await setup();
  data.compactions.read.mockRejectedValueOnce(new Error('refused'));
  let finish!: (value: SessionCompactions) => void;
  data.compactions.read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await refresh();
  await advance(300);
  const retryId = data.compactions.read.mock.calls.at(-1)![1];
  rerender({ ids: ['other'], scope: 'dashboard:7d' });
  await advance(1);
  expect(data.compactions.cancel).toHaveBeenCalledWith(retryId);
  await act(async () => finish(answer(fresh)));
  await advance(1);
  expect(result.current('main')).toEqual(verified);
  expect(result.current('other')).toEqual(verified);
});

it('keeps terminal results when the other row exhausts its transport retry', async () => {
  const { result, data, advance, refresh } = await setup(['main', 'terminal']);
  data.compactions.read
    .mockResolvedValueOnce({
      counts: [
        { id: 'main', outcome: { state: 'unknown', reason: 'cancelled' } },
        { id: 'terminal', outcome: { state: 'unknown', reason: 'ownership' } },
      ],
    })
    .mockRejectedValueOnce(new Error('refused'));
  await refresh();
  expect(result.current('terminal')).toEqual({ state: 'unknown', reason: 'ownership' });
  await advance(301);
  expect(result.current('main')).toBeUndefined();
  expect(result.current('terminal')).toEqual({ state: 'unknown', reason: 'ownership' });
});

it('keeps initial and retry commands within fifty IDs without retrying successful batches', async () => {
  const ids = Array.from({ length: 112 }, (_, index) => `session-${index}`);
  const { result, data, advance, refresh, initialReads } = await setup(ids);
  data.compactions.read.mockImplementation(async (named) =>
    named.includes(ids[0])
      ? answer({ state: 'unknown', reason: 'limit' }, named)
      : answer(fresh, named),
  );
  await refresh();
  expect(result.current(ids[0])).toEqual(verified);
  await advance(301);
  expect(result.current(ids[0])).toEqual({ state: 'unknown', reason: 'limit' });
  expect(result.current([...ids].sort().at(-1)!)).toEqual(fresh);
  const commands = data.compactions.read.mock.calls.slice(initialReads);
  expect(commands.map(([named]) => named.length)).toEqual([50, 50, 50, 12]);
  expect(commands.every(([named]) => new Set(named).size === named.length)).toBe(true);
});

it.each(['delay', 'native attempt'] as const)(
  'clears old source values and cancels its retry during %s',
  async (phase) => {
    const { result, data, advance, refresh, replaceSource, initialReads } = await setup();
    data.compactions.read.mockRejectedValueOnce(new Error('refused'));
    let finishOld!: (value: SessionCompactions) => void;
    if (phase === 'native attempt')
      data.compactions.read.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishOld = resolve;
          }),
      );
    await refresh();
    if (phase === 'native attempt') await advance(300);
    const oldRetryId = data.compactions.read.mock.calls.at(-1)![1];
    const pending: Array<(value: SessionCompactions) => void> = [];
    const replacement = Object.assign(
      new FixtureDataSource(structuredClone(fixture as FixtureExport)),
      {
        compactions: {
          read: vi.fn((ids: readonly string[], readId: string): Promise<SessionCompactions> => {
            void ids;
            void readId;
            return new Promise((resolve) => pending.push(resolve));
          }),
          cancel: vi.fn(async (_readId: string) => {
            void _readId;
          }),
        },
      },
    );
    replaceSource(replacement);
    await advance(1);
    expect(pending.length).toBeGreaterThan(0);
    expect(result.current('main')).toBeUndefined();
    if (phase === 'native attempt') {
      expect(data.compactions.cancel).toHaveBeenCalledWith(oldRetryId);
      await act(async () => finishOld(answer(fresh)));
      await advance(1);
      expect(result.current('main')).toBeUndefined();
    }
    await advance(1000);
    expect(data.compactions.read).toHaveBeenCalledTimes(initialReads + (phase === 'delay' ? 1 : 2));
    await act(async () => {
      for (const finish of pending) finish(answer(fresh));
    });
    await advance(1);
    expect(result.current('main')).toEqual(fresh);
  },
);
