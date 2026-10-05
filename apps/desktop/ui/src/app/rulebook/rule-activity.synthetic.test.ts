import { describe, expect, it } from 'vitest';
import type { RuleActivityFire } from '../../data/generated/RuleActivityFire';
import type { RuleActivityGroup } from '../../data/generated/RuleActivityGroup';
import type { RuleActivityModeCounts } from '../../data/generated/RuleActivityModeCounts';
import type { Loaded } from './rule-activity';
import * as synthetic from './rule-activity.synthetic';

/**
 * `ReadLimits::default()` in `crates/xt-rulebook/src/activity.rs`, which is
 * what the app reads with: tail bytes, lines, returned rows and groups.
 */
const MAX_TAIL_BYTES = 4 * 1024 * 1024;
const MAX_LINES = 10_000;
const MAX_ROWS = 100;
const MAX_GROUPS = 100;

const MODES = ['advise', 'gate', 'suppressed', 'unrecognized'] as const;
const noModes = (): RuleActivityModeCounts => ({
  advise: 0,
  gate: 0,
  suppressed: 0,
  unrecognized: 0,
});
const total = (modes: RuleActivityModeCounts) => MODES.reduce((sum, mode) => sum + modes[mode], 0);
const at = (instant: string) => Date.parse(instant);
const key = (row: { rulebook_id: string | null; rule_id: string }) =>
  JSON.stringify([row.rulebook_id, row.rule_id]);
/** The reader's group order: latest first, then (rulebook, rule) ascending, unscoped first. */
function groupOrder(a: RuleActivityGroup, b: RuleActivityGroup): number {
  const later = at(b.latest_observed) - at(a.latest_observed);
  if (later !== 0) return later;
  if (a.rulebook_id !== b.rulebook_id) {
    if (a.rulebook_id === null) return -1;
    if (b.rulebook_id === null) return 1;
    return a.rulebook_id < b.rulebook_id ? -1 : 1;
  }
  if (a.rule_id === b.rule_id) return 0;
  return a.rule_id < b.rule_id ? -1 : 1;
}

/** The groups `group()` in `activity.rs` would build from exactly these fires. */
function groupsOf(fires: readonly RuleActivityFire[]): RuleActivityGroup[] {
  const groups = new Map<string, RuleActivityGroup>();
  for (const fire of fires) {
    const group = groups.get(key(fire)) ?? {
      rulebook_id: fire.rulebook_id,
      rule_id: fire.rule_id,
      latest_observed: fire.fired_at,
      modes: noModes(),
    };
    group.modes[fire.mode.kind] += 1;
    if (at(fire.fired_at) > at(group.latest_observed)) group.latest_observed = fire.fired_at;
    groups.set(key(fire), group);
  }
  return [...groups.values()].sort(groupOrder);
}

/**
 * Every way a loaded answer contradicts how the R1 reader (`complete_lines`,
 * `scan`/`accept`, `into_snapshot` and `group` in `activity.rs`, under default
 * limits) and its one-to-one DTO mapping could have produced it. Empty when
 * the answer is one a read could return. Test-only: this is the finite set of
 * rules a fixture must obey, not a copy of the reader.
 */
