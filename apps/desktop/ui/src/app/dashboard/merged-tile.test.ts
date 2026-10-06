import { expect, it } from 'vitest';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { PrAutoCheckStatus } from '../../data/generated/PrAutoCheckStatus';
import {
  mergedTileView,
  prAttention,
  prAttentionPointer,
  prAttentionText,
  prCheckCounts,
  type AutoCheck,
} from './pr-effort';

const window: DashboardWindow = {
  days: 7,
  start_ms: Date.UTC(2026, 8, 1),
  end_ms: Date.UTC(2026, 8, 8),
  timezone: 'UTC',
  clock: 'fixture',
};
const tile = (
  counts: Partial<Omit<MetricMergedPrs, 'freshness'>>,
  freshness: Partial<MetricMergedPrs['freshness']> = {},
): MetricMergedPrs => {
  const unknown = counts.unknown_facts ?? 0;
  const known = counts.known_merged ?? 0;
  return {
    known_merged: known,
    unknown_facts: unknown,
    complete: unknown === 0,
    merged: unknown === 0 ? known : null,
    unresolved_type: 0,
    ...counts,
    freshness: {
      never_attempted: 0,
      refreshed: 0,
      failed_never_refreshed: 0,
      failed_after_refresh: 0,
      oldest_refreshed_at: null,
      newest_attempted_at: null,
      ...freshness,
    },
  };
};
const auto = (status: Partial<PrAutoCheckStatus> = {}): PrAutoCheckStatus => ({
  enabled: true,
  checking: false,
  paused: null,
  last_finished_at_ms: null,
  ...status,
});

it('says how many were never checked instead of showing a zero', () => {
  const view = mergedTileView(tile({ unknown_facts: 32 }, { never_attempted: 32 }), auto(), window);
  expect(view).toEqual({
    value: null,
    placeholder: '32 PRs not checked yet',
    aside: undefined,
    freshness: '32 PRs not checked yet.',
  });
});

it('says it is checking while an automatic run is going', () => {
  const view = mergedTileView(
    tile({ unknown_facts: 32 }, { never_attempted: 32 }),
    auto({ checking: true }),
    window,
  );
  // It does not claim which ones it is checking: only the unchecked count.
  expect(view.placeholder).toBe('32 PRs not checked yet');
  expect(view.aside).toBe('checking…');
  expect(view.freshness).toBe('Checking GitHub now… 32 PRs not checked yet.');
});

it('shows the known merged count with how many are still not checked', () => {
  const view = mergedTileView(
    tile(
      { known_merged: 5, unknown_facts: 3 },
      { never_attempted: 3, refreshed: 7, newest_attempted_at: Date.UTC(2026, 8, 7, 14, 3) },
    ),
    auto(),
    window,
  );
  expect(view).toEqual({
    value: 5,
    incomplete: true,
    aside: '3 not checked yet',
    freshness: 'Last checked Sep 7, 14:03. 3 PRs not checked yet.',
  });
  expect(
    mergedTileView(
      tile({ known_merged: 5, unknown_facts: 3 }, { never_attempted: 3 }),
      auto({ checking: true }),
      window,
    ).aside,
  ).toBe('checking…');
});

it('shows a known zero marked incomplete once GitHub has answered for some', () => {
  // 30 linked pull requests checked, none merged in range, 2 checks failing.
  const view = mergedTileView(
    tile({ unknown_facts: 2 }, { refreshed: 30, failed_never_refreshed: 2 }),
    auto(),
    window,
  );
  expect(view.value).toBe(0);
  expect(view.placeholder).toBeUndefined();
  expect(view.incomplete).toBe(true);
  expect(view.aside).toBe('2 could not be checked');
  expect(
    mergedTileView(
      tile(
        { known_merged: 1, unknown_facts: 3 },
        { refreshed: 4, never_attempted: 1, failed_after_refresh: 2 },
      ),
      auto(),
      window,
    ).aside,
  ).toBe('1 not checked yet');
});

it('a known count marked incomplete still says why checks paused', () => {
  const view = mergedTileView(
    tile({ unknown_facts: 2 }, { refreshed: 5, never_attempted: 2 }),
    auto({ paused: 'gh_signed_out' }),
    window,
  );
  expect(view.value).toBe(0);
  expect(view.incomplete).toBe(true);
  expect(view.aside).toBe('gh not signed in');
});

