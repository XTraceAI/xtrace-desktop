import { expect, it, vi } from 'vitest';
import type { DataSource } from '../data/DataSource';
import type { AccountUsage } from '../data/generated/AccountUsage';
import {
  accountUsageQueryOptions,
  accountUsageReadingMs,
  accountUsageRefreshMs,
} from './account-usage-query';

it('uses only the passive command for mount, timer and focus reads', async () => {
  const passive = vi.fn(async () => ({ claude: null, codex: null }));
  const explicitClaude = vi.fn(async () => ({ claude: null, codex: null }));
  const source = {
    kind: 'native',
    accountUsage: passive,
    refreshClaudeUsage: explicitClaude,
  } as unknown as DataSource;
  const options = accountUsageQueryOptions(source);
  expect(options.queryKey).toEqual(['account-usage']);
  expect(options.refetchOnWindowFocus).toBe(true);
  expect(options.refetchInterval()).toBe(accountUsageRefreshMs);
  const reading: AccountUsage = {
    claude: { state: 'unavailable', issue: 'reading', checked_at: null, windows: [] },
    codex: { state: 'unavailable', issue: 'source_unavailable', checked_at: null, windows: [] },
  };
  expect(options.refetchInterval({ state: { data: reading } })).toBe(accountUsageReadingMs);
  expect(
    options.refetchInterval({
      state: { data: { ...reading, claude: { ...reading.claude, issue: 'timeout' } } },
    }),
  ).toBe(accountUsageRefreshMs);
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
  expect(options.refetchInterval()).toBe(false);
  vi.restoreAllMocks();
  await options.queryFn(); // mount
  await options.queryFn(); // interval
  await options.queryFn(); // overdue focus
  expect(passive).toHaveBeenCalledTimes(3);
  expect(explicitClaude).not.toHaveBeenCalled();
});
