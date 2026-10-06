// @vitest-environment node
import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import { COMPACTION_CAUTION, COMPACTION_MEANING } from '../session-compactions';
import { laneCostDefinition, laneDefinition, PR_MEANING } from './ActivityLanes';
import { EFFORT_CONTEXT } from './DashboardPage';
import { MERGED_COUNTS, mergedUnknown } from './OverviewCard';
import { prAttentionPointer } from './pr-effort';
import { DELTA_HIDDEN, excludedNote, plainReason, plainReasons } from './present';

/** The longest note a definition popover adds after the rule's summary. */
const NOTE_MAX = 120;

it('keeps every note a definition adds short and free of rule codes', () => {
  const hours48 = { lane_start_ms: 0, lane_end_ms: 48 * 3_600_000 } as DashboardMetrics;
  const notes = [
    PR_MEANING,
    laneDefinition(hours48),
    laneCostDefinition(48),
    EFFORT_CONTEXT,
    prAttentionPointer({ paused: 'gh_signed_out', couldNot: 2, stale: 1_234, unchecked: 0 }),
    MERGED_COUNTS,
    mergedUnknown({ unknown_facts: 1_234 } as MetricMergedPrs),
    DELTA_HIDDEN,
    excludedNote([
      {
        host: 'claude-code',
        surface: 'desktop-app',
        degenerate_sessions: 3,
        qualifying_sessions: 10,
      },
    ]),
    excludedNote(
      Array.from({ length: 12 }, (_, index) => ({
        host: `host-${index}`,
        surface: null,
        degenerate_sessions: 3,
        qualifying_sessions: 10,
      })),
    ),
    COMPACTION_MEANING,
    COMPACTION_CAUTION,
    ...Object.values(plainReasons),
  ];
  for (const note of notes) {
    expect(note, note).toBeTruthy();
    expect(note!.length, note).toBeLessThanOrEqual(NOTE_MAX);
    expect(note, note).not.toMatch(/\b(?:[A-Z]-\d{2}[a-z]?|F\d{2})\b/);
    // Joined sentences keep their punctuation.
    expect(note, note).toMatch(/[.!?]$/);
  }
});

it('says why a tile has no number in plain words, and keeps an unknown reason as written', () => {
  expect(plainReason('No positive-duration active spans')).toBe(
    'No measurable agent activity in this range.',
  );
  expect(plainReason('No measurable eligible hands-off stretches')).toBe(
    'No stretch where an agent worked on its own in this range.',
  );
  expect(plainReason('a reason added later')).toBe('A reason added later.');
});

it('has plain wording for every fixed tile reason the dashboard backend can send', () => {
  // Only the shipped code: the Rust test module's own placeholder reasons are not sent.
  const source = readFileSync(
    new URL('../../../../src-tauri/src/dashboard.rs', import.meta.url),
    'utf8',
  ).split('#[cfg(test)]')[0];
  const reasons = new Set([
    // tile(value, previous, samples, unit, "M-xx", "reason")
    ...[...source.matchAll(/"[A-Z]-\d{2}[a-z]?",\s*"([^"]+)",?\s*\)/g)].map((match) => match[1]),
    // unavailable("reason", unit, rule)
    ...[...source.matchAll(/\bunavailable\(\s*"([^"]+)"/g)].map((match) => match[1]),
  ]);
  // The scan finds the reasons it is meant to; a changed call shape fails here.
  expect(reasons.size).toBeGreaterThanOrEqual(10);
  for (const reason of reasons) expect(Object.keys(plainReasons), reason).toContain(reason);
});