function coherenceViolations(loaded: Loaded): string[] {
  const violations: string[] = [];
  const check = (holds: boolean, rule: string) => {
    if (!holds) violations.push(rule);
  };
  const { counts, coverage } = loaded;
  const { snapshot } = counts;
  const malformed = Object.values(snapshot.malformed).reduce((sum, value) => sum + value, 0);
  const inWindow = total(counts.window_modes);
  const captured = Number(coverage.captured_len);
  const start = Number(coverage.scanned_start);
  const end = Number(coverage.scanned_end);
  const windowStart = at(loaded.window_start);
  const windowEnd = at(loaded.window_end);
  const inside = (instant: string) => at(instant) >= windowStart && at(instant) < windowEnd;

  check(loaded.schema_version === 2, 'schema version is 2');
  check(windowStart < windowEnd, 'the window is not empty');

  // Line accounting (`scan`, `accept`, `into_snapshot`).
  check(
    snapshot.lines === snapshot.blank_lines + malformed + snapshot.valid_rows,
    'lines = blank + malformed + valid rows',
  );
  check(snapshot.lines <= MAX_LINES, 'lines stay within the line bound');
  check(
    snapshot.valid_rows ===
      snapshot.distinct_ids + snapshot.duplicate_rows + snapshot.conflicted_rows,
    'valid rows = distinct + duplicate + conflicted rows',
  );
  check(
    (snapshot.conflicted_ids === 0) === (snapshot.conflicted_rows === 0),
    'conflicted IDs and conflicted rows are zero together',
  );
  check(
    snapshot.conflicted_rows >= 2 * snapshot.conflicted_ids,
    'each conflicted ID has at least two rows',
  );
  check(inWindow <= snapshot.distinct_ids, 'in-window rows are distinct rows');

  // Byte and line bounds (`capture`, `complete_lines`).
  check(
    coverage.byte_bound_reached === captured > MAX_TAIL_BYTES,
    'the byte bound is reached exactly when the ledger is over 4 MiB',
  );
  if (coverage.byte_bound_reached)
    check(start >= captured - MAX_TAIL_BYTES, 'a byte-bound scan starts inside the tail');
  if (
    coverage.byte_bound_reached &&
    !coverage.leading_partial_dropped &&
    !coverage.line_bound_reached
  )
    check(
      start === captured - MAX_TAIL_BYTES,
      'a byte-bound scan on a line start starts at the tail',
    );
  if (!coverage.byte_bound_reached && !coverage.line_bound_reached)
    check(start === 0, 'an unbounded scan starts at byte 0');
  check(
    !coverage.leading_partial_dropped || coverage.byte_bound_reached,
    'only a byte-bound read can start mid-line',
  );
  if (coverage.line_bound_reached) {
    check(snapshot.lines === MAX_LINES, 'a line-bound scan kept exactly 10,000 lines');
    check(start > 0, 'a line-bound scan dropped earlier lines');
  }
  check(0 <= start && start <= end && end <= captured, '0 ≤ scanned start ≤ end ≤ captured');
  check(end - start >= snapshot.lines, 'each scanned line takes at least its newline');
  check(
    coverage.trailing_partial_dropped === end < captured,
    'the scan ends before the capture exactly when an unfinished last line was dropped',
  );
  check(
    (counts.precision === 'exact') ===
      (start === 0 &&
        !coverage.line_bound_reached &&
        !coverage.trailing_partial_dropped &&
        malformed === 0 &&
        snapshot.conflicted_ids === 0),
    'exact exactly when the scan is whole, complete and clean',
  );

  // Observed instants (`into_snapshot`).
  const { oldest_observed: oldest, newest_observed: newest } = coverage;
  check(
    (oldest === null) === (snapshot.distinct_ids === 0) &&
      (newest === null) === (snapshot.distinct_ids === 0),
    'oldest and newest observed are null exactly when no row is distinct',
  );
  if (oldest !== null && newest !== null) {
    check(at(oldest) <= at(newest), 'oldest observed is not after newest');
    if (inWindow < snapshot.distinct_ids)
      check(!inside(oldest) || !inside(newest), 'a row outside the window was observed');
    else check(inside(oldest) && inside(newest), 'every observed row is in the window');
    if (inWindow === 0)
      check(!inside(oldest) && !inside(newest), 'no observed row is in an empty window');
    for (const fire of loaded.latest_fires)
      check(
        at(oldest) <= at(fire.fired_at) && at(fire.fired_at) <= at(newest),
        `fire ${fire.fire_id} lies between oldest and newest observed`,
      );
    const [first] = loaded.latest_fires;
    if (inside(newest))
      check(
        first !== undefined && at(first.fired_at) === at(newest),
        'an in-window newest is the first fire',
      );
  }

  // Returned fires (`into_snapshot`, presentation bound).
  const fires = loaded.latest_fires;
  check(fires.length === Math.min(MAX_ROWS, inWindow), 'fires = min(100, in-window rows)');
  check(loaded.fires_truncated === inWindow > MAX_ROWS, 'fires truncated exactly past 100 rows');
  check(new Set(fires.map((fire) => fire.fire_id)).size === fires.length, 'fire IDs are unique');
  for (const fire of fires) check(inside(fire.fired_at), `fire ${fire.fire_id} is in the window`);
  fires.slice(1).forEach((fire, index) => {
    const before = fires[index]!;
    check(
      at(before.fired_at) > at(fire.fired_at) ||
        (at(before.fired_at) === at(fire.fired_at) && before.fire_id > fire.fire_id),
      `fire ${fire.fire_id} follows a newer fire`,
    );
  });
  const fireModes = noModes();
  for (const fire of fires) fireModes[fire.mode.kind] += 1;
  for (const mode of MODES)
    check(
      loaded.fires_truncated
        ? fireModes[mode] <= counts.window_modes[mode]
        : fireModes[mode] === counts.window_modes[mode],
      `listed ${mode} fires agree with the window count`,
    );

  // Returned groups (`group`, presentation bound).
  const groups = loaded.observed_groups;
  const count = loaded.observed_group_count;
  check(groups.length === Math.min(MAX_GROUPS, count), 'groups = min(100, observed groups)');
  check(loaded.groups_truncated === count > MAX_GROUPS, 'groups truncated exactly past 100');
  check((count === 0) === (inWindow === 0), 'there are groups exactly when there are fires');
  check(count <= inWindow, 'every group has an in-window row');
  check(new Set(groups.map(key)).size === groups.length, 'group keys are unique');
  groups
    .slice(1)
    .forEach((group, index) =>
      check(groupOrder(groups[index]!, group) < 0, `group ${key(group)} follows a later group`),
    );
  for (const group of groups) {
    check(inside(group.latest_observed), `group ${key(group)} was observed in the window`);
    check(total(group.modes) > 0, `group ${key(group)} has a row`);
  }
  const groupModes = noModes();
  for (const group of groups) for (const mode of MODES) groupModes[mode] += group.modes[mode];
  for (const mode of MODES)
    check(
      loaded.groups_truncated
        ? groupModes[mode] <= counts.window_modes[mode]
        : groupModes[mode] === counts.window_modes[mode],
      `listed ${mode} group rows agree with the window count`,
    );
  if (loaded.groups_truncated)
    check(
      inWindow - total(groupModes) >= count - groups.length,
      'each group past the bound has a row',
    );

  // Groups against fires: the fires are the newest rows overall.
  const fromFires = groupsOf(fires);
  if (!loaded.fires_truncated) {
    check(
      JSON.stringify(
        groups.map((group) => [
          key(group),
          at(group.latest_observed),
          MODES.map((mode) => group.modes[mode]),
        ]),
      ) ===
        JSON.stringify(
          fromFires.map((group) => [
            key(group),
            at(group.latest_observed),
            MODES.map((mode) => group.modes[mode]),
          ]),
        ),
      'groups are exactly those of the fires',
    );
  } else {
    const listed = new Map(groups.map((group) => [key(group), group]));
    const lastLatest =
      groups.length > 0 ? at(groups[groups.length - 1]!.latest_observed) : Infinity;
    for (const derived of fromFires) {
      const group = listed.get(key(derived));
      if (!group) {
        // Only a tie with the last listed group can push a fire's group out.
        check(
          loaded.groups_truncated && at(derived.latest_observed) === lastLatest,
          `the group of listed fires ${key(derived)} is listed`,
        );
        continue;
      }
      check(
        at(group.latest_observed) === at(derived.latest_observed),
        `group ${key(group)} was last observed at its newest fire`,
      );
      for (const mode of MODES)
        check(
          group.modes[mode] >= derived.modes[mode],
          `group ${key(group)} counts its listed ${mode} fires`,
        );
    }
  }
  return violations;
}

