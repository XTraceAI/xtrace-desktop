import { execFileSync } from 'node:child_process';
import { appendFile, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { scanRepository } from '../security/scan-secrets.mjs';
import { githubApi, pages } from './api.mjs';
import { readPublicContent } from './content.mjs';
import { withCandidateSource } from './source.mjs';
import {
  prNumber,
  PublicationError,
  readQueue,
  repositoryName,
  requireValue,
  sha,
} from './metadata.mjs';

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
export async function preparePublication({ api, repository, eventName, event, runId }) {
  repositoryName(repository);
  prNumber(runId);
  requireValue(
    eventName === 'workflow_dispatch' && event.repository?.full_name === repository,
    'Publication audits require an explicit workflow dispatch for this repository.',
  );
  const path = '/repos/' + repository;
  // Write terminal failures before scheduling scans. Cancellation, setup failure or
  // timeout cannot strand an in-progress check or preserve a previous success.
  const metadata = await api(path);
  requireValue(typeof metadata.default_branch === 'string', 'Default branch is unavailable.');
  const queue = await readQueue(api, repository, metadata.default_branch, { allowMissing: true });
  const queuePending = [];
  for (const entry of queue) {
    const head = sha(entry.headCommit?.oid);
    const check = await api(path + '/check-runs', {
      name: 'publication-content-advisory',
      head_sha: head,
      status: 'completed',
      conclusion: 'failure',
      external_id: 'publication-content-advisory:' + runId,
      output: {
        title: 'Queue publication checks are not activated',
        summary:
          'Trusted combined-tree scanning and live queue acceptance are required before queue activation.',
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
      name: 'publication-content-advisory',
      head_sha: pr.head.sha,
      status: 'completed',
      conclusion: 'failure',
      external_id: 'publication-content-advisory:' + runId,
      output: {
        title: 'Disclosure scan has not completed',
        summary:
          'Advisory scan incomplete. Retry incomplete runs; a maintainer must perform current disclosure review.',
      },
    });
    requireValue(Number.isSafeInteger(check.id), 'GitHub did not create the content check.');
    pending.push({ number: pr.number, head: pr.head.sha, check: check.id });
  }
  requireValue(
    pending.length <= 256,
    'More than 256 open PRs exceed the supported scan matrix; all created checks remain failed.',
  );
  return { pending, queue: queuePending };
}

export async function checkPublication({
  api,
  repository,
  repo,
  member,
  runId,
  trustedHead,
  scan = scanText,
}) {
  repositoryName(repository);
  prNumber(runId);
  sha(trustedHead);
  prNumber(member?.number);
  sha(member?.head);
  prNumber(member?.check);
  const path = '/repos/' + repository;
  const check = await api(path + '/check-runs/' + member.check);
  requireValue(
    check.name === 'publication-content-advisory' &&
      check.head_sha === member.head &&
      check.external_id === 'publication-content-advisory:' + runId &&
      check.status === 'completed' &&
      check.conclusion === 'failure',
    'Content check does not belong to this run and source head.',
  );
  let conclusion = 'failure';
  let summary = 'Advisory scan could not complete; maintainer disclosure review is required.';
  try {
    const before = await readPublicContent(api, repository, member.number);
    requireValue(before.pr.head.sha === member.head, 'Source PR changed; rerun content checks.');
    await scan(repo, before.texts, {
      repository,
      number: member.number,
      head: member.head,
      base: before.pr.base.sha,
      retainedHeads: before.content.headRevisions.heads,
    });
    const after = await readPublicContent(api, repository, member.number);
    requireValue(
      after.pr.head.sha === member.head &&
        after.pr.base.sha === before.pr.base.sha &&
        after.digest === before.digest,
      'Public content changed during the check; repeat disclosure review.',
    );
    const metadata = await api(path);
    requireValue(typeof metadata.default_branch === 'string', 'Default branch is unavailable.');
    const trusted = await api(path + '/commits/' + encodeURIComponent(metadata.default_branch));
    requireValue(
      trusted.sha === trustedHead,
      'Trusted default-branch code changed during scanning; rerun checks.',
    );
    // Refresh the complete disclosure after the trusted-code requests, with no
    // intervening API work before writing the result. GitHub offers no atomic
    // content-read/check-write operation; later edits still need reconciliation.
    const current = await readPublicContent(api, repository, member.number);
    requireValue(
      current.pr.head.sha === member.head &&
        current.pr.base.sha === before.pr.base.sha &&
        current.digest === before.digest,
      'Public content changed before the result was written; repeat disclosure review.',
    );
    conclusion = 'success';
    summary =
      'Advisory scan found no detected secret in the observed source and public text. This check is not merge authorization; attachments and semantics still need review before publication.';
  } catch (error) {
    if (error instanceof PublicationError) summary = error.message;
  }
  await api(
    path + '/check-runs/' + member.check,
    {
      status: 'completed',
      conclusion,
      output: {
        title:
          conclusion === 'success'
            ? 'Advisory publication scan complete'
            : 'Publication scan incomplete',
        summary,
      },
    },
    'PATCH',
  );
  return { checked: 1, failures: conclusion === 'failure' ? 1 : 0 };
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
    const runId = prNumber(Number(process.env.GITHUB_RUN_ID));
    const mode = process.argv[2];
    requireValue(
      ['prepare', 'check'].includes(mode) && process.argv.length === 3,
      'Choose a supported publication content phase.',
    );
    if (mode === 'prepare') {
      const result = await preparePublication({
        api,
        repository,
        runId,
        eventName: process.env.GITHUB_EVENT_NAME,
        event,
      });
      requireValue(Boolean(process.env.GITHUB_OUTPUT), 'Workflow output is unavailable.');
      await appendFile(
        process.env.GITHUB_OUTPUT,
        'matrix=' +
          JSON.stringify({ include: result.pending }) +
          '\n' +
          'count=' +
          result.pending.length +
          '\n' +
          'trusted=' +
          actual +
          '\n',
      );
      console.log('Scheduled ' + result.pending.length + ' independent content scans.');
    } else {
      const result = await checkPublication({
        api,
        repository,
        repo,
        runId,
        member: JSON.parse(process.env.PUBLICATION_MEMBER),
        trustedHead: actual,
      });
      console.log('Advisory scan completed; failures: ' + result.failures + '.');
      process.exitCode = result.failures ? 1 : 0;
    }
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
