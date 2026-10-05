import type { RuleActivityFire } from '../../data/generated/RuleActivityFire';
import type { RuleActivityGroup } from '../../data/generated/RuleActivityGroup';
import type { RuleActivityResult } from '../../data/generated/RuleActivityResult';
import type { Loaded } from './rule-activity';

/**
 * Test-only synthetic rule activity answers: not a real ledger and not a
 * design sample. Shaped the way `rule_activity_read` shapes them: fires newest
 * first, groups latest observed first, byte offsets as decimal strings, and
 * the read ID the native side echoes (a fixture need not). A group's latest
 * observed instant is its newest fire's, so `group(n)` pairs with `fire(n)`.
 * Every loaded answer here must pass `coherenceViolations` in
 * `rule-activity.synthetic.test.ts`, which holds the reader's rules.
 */
export const WINDOW = {
  read_at: '2026-09-23T09:00:00Z',
  window_start: '2026-09-09T09:00:00Z',
  window_end: '2026-09-23T09:00:00Z',
} as const;

export const fire = (index: number, facts: Partial<RuleActivityFire> = {}): RuleActivityFire => ({
  fire_id: `fire-${index}`,
  rule_id: `synthetic-rule-${index}`,
  rule_version: { kind: 'number', value: String(1 + (index % 2)) },
  rulebook_id: 'synthetic-book',
  // One minute apart, newest first.
  fired_at: new Date(Date.parse('2026-09-22T12:00:00Z') - index * 60_000).toISOString(),
  tool: 'Bash',
  hook_phase: 'pre_tool_use',
  host: 'claude',
  session_id: `synthetic-session-${index}`,
  mode: { kind: 'advise' },
  ...facts,
});

export const group = (
  index: number,
  facts: Partial<RuleActivityGroup> = {},
): RuleActivityGroup => ({
  rulebook_id: 'synthetic-book',
  rule_id: `synthetic-rule-${index}`,
  latest_observed: new Date(Date.parse('2026-09-22T12:00:00Z') - index * 60_000).toISOString(),
  modes: { advise: 1, gate: 0, suppressed: 0, unrecognized: 0 },
  ...facts,
});

/**
 * A complete, clean scan: exact counts. One of its seven rows was recorded
 * before the window, so it is the oldest observed but not a fire.
 */
export const loaded = (facts: Partial<Loaded> = {}): Loaded => ({
  state: 'loaded',
  read_id: 'synthetic-read',
  source: 'default_local_rulebook',
  schema_version: 2,
  ...WINDOW,
  counts: {
    precision: 'exact',
    snapshot: {
      lines: 7,
      blank_lines: 0,
      valid_rows: 7,
      distinct_ids: 7,
      duplicate_rows: 0,
      conflicted_ids: 0,
      conflicted_rows: 0,
      malformed: {
        oversize_line: 0,
        invalid_json: 0,
        invalid_shape: 0,
        invalid_timestamp: 0,
        oversize_value: 0,
      },
    },
    window_modes: { advise: 3, gate: 1, suppressed: 1, unrecognized: 1 },
  },
  coverage: {
    captured_len: '4096',
    scanned_start: '0',
    scanned_end: '4096',
    byte_bound_reached: false,
    line_bound_reached: false,
    leading_partial_dropped: false,
    trailing_partial_dropped: false,
    grew_after_capture: false,
    oldest_observed: '2026-09-08T12:00:00.000Z',
    newest_observed: '2026-09-22T12:00:00.000Z',
  },
  latest_fires: [
    fire(0, { rule_id: 'synthetic-no-force-push' }),
    fire(1, { rule_id: 'synthetic-no-force-push', mode: { kind: 'gate' } }),
    fire(2, {
      rule_id: 'synthetic-unscoped',
      rulebook_id: null,
      rule_version: { kind: 'label', value: 'draft' },
      mode: { kind: 'suppressed' },
      tool: null,
      hook_phase: null,
      host: null,
    }),
    fire(3, { rule_id: 'synthetic-no-force-push' }),
    fire(4, {
      rule_id: 'synthetic-odd-mode',
      rule_version: null,
      mode: { kind: 'unrecognized', value: 'shadow' },
    }),
    fire(5, { rule_id: '<b>synthetic-markup</b>', tool: '<i>tool</i>' }),
  ],
  fires_truncated: false,
  observed_groups: [
    group(0, {
      rule_id: 'synthetic-no-force-push',
      modes: { advise: 2, gate: 1, suppressed: 0, unrecognized: 0 },
    }),
    group(2, {
      rule_id: 'synthetic-unscoped',
      rulebook_id: null,
      modes: { advise: 0, gate: 0, suppressed: 1, unrecognized: 0 },
    }),
    group(4, {
      rule_id: 'synthetic-odd-mode',
      modes: { advise: 0, gate: 0, suppressed: 0, unrecognized: 1 },
    }),
    group(5, { rule_id: '<b>synthetic-markup</b>' }),
  ],
  observed_group_count: 4,
  groups_truncated: false,
  ...facts,
});

