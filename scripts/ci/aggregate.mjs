import { requireSuccessfulJobs } from './policy.mjs';

try {
  const results = JSON.parse(process.env.NEEDS_JSON);
  // Every job runs for each supported event. Hooks report explicit absence in a
  // successful job; no skipped dependency is silently promoted to success.
  requireSuccessfulJobs(results, ['policy', 'ui', 'security']);
  console.log('Every required CI job succeeded.');
} catch {
  console.error(
    'CI aggregate failed: a required job was missing, skipped, cancelled or unsuccessful.',
  );
  process.exitCode = 1;
}
