import { expect, it } from 'vitest';
import type { PrAttemptOutcome } from '../../data/generated/PrAttemptOutcome';
import type { PrRefreshReport } from '../../data/generated/PrRefreshReport';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import { prRow } from '../prs.synthetic';
import { notFoundOnGitHub, outcomeText, refreshReportText, rowStatusText } from './pr-effort';

const window: DashboardWindow = {
  days: 7,
  start_ms: Date.UTC(2026, 8, 1),
  end_ms: Date.UTC(2026, 8, 8),
  timezone: 'UTC',
  clock: 'fixture',
};

const saved = { persistence: 'recorded', write: 'applied' } as const;
const row = (id: number, outcome: PrAttemptOutcome) => ({
  id,
  pull_request: {
    id,
    repository: 'xtrace/app',
    number: id,
    url: `https://github.com/xtrace/app/pull/${id}`,
  },
  outcome,
});
const report = (outcomes: PrAttemptOutcome[]): PrRefreshReport => {
  const count = (kind: PrAttemptOutcome['outcome']) =>
    outcomes.filter((outcome) => outcome.outcome === kind).length;
  return {
    requested: outcomes.length,
    attempted: count('succeeded') + count('failed'),
    succeeded: count('succeeded'),
    failed: count('failed'),
    skipped: count('skipped'),
    unrecorded: 0,
    cancelled: false,
    committed: true,
    rows: outcomes.map((outcome, index) => row(index + 1, outcome)),
  };
};

it('says a missing pull request was not found on GitHub, not that the check failed', () => {
  const missing: PrAttemptOutcome = { outcome: 'failed', error: 'not_found', persistence: saved };
  expect(outcomeText(missing)).toBe('Not found on GitHub · saved');
  expect(
    refreshReportText(
      report([
        { outcome: 'succeeded', persistence: saved },
        missing,
        { outcome: 'failed', error: 'execution_failed', persistence: saved },
      ]),
    ),
  ).toBe(
    'Requested 3: 1 checked, 1 could not be checked, 1 not found on GitHub, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
});

it('keeps the usual wording when nothing was missing', () => {
  const failed: PrAttemptOutcome = {
    outcome: 'failed',
    error: 'execution_failed',
    persistence: saved,
  };
  expect(outcomeText(failed)).toBe(
    'Could not be checked: gh pr view failed; earlier facts are kept · saved',
  );
  expect(refreshReportText(report([failed]))).toBe(
    'Requested 1: 0 checked, 1 could not be checked, 0 skipped. Stored facts changed; the Dashboard reads them again.',
  );
});

it('words a not-found answer by whether GitHub confirmed the pull request before', () => {
  const missing: PrAttemptOutcome = { outcome: 'failed', error: 'not_found', persistence: saved };
  const never = prRow(1, { last_attempted_at_ms: 1, status: { status: 'not_found_on_github' } });
  const confirmed = prRow(2, {
    refreshed_at_ms: 1,
    last_attempted_at_ms: 2,
    status: { status: 'failed_after_refresh', error: 'not_found' },
  });
  expect(outcomeText(missing, never)).toBe(
    'Not found on GitHub; not counted as a pull request · saved',
  );
  expect(outcomeText(missing, confirmed)).toBe('Not found on GitHub; earlier facts kept · saved');
  // Before the list is read again the row still says "not checked yet": it
  // had no facts to keep either way.
  expect(outcomeText(missing, prRow(3))).toBe(
    'Not found on GitHub; not counted as a pull request · saved',
  );
  expect(notFoundOnGitHub(never)).toBe(true);
  expect(notFoundOnGitHub(confirmed)).toBe(false);
  expect(rowStatusText(never, window)).toMatch(
    /^not found on GitHub \(checked .+\); not counted as a pull request$/,
  );
  expect(rowStatusText(confirmed, window)).toMatch(
    /^stale: not found on GitHub at the last check; facts from .+$/,
  );
});
