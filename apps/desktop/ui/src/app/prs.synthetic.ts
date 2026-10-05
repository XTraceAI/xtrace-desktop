import type { PrRow } from '../data/generated/PrRow';

/**
 * Test-only synthetic stored pull requests: not real history and not a design
 * sample. Rows are shaped the way `prs_list` shapes them: a link alone sets no
 * metadata, a failed refresh keeps the facts of an earlier success, and rows
 * arrive in repository then number order.
 */
export const prRow = (id: number, facts: Partial<PrRow> = {}): PrRow => ({
  pull_request: {
    id,
    repository: 'xtrace/app',
    number: 100 + id,
    url: `https://github.com/xtrace/app/pull/${100 + id}`,
  },
  linked_sessions: 1,
  title: null,
  state: null,
  merged_at: null,
  additions: null,
  deletions: null,
  head_ref_name: null,
  refreshed_at_ms: null,
  last_attempted_at_ms: null,
  status: { status: 'never_attempted' },
  ...facts,
});

const FACTS_AT = Date.parse('2026-09-06T00:00:00Z');
const TRIED_AT = Date.parse('2026-09-07T00:00:00Z');

/**
 * One row for each thing the list has to tell apart: the four refresh
 * statuses, every cached state, a cached zero beside an unknown size,
 * lockfile-sized counts (five and seven digits), and text long enough to need
 * its ellipsis.
 */
export const PR_CACHE_STATES: PrRow[] = [
  prRow(1),
  prRow(2, {
    title: 'docs: explain the cached pull-request inventory and what it does not measure yet',
    state: 'open',
    additions: 0,
    deletions: 0,
    head_ref_name: 'docs/cached-pull-request-inventory-explanation',
    linked_sessions: 12,
    refreshed_at_ms: FACTS_AT,
    last_attempted_at_ms: FACTS_AT,
    status: { status: 'refreshed' },
  }),
  prRow(3, {
    last_attempted_at_ms: TRIED_AT,
    status: { status: 'failed_never_refreshed', error: 'unauthorized' },
  }),
  prRow(4, {
    title: 'feat: kept',
    state: 'merged',
    merged_at: '2026-09-05T12:00:00Z',
    additions: 1_234_567,
    deletions: 987_654,
    head_ref_name: 'feat/kept',
    linked_sessions: 1_204,
    refreshed_at_ms: FACTS_AT,
    last_attempted_at_ms: TRIED_AT,
    status: { status: 'failed_after_refresh', error: 'rate_limited' },
  }),
  prRow(5, {
    title: 'fix: abandoned',
    state: 'closed',
    additions: 5,
    deletions: null,
    refreshed_at_ms: FACTS_AT,
    last_attempted_at_ms: FACTS_AT,
    status: { status: 'refreshed' },
  }),
  prRow(6, {
    title: 'feat: merged without a stored time',
    state: 'merged',
    merged_at: null,
    additions: 12_345,
    deletions: 67_890,
    refreshed_at_ms: FACTS_AT,
    last_attempted_at_ms: FACTS_AT,
    status: { status: 'refreshed' },
  }),
];

/**
 * A long cached list across two repositories, in the order storage lists
 * them; one row in fifty has a cached title.
 */
export const manyPrRows = (total: number): PrRow[] =>
  Array.from({ length: total }, (_, index) => {
    const repository = index < total / 2 ? 'octo-org/tools' : 'xtrace/app';
    const number = 1000 + index;
    return prRow(index + 1, {
      pull_request: {
        id: index + 1,
        repository,
        number,
        url: `https://github.com/${repository}/pull/${number}`,
      },
      title: index % 50 === 0 ? `feat: milestone ${index}` : null,
    });
  });
