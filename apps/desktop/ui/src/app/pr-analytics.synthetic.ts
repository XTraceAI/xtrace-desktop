import synthetic from './prs-analytics.synthetic.json';
import type { PullRequestSessionsRequest } from '../data/DataSource';
import type { FixturePrSessions } from '../data/generated/FixturePrSessions';
import type { PrAnalyticsPage } from '../data/generated/PrAnalyticsPage';
import type { SessionPage } from '../data/generated/SessionPage';

/**
 * Test-only: the PRs page's reports and drilldown pages as the app's own Rust
 * commands assembled them over a seeded synthetic database
 * (`tests/pr_analytics.rs::ui_export`, byte-checked there). Not real history
 * and not a design sample. Nothing here computes a value: a lookup either
 * finds the exported answer to exactly this request or fails.
 */
export type SyntheticScenario = {
  pr_analytics: PrAnalyticsPage[];
  /** Every page, with the cursor that requested it (`null` for the first). */
  pr_sessions: (FixturePrSessions & { after: string | null })[];
};

// JSON imports widen literal unions; the export is the generated shape.
export const scenarios = synthetic as unknown as Record<'measured' | 'sparse', SyntheticScenario>;

export function syntheticReport(
  scenario: SyntheticScenario,
  days: number,
  confirmedOnly: boolean,
): PrAnalyticsPage {
  const page = scenario.pr_analytics.find(
    (entry) => entry.window.days === days && entry.report.confirmed_only === confirmedOnly,
  );
  if (!page) throw new Error(`no exported report for ${days}d confirmed=${confirmedOnly}`);
  return structuredClone(page);
}

export function syntheticSessions(
  scenario: SyntheticScenario,
  request: PullRequestSessionsRequest,
  after: string | null,
): SessionPage {
  const entry = scenario.pr_sessions.find(
    (candidate) =>
      candidate.repository === request.repository &&
      candidate.number === request.number &&
      candidate.confirmed_only === request.confirmedOnly &&
      candidate.window_days === request.windowDays &&
      candidate.window_end_ms === request.windowEndMs &&
      candidate.after === after,
  );
  if (!entry) throw new Error(`no exported page for ${JSON.stringify({ ...request, after })}`);
  return structuredClone(entry.page);
}
