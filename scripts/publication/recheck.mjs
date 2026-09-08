import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { scanRepository } from '../security/scan-secrets.mjs';
import { githubApi, pages } from './api.mjs';
import { readPublicContent, requireDisclosure } from './content.mjs';
import { withCandidateSource } from './source.mjs';
import {
  prNumber,
  PublicationError,
  queueMembers,
  readQueue,
  repositoryName,
  requireValue,
  sha,
} from './metadata.mjs';

const events = new Set([
  'pull_request_target',
  'issues',
  'issue_comment',
  'schedule',
  'workflow_run',
  'workflow_dispatch',
]);

export async function scanText(repo, texts, source, sourceOptions) {
  const temporary = await mkdtemp(join(tmpdir(), 'publication-content-'));
  try {
    const content = [];
    for (const [index, text] of texts.entries()) {
      const path = join(temporary, String(index) + '.txt');
      await writeFile(path, text, { mode: 0o600 });
      content.push(path);
    }
    await withCandidateSource(
      repo,
      source,
      async ({ base, head }) => {
        requireValue(
          (await scanRepository({ repo, diffs: [`${base}..${head}`], content })).length === 0,
          'Detected secret in candidate source or publication content; remove it and repeat disclosure review.',
        );
      },
      sourceOptions,
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

// This entrypoint only reads trusted default-branch code. PR data is never
// checked out or executed; checks:write is used only for this named context.
export async function recheckPublication({
  api,
  repository,
  repo,
  eventName,
  event,
  scan = scanText,
}) {
  repositoryName(repository);
  requireValue(
    events.has(eventName) &&
      (event.repository?.full_name === repository || eventName === 'schedule'),
    'Unsupported publication content event.',
  );
  const path = '/repos/' + repository;
  if (eventName === 'workflow_run') {
    prNumber(event.workflow_run?.id);
    const run = await api(path + '/actions/runs/' + event.workflow_run.id);
    const workflow = await api(path + '/actions/workflows/publication.yml');
    requireValue(
      run.repository?.full_name === repository &&
        run.workflow_id === workflow.id &&
        run.status === 'completed',
      'Publication signal does not belong to the expected repository workflow.',
    );
  }
  // Invalidate existing queue results before any fallible sequential source scan.
  // Later metadata/read failures then leave a blocking pending check in place.
  const metadata = await api(path);
  requireValue(typeof metadata.default_branch === 'string', 'Default branch is unavailable.');
  const base = sha(
    (await api(path + '/commits/' + encodeURIComponent(metadata.default_branch))).sha,
  );
  const queue = await readQueue(api, repository, metadata.default_branch, { allowMissing: true });
  const queuePending = [];
  for (const entry of queue) {
    const head = sha(entry.headCommit?.oid);
    const check = await api(path + '/check-runs', {
      name: 'publication-content',
      head_sha: head,
      status: 'in_progress',
      output: {
        title: 'Queue disclosure review pending',
        summary: 'Checking current source PR disclosure snapshots.',
      },
    });
    requireValue(Number.isSafeInteger(check.id), 'GitHub did not create the queue content check.');
    queuePending.push({ head, check: check.id });
  }
  const pulls = await pages(api, path + '/pulls?state=open');
  const pending = [];
  const seen = new Set();
  for (const pr of pulls) {
    prNumber(pr.number);
    sha(pr.head?.sha);
    requireValue(pr.state === 'open' && !seen.has(pr.number), 'Open PR pagination is incomplete.');
    seen.add(pr.number);
    const check = await api(path + '/check-runs', {
      name: 'publication-content',
      head_sha: pr.head.sha,
      status: 'in_progress',
      output: {
        title: 'Disclosure review pending',
        summary: 'Checking the current public content snapshot.',
      },
    });
    requireValue(Number.isSafeInteger(check.id), 'GitHub did not create the content check.');
    pending.push({ number: pr.number, head: pr.head.sha, check: check.id });
  }
  let failures = 0;
  const outcomes = new Map();
  for (const member of pending) {
    let conclusion = 'failure';
    let verifiedDigest;
    let summary = 'Content check could not complete; publication remains blocked.';
    try {
      const before = await readPublicContent(api, repository, member.number);
      requireValue(before.pr.head.sha === member.head, 'Source PR changed; rerun content checks.');
      requireDisclosure(before);
      await scan(repo, before.texts, {
        repository,
        number: member.number,
        head: member.head,
        base: before.pr.base.sha,
      });
      const after = await readPublicContent(api, repository, member.number);
      requireDisclosure(after);
      requireValue(
        after.pr.head.sha === member.head && after.digest === before.digest,
        'Public content changed during the check; repeat disclosure review.',
      );
      // Read the exact current head immediately before writing a successful result.
      const current = await api(path + '/pulls/' + member.number);
      requireValue(
        current.state === 'open' &&
          current.head?.sha === member.head &&
          current.body === after.pr.body,
        'Source PR changed before the result was written; rerun content checks.',
      );
      conclusion = 'success';
      verifiedDigest = before.digest;
      summary =
        'Trusted scanning found no detected secret in candidate source or current disclosure content. The snapshot is current; semantic review remains the reviewer’s responsibility.';
    } catch (error) {
      failures++;
      if (error instanceof PublicationError) summary = error.message;
    }
    await api(
      path + '/check-runs/' + member.check,
      {
        status: 'completed',
        conclusion,
        output: {
          title:
            conclusion === 'success' ? 'Disclosure snapshot current' : 'Disclosure review required',
          summary,
        },
      },
      'PATCH',
    );
    outcomes.set(member.number, { head: member.head, conclusion, digest: verifiedDigest });
  }
  for (const { head, check } of queuePending) {
    let conclusion = 'failure';
    try {
      const members = queueMembers({ base_sha: base, head_sha: head }, queue);
      requireValue(
        members.every(
          (member) =>
            outcomes.get(member.number)?.head === member.head &&
            outcomes.get(member.number)?.conclusion === 'success',
        ),
        'Queue member disclosure review has not passed.',
      );
      const currentQueue = await readQueue(api, repository, metadata.default_branch);
      requireValue(
        JSON.stringify(currentQueue) === JSON.stringify(queue) &&
          (await api(path + '/commits/' + encodeURIComponent(metadata.default_branch))).sha ===
            base,
        'Merge queue changed during disclosure checks.',
      );
      // Refresh source content again immediately before certifying a queue head.
      for (const member of members) {
        const current = await readPublicContent(api, repository, member.number);
        requireDisclosure(current);
        requireValue(
          current.pr.head.sha === member.head &&
            current.digest === outcomes.get(member.number)?.digest,
          'Queue source content changed after scanning.',
        );
      }
      conclusion = 'success';
    } catch {
      failures++;
    }
    await api(
      path + '/check-runs/' + check,
      {
        status: 'completed',
        conclusion,
        output: {
          title: 'Queue disclosure review',
          summary:
            conclusion === 'success'
              ? 'All current source PR disclosure snapshots passed.'
              : 'Queue metadata or a source PR disclosure snapshot is stale or incomplete.',
        },
      },
      'PATCH',
    );
  }
  return { checked: pending.length + queue.length, failures };
}

async function main() {
  try {
    requireValue(
      process.env.GITHUB_ACTIONS === 'true',
      'Content check writes are only available in the trusted workflow.',
    );
    const repository = repositoryName(process.env.GITHUB_REPOSITORY);
    const api = githubApi(process.env.GITHUB_TOKEN);
    const event = JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, 'utf8'));
    const metadata = await api('/repos/' + repository);
    requireValue(typeof metadata.default_branch === 'string', 'Default branch is unavailable.');
    const expected = await api(
      '/repos/' + repository + '/commits/' + encodeURIComponent(metadata.default_branch),
    );
    const repo = resolve(process.cwd());
    const actual = execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd: repo,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    requireValue(
      actual === sha(expected.sha),
      'Content checks must execute the current trusted default branch.',
    );
    const result = await recheckPublication({
      api,
      repository,
      repo,
      eventName: process.env.GITHUB_EVENT_NAME,
      event,
    });
    console.log(
      'Content checks completed for ' +
        result.checked +
        ' PR(s); blocked: ' +
        result.failures +
        '.',
    );
    process.exitCode = result.failures ? 1 : 0;
  } catch (error) {
    console.error(
      error instanceof PublicationError
        ? error.message
        : 'Content check failed while reading or updating required metadata.',
    );
    process.exitCode = 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