/**
 * A whole small ledger that was still being written: its unfinished last line
 * was skipped, and among its 11 complete lines were 2 malformed lines and 2
 * rows of one fire ID recorded with differing details.
 */
export const lowerBound = (): Loaded => {
  const base = loaded();
  return {
    ...base,
    counts: {
      ...base.counts,
      precision: 'lower_bound',
      snapshot: {
        ...base.counts.snapshot,
        lines: 11,
        valid_rows: 9,
        conflicted_ids: 1,
        conflicted_rows: 2,
        malformed: { ...base.counts.snapshot.malformed, invalid_json: 2 },
      },
    },
    coverage: {
      ...base.coverage,
      scanned_end: '3968',
      trailing_partial_dropped: true,
      grew_after_capture: true,
    },
  };
};

const emptyModes = { advise: 0, gate: 0, suppressed: 0, unrecognized: 0 };
/** A clean scan whose seven rows were all recorded before the window. */
export const emptyExact = (): Loaded => {
  const base = loaded();
  return {
    ...base,
    counts: { ...base.counts, window_modes: emptyModes },
    coverage: {
      ...base.coverage,
      oldest_observed: '2026-09-08T11:54:00.000Z',
      newest_observed: '2026-09-08T12:00:00.000Z',
    },
    latest_fires: [],
    observed_groups: [],
    observed_group_count: 0,
  };
};
/**
 * An incomplete scan that found nothing in the part it read: one line's
 * timestamp could not be read, so that row might have been in the window.
 */
export const emptyLowerBound = (): Loaded => {
  const base = emptyExact();
  return {
    ...base,
    counts: {
      ...base.counts,
      precision: 'lower_bound',
      snapshot: {
        ...base.counts.snapshot,
        lines: 8,
        malformed: { ...base.counts.snapshot.malformed, invalid_timestamp: 1 },
      },
    },
  };
};

/**
 * More rows and groups than the presentation bound of 100 each. The newest 100
 * rows are one advise fire each of 100 rules, so those rules are the latest
 * observed groups. The 125 older rows are 80 advise and 5 suppressed rows of
 * the first rule and one gate row of each of 40 further rules, whose groups
 * are all older than the bound.
 */
export const truncated = (): Loaded => {
  const base = loaded();
  const rows = 225;
  return {
    ...base,
    counts: {
      ...base.counts,
      snapshot: { ...base.counts.snapshot, lines: rows, valid_rows: rows, distinct_ids: rows },
      window_modes: { advise: 180, gate: 40, suppressed: 5, unrecognized: 0 },
    },
    coverage: {
      ...base.coverage,
      captured_len: '65536',
      scanned_end: '65536',
      // Every row is in the window, so the oldest observed is the oldest fire.
      oldest_observed: new Date(
        Date.parse('2026-09-22T12:00:00Z') - (rows - 1) * 60_000,
      ).toISOString(),
    },
    latest_fires: Array.from({ length: 100 }, (_, index) => fire(index)),
    fires_truncated: true,
    observed_groups: Array.from({ length: 100 }, (_, index) =>
      index === 0
        ? group(0, { modes: { advise: 81, gate: 0, suppressed: 5, unrecognized: 0 } })
        : group(index),
    ),
    observed_group_count: 140,
    groups_truncated: true,
  };
};

/**
 * A scan that stopped at the reader's line bound of 10,000 with every line an
 * advise row of one rule: the largest count a group can show, as a lower bound.
 */
export const lowerBoundWide = (): Loaded => {
  const base = truncated();
  const rows = 10_000;
  return {
    ...base,
    counts: {
      ...base.counts,
      precision: 'lower_bound',
      snapshot: { ...base.counts.snapshot, lines: rows, valid_rows: rows, distinct_ids: rows },
      window_modes: { advise: rows, gate: 0, suppressed: 0, unrecognized: 0 },
    },
    coverage: {
      ...base.coverage,
      captured_len: '3145728',
      scanned_start: '131072',
      scanned_end: '3145728',
      line_bound_reached: true,
      oldest_observed: '2026-09-15T13:21:00.000Z',
    },
    latest_fires: base.latest_fires.map((row) => ({ ...row, rule_id: 'synthetic-rule-0' })),
    observed_groups: [
      group(0, { modes: { advise: rows, gate: 0, suppressed: 0, unrecognized: 0 } }),
    ],
    observed_group_count: 1,
    groups_truncated: false,
  };
};

/**
 * The truncated answer at the group bound's edge: the oldest listed fire ties
 * in time with an older, unlisted gate row of `synthetic-rule-100`, whose
 * group sorts first in the tie and takes the last returned place. The listed
 * fire's own group, `synthetic-rule-99`, is the 101st and not returned.
 */
