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
      manual_failed_never_refreshed: 0,
      manual_failed_after_refresh: 0,
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
    freshness: '32 PRs are not checked yet.',
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
  expect(view.freshness).toBe('Checking GitHub now… 32 PRs are not checked yet.');
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
    freshness: 'Last checked Sep 7, 2:03 PM. 3 PRs are not checked yet.',
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
    '2 PRs are not checked yet. 1 PR could not be checked. The last try failed. Click the red ! on the Effort card to check it again.',
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
    freshness: 'Last checked Sep 7, 9:00 AM.',
  });
  const missing = mergedTileView(
    tile({ unknown_facts: 2 }, { never_attempted: 2 }),
    auto({ paused: 'gh_missing' }),
    window,
  );
  expect(missing.aside).toBe('gh not found');
  expect(missing.freshness).toBe(
    'XTrace could not run the GitHub CLI (gh), so it stopped checking. 2 PRs are not checked yet. Install gh, then click the red ! on the Effort card to check again.',
  );
  const signedOut = mergedTileView(
    tile({ unknown_facts: 5 }, { never_attempted: 2, failed_never_refreshed: 3 }),
    auto({ paused: 'gh_signed_out' }),
    window,
  );
  expect(signedOut.aside).toBe('gh not signed in');
  expect(signedOut.freshness).toBe(
    'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. 2 PRs are not checked yet. 3 PRs could not be checked. The last try failed. Run gh auth login, then click the red ! on the Effort card to check them again.',
  );
  expect(
    mergedTileView(
      tile({ unknown_facts: 2 }, { never_attempted: 2 }),
      auto({ enabled: false }),
      window,
    ).freshness,
  ).toBe(
    '2 PRs are not checked yet. Automatic checks are off. Click the red ! on the Effort card to check them.',
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
type Freshness = Partial<MetricMergedPrs['freshness']>;
/** The red ! for these counts; it must be there. */
const shown = (counts: Freshness, status: AutoCheck = auto(), noMergeTime = 0) =>
  prAttention(counted(counts, noMergeTime), status)!;
/** What the red ! names: its reason and its counts, without the tile's own. */
const reason = (counts: Freshness, status: AutoCheck = auto()) => {
  const attention = prAttention(counted(counts), status);
  return attention && { paused: attention.paused, unchecked: attention.unchecked };
};

it('needs the user only for a failed check, a gh pause, or unchecked ones nothing will check', () => {
  // Not checked yet while automatic checks run: XTrace checks them by itself.
  expect(reason({ never_attempted: 4 })).toBeNull();
  expect(reason({ never_attempted: 4 }, auto({ checking: true }))).toBeNull();
  // The status not known yet is not a reason either.
  expect(reason({ never_attempted: 4 }, undefined)).toBeNull();
  expect(reason({ refreshed: 9 })).toBeNull();
  // Merged without a merge time is not a failed check: checking again changes nothing.
  expect(prAttention(counted({ refreshed: 2 }, 1), auto())).toBeNull();
  // A failed last check, with or without earlier facts.
  expect(reason({ failed_never_refreshed: 2 })).toEqual({ paused: null, unchecked: 0 });
  expect(reason({ failed_after_refresh: 1 })).toEqual({ paused: null, unchecked: 0 });
  // gh stopped the automatic check, whatever the counts.
  expect(reason({ refreshed: 3 }, auto({ paused: 'gh_signed_out' }))).toEqual({
    paused: 'gh_signed_out',
    unchecked: 0,
  });
  // Off (or a source that never checks on its own) with some never checked.
  expect(reason({ never_attempted: 3 }, auto({ enabled: false }))).toEqual({
    paused: null,
    unchecked: 3,
  });
  expect(reason({ never_attempted: 3 }, null)?.unchecked).toBe(3);
  expect(reason({ refreshed: 3 }, auto({ enabled: false }))).toBeNull();
  // Failed ones a manual refresh already met do not count; others still do.
  expect(reason({ failed_never_refreshed: 1, manual_failed_never_refreshed: 1 })).toBeNull();
  expect(reason({ failed_after_refresh: 1, manual_failed_after_refresh: 1 })).toBeNull();
  expect(reason({ failed_never_refreshed: 2, manual_failed_never_refreshed: 1 })).not.toBeNull();
});

it('says in plain words what is wrong and what clicking does, with "it" for one', () => {
  expect(prAttentionText(shown({ failed_never_refreshed: 2 }))).toBe(
    '2 pull requests could not be checked. The last try failed. Click to check them again.',
  );
  expect(prAttentionText(shown({ failed_never_refreshed: 1 }))).toBe(
    '1 pull request could not be checked. The last try failed. Click to check it again.',
  );
  expect(prAttentionText(shown({ failed_after_refresh: 1 }))).toBe(
    '1 pull request is stale after a failed check. Older facts are shown. Click to check it again.',
  );
  expect(prAttentionText(shown({ refreshed: 1 }, auto({ paused: 'gh_missing' })))).toBe(
    'XTrace could not run the GitHub CLI (gh), so it stopped checking. Install gh, then click to check again.',
  );
  expect(
    prAttentionText(shown({ failed_never_refreshed: 1 }, auto({ paused: 'gh_signed_out' }))),
  ).toBe(
    'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. 1 pull request could not be checked. The last try failed. Run gh auth login, then click to check it again.',
  );
  expect(prAttentionText(shown({ never_attempted: 1 }, auto({ enabled: false })))).toBe(
    'Automatic checks are off and 1 pull request is not checked yet. Click to check it.',
  );
  expect(prAttentionText(shown({ never_attempted: 3 }, auto({ enabled: false })))).toBe(
    'Automatic checks are off and 3 pull requests are not checked yet. Click to check them.',
  );
  expect(prAttentionText(shown({ failed_never_refreshed: 2 }), 'dialog')).toBe(
    '2 pull requests could not be checked. The last try failed. Choose them below and refresh them again.',
  );
  expect(prAttentionText(shown({ failed_after_refresh: 1 }), 'dialog')).toBe(
    '1 pull request is stale after a failed check. Older facts are shown. Choose it below and refresh it again.',
  );
});

it('uses the same words for the same count on the tile and on the red !', () => {
  // Three checked, one failed with no facts, one failed with earlier facts,
  // one merged with no merge time.
  const probe = tile(
    { known_merged: 2, unknown_facts: 2 },
    { refreshed: 3, failed_never_refreshed: 1, failed_after_refresh: 1 },
  );
  expect(prCheckCounts(probe)).toEqual({
    notChecked: 0,
    couldNot: 1,
    stale: 1,
    triedCouldNot: 0,
    triedStale: 0,
    noMergeTime: 1,
  });
  const view = mergedTileView(probe, auto(), window);
  expect(view.aside).toBe('1 could not be checked');
  expect(view.freshness).toBe(
    '1 PR could not be checked. The last try failed. 1 PR is stale after a failed check. Older facts are shown. 1 PR merged with no merge time. Click the red ! on the Effort card to check them again.',
  );
  expect(prAttentionText(prAttention(probe, auto())!)).toBe(
    '1 pull request could not be checked. The last try failed. 1 pull request is stale after a failed check. Older facts are shown. Click to check them again.',
  );
});

it('names what the tile tip points to, even when the only reason is a stale pull request', () => {
  const stale = counted({ refreshed: 2, failed_after_refresh: 1 });
  expect(mergedTileView(stale, auto(), window).freshness).toBe(
    '1 PR is stale after a failed check. Older facts are shown. Click the red ! on the Effort card to check it again.',
  );
  expect(prAttentionPointer(shown({ never_attempted: 2 }, auto({ enabled: false })))).toBe(
    'Click the red ! on the Effort card to check them.',
  );
  expect(prAttentionPointer(shown({ refreshed: 1 }, auto({ paused: 'gh_missing' })))).toBe(
    'Install gh, then click the red ! on the Effort card to check again.',
  );
});

/**
 * Every text for one state: the tile's tip, and (when the red ! is shown) its
 * tip and the dialog's line. Each count and each pronoun in them must name
 * the same pull requests.
 */
const CASES: {
  counts: Freshness;
  status: AutoCheck;
  noMergeTime?: number;
  tile: string;
  tip?: string;
  dialog?: string;
}[] = [
  { counts: { never_attempted: 2 }, status: auto(), tile: '2 PRs are not checked yet.' },
  {
    counts: { never_attempted: 2 },
    status: auto({ checking: true }),
    tile: 'Checking GitHub now… 2 PRs are not checked yet.',
  },
  {
    counts: { never_attempted: 2 },
    status: auto({ enabled: false }),
    tile: '2 PRs are not checked yet. Automatic checks are off. Click the red ! on the Effort card to check them.',
    tip: 'Automatic checks are off and 2 pull requests are not checked yet. Click to check them.',
    dialog:
      'Automatic checks are off and 2 pull requests are not checked yet. Choose them below and refresh them.',
  },
  {
    counts: { never_attempted: 2 },
    status: null,
    tile: '2 PRs are not checked yet. Click the red ! on the Effort card to check them.',
    tip: 'Automatic checks are off and 2 pull requests are not checked yet. Click to check them.',
  },
  { counts: { never_attempted: 2 }, status: undefined, tile: '2 PRs are not checked yet.' },
  {
    counts: { refreshed: 2 },
    status: auto({ enabled: false }),
    tile: 'Automatic checks are off.',
  },
  {
    counts: { refreshed: 2 },
    status: auto(),
    noMergeTime: 1,
    tile: '1 PR merged with no merge time.',
  },
  {
    counts: { refreshed: 1, failed_after_refresh: 1 },
    status: auto(),
    tile: '1 PR is stale after a failed check. Older facts are shown. Click the red ! on the Effort card to check it again.',
    tip: '1 pull request is stale after a failed check. Older facts are shown. Click to check it again.',
  },
  {
    counts: { failed_never_refreshed: 1 },
    status: auto(),
    tile: '1 PR could not be checked. The last try failed. Click the red ! on the Effort card to check it again.',
    tip: '1 pull request could not be checked. The last try failed. Click to check it again.',
  },
  {
    counts: { refreshed: 2 },
    status: auto({ paused: 'gh_missing' }),
    tile: 'XTrace could not run the GitHub CLI (gh), so it stopped checking. Install gh, then click the red ! on the Effort card to check again.',
    tip: 'XTrace could not run the GitHub CLI (gh), so it stopped checking. Install gh, then click to check again.',
    // Nothing in particular is waiting: the dialog names what to choose.
    dialog:
      'XTrace could not run the GitHub CLI (gh), so it stopped checking. Install gh, then choose pull requests below and refresh them.',
  },
  {
    counts: { never_attempted: 1 },
    status: auto({ paused: 'gh_signed_out' }),
    tile: 'The GitHub CLI (gh) is not signed in, so XTrace stopped checking. 1 PR is not checked yet. Run gh auth login, then click the red ! on the Effort card to check again.',
  },
  // Failures a manual refresh already met: listed with how many, no pointer.
  {
    counts: { failed_never_refreshed: 1, manual_failed_never_refreshed: 1 },
    status: auto(),
    tile: '1 PR could not be checked. The last try failed. Your own check of it failed too.',
  },
  {
    counts: { refreshed: 1, failed_after_refresh: 2, manual_failed_after_refresh: 2 },
    status: auto(),
    tile: '2 PRs are stale after a failed check. Older facts are shown. Your own checks of both failed too.',
  },
  // Two failed, one of them already met by hand: the same two everywhere,
  // and the pointer names the other one.
  {
    counts: { failed_never_refreshed: 2, manual_failed_never_refreshed: 1 },
    status: auto(),
    tile: '2 PRs could not be checked. The last try failed. 1 of them also failed when you checked it yourself. Click the red ! on the Effort card to check the other one again.',
    tip: '2 pull requests could not be checked. The last try failed. 1 of them also failed when you checked it yourself. Click to check the other one again.',
    dialog:
      '2 pull requests could not be checked. The last try failed. 1 of them also failed when you checked it yourself. Choose the other one below and refresh it again.',
  },
  {
    counts: { failed_never_refreshed: 4, manual_failed_never_refreshed: 2 },
    status: auto(),
    tile: '4 PRs could not be checked. The last try failed. 2 of them also failed when you checked them yourself. Click the red ! on the Effort card to check the other 2 again.',
  },
  {
    counts: { refreshed: 1, failed_after_refresh: 3, manual_failed_after_refresh: 3 },
    status: auto(),
    tile: '3 PRs are stale after a failed check. Older facts are shown. Your own checks of all 3 failed too.',
  },
  {
    counts: { failed_never_refreshed: 4, manual_failed_never_refreshed: 1 },
    status: auto(),
    tile: '4 PRs could not be checked. The last try failed. 1 of them also failed when you checked it yourself. Click the red ! on the Effort card to check the other 3 again.',
    tip: '4 pull requests could not be checked. The last try failed. 1 of them also failed when you checked it yourself. Click to check the other 3 again.',
    dialog:
      '4 pull requests could not be checked. The last try failed. 1 of them also failed when you checked it yourself. Choose the other 3 below and refresh them again.',
  },
  {
    counts: { failed_never_refreshed: 1, manual_failed_never_refreshed: 1 },
    status: auto({ paused: 'gh_missing' }),
    tile: 'XTrace could not run the GitHub CLI (gh), so it stopped checking. 1 PR could not be checked. The last try failed. Your own check of it failed too. Install gh, then click the red ! on the Effort card to check again.',
    tip: 'XTrace could not run the GitHub CLI (gh), so it stopped checking. 1 pull request could not be checked. The last try failed. Your own check of it failed too. Install gh, then click to check again.',
  },
];

it('says the same counts and pronouns on the tile, the red ! and the dialog, and points only when shown', () => {
  for (const { counts, status, noMergeTime = 0, tile: line, tip, dialog } of CASES) {
    const input = counted(counts, noMergeTime);
    expect(mergedTileView(input, status, window).freshness, line).toBe(line);
    const attention = prAttention(input, status);
    // The pointer is said exactly when the red ! is shown.
    expect(line.includes('the red !'), line).toBe(attention !== null);
    if (tip) expect(prAttentionText(attention!)).toBe(tip);
    if (dialog) expect(prAttentionText(attention!, 'dialog')).toBe(dialog);
  }
});

it('stops asking about a pull request a manual refresh already failed for', () => {
  const tried = counted({
    refreshed: 2,
    failed_never_refreshed: 1,
    failed_after_refresh: 1,
    manual_failed_never_refreshed: 1,
    manual_failed_after_refresh: 1,
  });
  expect(prAttention(tried, auto())).toBeNull();
  // Still stated as failed in the tile's words, with nothing pointing to a red !.
  expect(mergedTileView(tried, auto(), window).freshness).toBe(
    '1 PR could not be checked. The last try failed. Your own check of it failed too. 1 PR is stale after a failed check. Older facts are shown. Your own check of it failed too.',
  );
  // The other reasons still stand: a gh pause, or another failure not yet tried by hand.
  expect(reason({ refreshed: 1 }, auto({ paused: 'gh_signed_out' }))?.paused).toBe('gh_signed_out');
  // Automatic checks off: a never-checked one still counts as before.
  expect(reason({ never_attempted: 1 }, auto({ enabled: false }))?.unchecked).toBe(1);
});