/** Every loaded answer the synthetic module exports, found by enumeration. */
const loadedExports = Object.entries(synthetic).flatMap(([name, value]) => {
  const answer: unknown =
    typeof value === 'function' ? (value.length === 0 ? (value as () => unknown)() : null) : value;
  return typeof answer === 'object' &&
    answer !== null &&
    (answer as { state?: unknown }).state === 'loaded'
    ? [[name, answer as Loaded] as const]
    : [];
});

describe('synthetic rule activity answers', () => {
  it('are every one an answer a read under default limits could return', () => {
    expect(loadedExports.map(([name]) => name)).toEqual(
      expect.arrayContaining([
        'loaded',
        'lowerBound',
        'emptyExact',
        'emptyLowerBound',
        'truncated',
        'lowerBoundWide',
        'pastCap',
        'identities',
        'identitiesRefreshed',
      ]),
    );
    // Every export's violations at once, so one bad fixture hides no other.
    expect(
      Object.fromEntries(
        loadedExports.map(([name, answer]) => [name, coherenceViolations(answer)]),
      ),
    ).toEqual(Object.fromEntries(loadedExports.map(([name]) => [name, []])));
  });

  it('rejects a byte bound on a ledger under 4 MiB', () => {
    const answer = synthetic.loaded();
    answer.coverage.byte_bound_reached = true;
    expect(coherenceViolations(answer)).toContain(
      'the byte bound is reached exactly when the ledger is over 4 MiB',
    );
  });

  it('rejects a line bound under 10,000 lines', () => {
    const answer = synthetic.emptyExact();
    answer.counts.precision = 'lower_bound';
    answer.coverage.line_bound_reached = true;
    expect(coherenceViolations(answer)).toEqual(
      expect.arrayContaining([
        'a line-bound scan kept exactly 10,000 lines',
        'a line-bound scan dropped earlier lines',
      ]),
    );
  });

  it('rejects line counts that do not add up', () => {
    const answer = synthetic.loaded();
    answer.counts.precision = 'lower_bound';
    answer.counts.snapshot.malformed.invalid_json = 2;
    answer.counts.snapshot.conflicted_ids = 1;
    answer.counts.snapshot.conflicted_rows = 2;
    expect(coherenceViolations(answer)).toEqual([
      'lines = blank + malformed + valid rows',
      'valid rows = distinct + duplicate + conflicted rows',
    ]);
  });
});