it('tells a failed check apart from one that never ran', () => {
  const view = mergedTileView(
    tile({ unknown_facts: 3 }, { never_attempted: 2, failed_never_refreshed: 1 }),
    auto(),
    window,
  );
  expect(view.placeholder).toBe('2 PRs not checked yet');
  expect(view.aside).toBe('1 could not be checked');
  expect(view.freshness).toBe(
    '2 PRs not checked yet. 1 PR could not be checked; the last try failed. Click the red ! on the Effort card to check it again.',
  );
  expect(
    mergedTileView(tile({ unknown_facts: 1 }, { failed_never_refreshed: 1 }), auto(), window)
      .placeholder,
  ).toBe('1 PR could not be checked');
});

it('a complete count is a number, and a paused or switched-off check says why', () => {
  const complete = mergedTileView(
    tile({ known_merged: 4 }, { refreshed: 4, newest_attempted_at: Date.UTC(2026, 8, 7, 9) }),
    auto(),
    window,
  );
  expect(complete).toEqual({
    value: 4,
    aside: undefined,
    freshness: 'Last checked Sep 7, 09:00.',
  });
  const missing = mergedTileView(
    tile({ unknown_facts: 2 }, { never_attempted: 2 }),
    auto({ paused: 'gh_missing' }),
    window,
  );
  expect(missing.aside).toBe('gh not found');
  expect(missing.freshness).toBe(
    'XTrace could not run the GitHub CLI (gh), so it stopped checking. 2 PRs not checked yet. Install gh, then click the red ! on the Effort card to check again.',
  );
  const signedOut = mergedTileView(
    tile({ unknown_facts: 5 }, { never_attempted: 2, failed_never_refreshed: 3 }),
    auto({ paused: 'gh_signed_out' }),
    window,
  );
  expect(signedOut.aside).toBe('gh not signed in');
  expect(signedOut.freshness).toBe(
    'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. 2 PRs not checked yet. 3 PRs could not be checked; the last try failed. Run gh auth login, then click the red ! on the Effort card to check them again.',
  );
  expect(
    mergedTileView(
      tile({ unknown_facts: 2 }, { never_attempted: 2 }),
      auto({ enabled: false }),
      window,
    ).freshness,
  ).toBe(
    '2 PRs not checked yet. Automatic checks are off. Click the red ! on the Effort card to check them.',
  );
  // Off, but every pull request already checked: nothing to point to.
  expect(
    mergedTileView(tile({ known_merged: 2 }, { refreshed: 2 }), auto({ enabled: false }), window)
      .freshness,
  ).toBe('Automatic checks are off.');
  expect(mergedTileView(tile({}), auto(), window)).toEqual({
    value: 0,
    aside: undefined,
    freshness: 'No agent session is linked to a pull request yet.',
  });
});

/** A tile whose undecided count is what its freshness says, plus `noMergeTime` more. */
const counted = (counts: Partial<MetricMergedPrs['freshness']> = {}, noMergeTime = 0) =>
  tile(
    {
      known_merged: counts.refreshed ?? 0,
      unknown_facts:
        (counts.never_attempted ?? 0) + (counts.failed_never_refreshed ?? 0) + noMergeTime,
    },
    counts,
  );
const none = { paused: null, couldNot: 0, stale: 0, unchecked: 0 };

it('needs the user only for a failed check, a gh pause, or unchecked ones nothing will check', () => {
  // Not checked yet while automatic checks run: XTrace checks them by itself.
  expect(prAttention(counted({ never_attempted: 4 }), auto())).toBeNull();
  expect(prAttention(counted({ never_attempted: 4 }), auto({ checking: true }))).toBeNull();
  // The status not known yet is not a reason either.
  expect(prAttention(counted({ never_attempted: 4 }), undefined)).toBeNull();
  expect(prAttention(counted({ refreshed: 9 }), auto())).toBeNull();
  // Merged without a merge time is not a failed check: checking again changes nothing.
  expect(prAttention(counted({ refreshed: 2 }, 1), auto())).toBeNull();
  // A failed last check, with or without earlier facts.
  expect(prAttention(counted({ failed_never_refreshed: 2 }), auto())).toEqual({
    ...none,
    couldNot: 2,
  });
  expect(prAttention(counted({ failed_after_refresh: 1 }), auto())).toEqual({ ...none, stale: 1 });
  // gh stopped the automatic check, whatever the counts.
  expect(prAttention(counted({ refreshed: 3 }), auto({ paused: 'gh_signed_out' }))).toEqual({
    ...none,
    paused: 'gh_signed_out',
  });
  // Off (or a source that never checks on its own) with some never checked.
  expect(prAttention(counted({ never_attempted: 3 }), auto({ enabled: false }))).toEqual({
    ...none,
    unchecked: 3,
  });
  expect(prAttention(counted({ never_attempted: 3 }), null)?.unchecked).toBe(3);
  expect(prAttention(counted({ refreshed: 3 }), auto({ enabled: false }))).toBeNull();
});