export const pastCap = (): Loaded => {
  const base = truncated();
  const tie = base.latest_fires[99]!.fired_at;
  return {
    ...base,
    observed_groups: [
      ...base.observed_groups.slice(0, 99),
      group(100, {
        latest_observed: tie,
        modes: { advise: 0, gate: 1, suppressed: 0, unrecognized: 0 },
      }),
    ],
  };
};

/** A UUID-shaped synthetic identifier, like those real ledgers record; never a real one. */
const uuid = (kind: string, index: number) =>
  `${kind.repeat(8)}-0000-4000-8000-${index.toString(16).padStart(12, '0')}`;
export const BOOK_A = uuid('b', 0xa);
export const BOOK_B = uuid('b', 0xb);
/**
 * Eight observed pairs with opaque IDs. The last reuses the first's rule ID
 * with no rulebook, so it is a group of its own.
 */
export const IDENTITY_PAIRS = [
  ...Array.from({ length: 7 }, (_, index) => ({
    rulebook_id: index < 4 ? BOOK_A : BOOK_B,
    rule_id: uuid('a', index),
  })),
  { rulebook_id: null, rule_id: uuid('a', 0) },
] as const;

/**
 * A clean ledger of 321 lines whose 252 in-window rows fire the eight pairs in
 * turn, one minute apart, every fifth a gate; its 69 older rows are before the
 * window. `arrived` newer advise rows of the last pair came after, so a read
 * then puts that group first and every other one a place later, with its own
 * counts unchanged. Fire IDs stay with their rows across reads.
 */
function identityLedger(arrived: number): Loaded {
  const base = loaded();
  const newest = Date.parse('2026-09-22T12:00:00Z');
  const rows: RuleActivityFire[] = [
    ...Array.from({ length: arrived }, (_, index) =>
      fire(0, {
        ...IDENTITY_PAIRS[7],
        fire_id: uuid('f', 0x1000 + arrived - index),
        fired_at: new Date(newest + (arrived - index) * 60_000).toISOString(),
      }),
    ),
    ...Array.from({ length: 252 }, (_, index) =>
      fire(index, {
        ...IDENTITY_PAIRS[index % 8],
        fire_id: uuid('f', 252 - index),
        mode: { kind: index % 5 === 0 ? 'gate' : 'advise' },
      }),
    ),
  ];
  const groups = new Map<string, RuleActivityGroup>();
  for (const row of rows) {
    const key = JSON.stringify([row.rulebook_id, row.rule_id]);
    const seen =
      groups.get(key) ??
      group(0, {
        rulebook_id: row.rulebook_id,
        rule_id: row.rule_id,
        latest_observed: row.fired_at,
        modes: { advise: 0, gate: 0, suppressed: 0, unrecognized: 0 },
      });
    seen.modes[row.mode.kind] += 1;
    groups.set(key, seen);
  }
  const lines = rows.length + 69;
  const gate = rows.filter((row) => row.mode.kind === 'gate').length;
  return {
    ...base,
    counts: {
      ...base.counts,
      snapshot: { ...base.counts.snapshot, lines, valid_rows: lines, distinct_ids: lines },
      window_modes: { advise: rows.length - gate, gate, suppressed: 0, unrecognized: 0 },
    },
    coverage: {
      ...base.coverage,
      captured_len: '98304',
      scanned_end: '98304',
      newest_observed: rows[0]!.fired_at,
    },
    latest_fires: rows.slice(0, 100),
    fires_truncated: true,
    // Rows are newest first, so first-seen order is latest-observed order.
    observed_groups: [...groups.values()],
    observed_group_count: groups.size,
  };
}
export const identities = () => identityLedger(0);
export const identitiesRefreshed = () => identityLedger(1);

export const unavailable: RuleActivityResult = {
  state: 'unavailable',
  source: 'default_local_rulebook',
  part: 'ledger',
  reason: 'missing',
  found_version: null,
};
export const unsupportedSchema: RuleActivityResult = {
  state: 'unavailable',
  source: 'default_local_rulebook',
  part: 'schema_marker',
  reason: 'unsupported_schema',
  found_version: '9',
};
export const sourceChanged: RuleActivityResult = {
  state: 'source_changed',
  source: 'default_local_rulebook',
  change: 'shrunk',
};
export const deadline: RuleActivityResult = { state: 'interrupted', reason: 'deadline' };
export const interruptedCancelled: RuleActivityResult = {
  state: 'interrupted',
  reason: 'cancelled',
};
export const busy: RuleActivityResult = { state: 'busy', reason: 'another_read' };
export const closed: RuleActivityResult = { state: 'closed' };
export const failed: RuleActivityResult = { state: 'failed' };
export const invalidReadId: RuleActivityResult = { state: 'invalid_read_id' };