it('says in plain words what is wrong and what clicking does, with "it" for one', () => {
  expect(prAttentionText({ ...none, couldNot: 2 })).toBe(
    '2 pull requests could not be checked. Click to check them again.',
  );
  expect(prAttentionText({ ...none, couldNot: 1 })).toBe(
    '1 pull request could not be checked. Click to check it again.',
  );
  expect(prAttentionText({ ...none, stale: 1 })).toBe(
    'The last check of 1 pull request failed; older facts are shown. Click to check it again.',
  );
  expect(prAttentionText({ ...none, paused: 'gh_missing' })).toBe(
    'XTrace could not run the GitHub CLI (gh), so it stopped checking. Install gh, then click to check again.',
  );
  expect(prAttentionText({ ...none, paused: 'gh_signed_out', couldNot: 1 })).toBe(
    'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. 1 pull request could not be checked. Run gh auth login, then click to check it again.',
  );
  expect(prAttentionText({ ...none, unchecked: 1 })).toBe(
    'Automatic checks are off and 1 pull request was never checked. Click to check it.',
  );
  expect(prAttentionText({ ...none, unchecked: 3 })).toBe(
    'Automatic checks are off and 3 pull requests were never checked. Click to check them.',
  );
  expect(prAttentionText({ ...none, couldNot: 2 }, 'dialog')).toBe(
    '2 pull requests could not be checked. Choose them below and refresh them again.',
  );
  expect(prAttentionText({ ...none, stale: 1 }, 'dialog')).toBe(
    'The last check of 1 pull request failed; older facts are shown. Choose it below and refresh it again.',
  );
});

it('uses the same words for the same count on the tile and on the red !', () => {
  // Three checked, one failed with no facts, one failed with earlier facts,
  // one merged with no merge time.
  const probe = tile(
    { known_merged: 2, unknown_facts: 2 },
    { refreshed: 3, failed_never_refreshed: 1, failed_after_refresh: 1 },
  );
  expect(prCheckCounts(probe)).toEqual({ notChecked: 0, couldNot: 1, stale: 1, noMergeTime: 1 });
  const view = mergedTileView(probe, auto(), window);
  expect(view.aside).toBe('1 could not be checked');
  expect(view.freshness).toBe(
    '1 PR could not be checked; the last try failed. The last check of 1 PR failed; older facts are shown. 1 PR merged with no merge time. Click the red ! on the Effort card to check them again.',
  );
  expect(prAttentionText(prAttention(probe, auto())!)).toBe(
    '1 pull request could not be checked. The last check of 1 pull request failed; older facts are shown. Click to check them again.',
  );
});

it('names what the tile tip points to, even when the only reason is a stale pull request', () => {
  const stale = counted({ refreshed: 2, failed_after_refresh: 1 });
  expect(mergedTileView(stale, auto(), window).freshness).toBe(
    'The last check of 1 PR failed; older facts are shown. Click the red ! on the Effort card to check it again.',
  );
  expect(prAttentionPointer({ ...none, unchecked: 2 })).toBe(
    'Click the red ! on the Effort card to check them.',
  );
  expect(prAttentionPointer({ ...none, paused: 'gh_missing' })).toBe(
    'Install gh, then click the red ! on the Effort card to check again.',
  );
});

it('points to the red ! in the tile exactly when the red ! is shown', () => {
  const cases: [MetricMergedPrs, AutoCheck][] = [
    [counted({ never_attempted: 2 }), auto()],
    [counted({ never_attempted: 2 }), auto({ checking: true })],
    [counted({ never_attempted: 2 }), auto({ enabled: false })],
    [counted({ never_attempted: 2 }), null],
    [counted({ never_attempted: 2 }), undefined],
    [counted({ refreshed: 2 }), auto({ enabled: false })],
    [counted({ refreshed: 2 }, 1), auto()],
    [counted({ refreshed: 1, failed_after_refresh: 1 }), auto()],
    [counted({ failed_never_refreshed: 1 }), auto()],
    [counted({ refreshed: 2 }), auto({ paused: 'gh_missing' })],
    [counted({ never_attempted: 1 }), auto({ paused: 'gh_signed_out' })],
  ];
  for (const [input, status] of cases) {
    const line = mergedTileView(input, status, window).freshness;
    const shown = prAttention(input, status) !== null;
    expect(line.includes('the red !'), line).toBe(shown);
  }
});
