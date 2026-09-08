import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Parser } from 'commonmark';
import { githubApi, pages } from '../publication/api.mjs';
import {
  linkedIssues,
  repositoryName,
  resolveEvent,
  sha,
  validatePullRequest,
} from '../publication/metadata.mjs';
import { CheckError, validateDco } from './policy.mjs';
import { checkpointEvidence, checkpointFix, requiredCheckpoints } from './checkpoints.mjs';

export function planSlot(body, branch, policy) {
  if (policy?.version !== 1 || !policy.slots || !policy.maintenance)
    throw new CheckError('Trusted stage policy is unavailable.');
  const tree = new Parser().parse(body);
  const controls = [];
  const sections = new Map();
  let section;
  for (let node = tree.firstChild; node; node = node.next) {
    if (node.type === 'heading' && node.level === 2) {
      section = node.firstChild?.literal;
      sections.set(section, false);
    }
    if (section && ['paragraph', 'list', 'code_block', 'block_quote'].includes(node.type)) {
      const text = [];
      const walker = node.walker();
      let event;
      while ((event = walker.next())) {
        if (event.entering && ['text', 'code', 'code_block'].includes(event.node.type))
          text.push(event.node.literal);
      }
      const value = text.join(' ').trim();
      if (
        value &&
        !/^(?:Describe |List commands |Follow |Plan slot: |Maintenance: |Disclosure snapshot: |\[[ xX]\] I reviewed the final PR text)/.test(
          value,
        )
      )
        sections.set(section, true);
    }
    if (node.type !== 'paragraph') continue;
    const literal =
      node.firstChild?.type === 'text' && !node.firstChild.next ? node.firstChild.literal : null;
    if (literal) {
      const control = /^(Plan slot|Maintenance): ([A-Za-z0-9-]+)$/.exec(literal);
      if (control) controls.push(control);
    }
  }
  if (controls.length !== 1 || !sections.get('Change') || !sections.get('Verification'))
    throw new CheckError(
      'PR needs one plan classification and substantive Change and Verification sections.',
    );
  const [, kind, id] = controls[0];
  const slot = kind === 'Plan slot' ? policy.slots[id] : policy.maintenance[id];
  if (
    !slot ||
    (kind === 'Plan slot'
      ? !branch.startsWith(`feat/${id.toLowerCase()}-`)
      : branch !== slot.branch)
  )
    throw new CheckError('Unknown plan classification or source branch does not match its slot.');
  requiredCheckpoints(slot.stage);
  return { id, stage: slot.stage };
}

export async function sourceCommits(api, repository, pr) {
  const commits = await pages(api, `/repos/${repository}/pulls/${pr.number}/commits`);
  if (
    !Number.isSafeInteger(pr.commits) ||
    pr.commits < 1 ||
    pr.commits > 250 ||
    commits.length !== pr.commits ||
    new Set(commits.map((item) => sha(item.sha))).size !== commits.length
  )
    throw new CheckError('Source PR commit pagination is incomplete.');
  return commits.map((item) => ({ ...item.commit, sha: item.sha }));
}

async function trustedPolicy(api, repository, configuredSha) {
  const repo = await api(`/repos/${repository}`);
  if (typeof repo.default_branch !== 'string')
    throw new CheckError('Default branch is unavailable.');
  const ref = configuredSha
    ? sha(configuredSha)
    : sha(
        (await api(`/repos/${repository}/commits/${encodeURIComponent(repo.default_branch)}`)).sha,
      );
  const file = await api(`/repos/${repository}/contents/scripts/ci/stages.json?ref=${ref}`);
  if (
    file.type !== 'file' ||
    file.encoding !== 'base64' ||
    file.size > 100_000 ||
    typeof file.content !== 'string'
  )
    throw new CheckError('Trusted stage policy is unavailable.');
  return { ref, policy: JSON.parse(Buffer.from(file.content, 'base64').toString('utf8')) };
}

export async function checkSourcePolicy({
  api,
  repository,
  eventName,
  event,
  policySha,
  checkpoints = {},
}) {
  repositoryName(repository);
  if (eventName === 'push') {
    if (
      event.repository?.full_name !== repository ||
      event.ref !== 'refs/heads/' + event.repository.default_branch
    )
      throw new CheckError('Unsupported push event.');
    const comparison = await api(
      `/repos/${repository}/compare/${sha(event.before)}...${sha(event.after)}`,
    );
    if (
      !Array.isArray(comparison.commits) ||
      comparison.total_commits < 1 ||
      comparison.total_commits > 250 ||
      comparison.commits.length !== comparison.total_commits
    )
      throw new CheckError('Push commit comparison is incomplete.');
    for (const commit of comparison.commits) validateDco({ ...commit.commit, sha: commit.sha });
    return comparison.commits.length;
  }
  const selection = await resolveEvent(eventName, event, repository, api);
  const { policy } = await trustedPolicy(api, repository, policySha);
  const snapshots = [];
  let count = 0;
  for (const member of selection.members) {
    const route = `/repos/${repository}/pulls/${member.number}`;
    const pr = validatePullRequest(await api(route), repository, member.head);
    const slot = planSlot(pr.body, pr.head.ref, policy);
    const commits = await sourceCommits(api, repository, pr);
    for (const commit of commits) validateDco(commit);
    count += commits.length;
    const evidenceSnapshots = [];
    for (const checkpoint of requiredCheckpoints(slot.stage)) {
      const approvers = checkpoints.approvers;
      const issueNumber = checkpoints.issues?.[checkpoint];
      if (
        !Array.isArray(approvers) ||
        !approvers.length ||
        !approvers.every((name) => typeof name === 'string' && /^[A-Za-z0-9-]+$/.test(name))
      )
        throw new CheckError('Checkpoint approvers have not been configured.');
      const prComments = await pages(api, `/repos/${repository}/issues/${pr.number}/comments`);
      if (checkpointFix(prComments, approvers, checkpoint, member.head)) {
        evidenceSnapshots.push({
          route: `/repos/${repository}/issues/${pr.number}/comments`,
          value: prComments,
          paged: true,
        });
        continue;
      }
      if (!Number.isSafeInteger(issueNumber) || issueNumber < 1)
        throw new CheckError('Required checkpoint issue has not been configured.');
      if (
        !linkedIssues(pr.body, repository).some(
          (link) =>
            link.repository.toLowerCase() === repository.toLowerCase() &&
            link.number === issueNumber,
        )
      )
        throw new CheckError(
          'PR must link its checkpoint issue so current-content review includes approval changes.',
        );
      const issueRoute = `/repos/${repository}/issues/${issueNumber}`;
      const issue = await api(issueRoute);
      const comments = await pages(api, issueRoute + '/comments');
      const evidence = checkpointEvidence(issue, comments, approvers, checkpoint);
      const comparison = await api(
        `/repos/${repository}/compare/${evidence.source}...${pr.base.sha}`,
      );
      if (!['ahead', 'identical'].includes(comparison.status))
        throw new CheckError('Checkpoint source is not part of the PR base.');
      evidenceSnapshots.push(
        { route: issueRoute, value: issue },
        { route: issueRoute + '/comments', value: comments, paged: true },
      );
    }
    snapshots.push({ route, pr, evidenceSnapshots });
  }
  for (const snapshot of snapshots) {
    const current = await api(snapshot.route);
    if (
      current.head?.sha !== snapshot.pr.head.sha ||
      current.base?.sha !== snapshot.pr.base.sha ||
      current.body !== snapshot.pr.body ||
      current.state !== 'open'
    )
      throw new CheckError('Source PR metadata changed during validation.');
    for (const item of snapshot.evidenceSnapshots) {
      const current = item.paged ? await pages(api, item.route) : await api(item.route);
      if (JSON.stringify(current) !== JSON.stringify(item.value))
        throw new CheckError('Checkpoint evidence changed during validation.');
    }
  }
  if (
    eventName === 'merge_group' &&
    JSON.stringify(await resolveEvent(eventName, event, repository, api)) !==
      JSON.stringify(selection)
  )
    throw new CheckError('Merge queue changed during policy validation.');
  return count;
}

async function main() {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (!token || !process.env.GITHUB_EVENT_PATH)
    throw new CheckError('CI requires a read-only token and event file.');
  const count = await checkSourcePolicy({
    api: githubApi(token),
    repository: process.env.GITHUB_REPOSITORY,
    eventName: process.env.GITHUB_EVENT_NAME,
    event: JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, 'utf8')),
    policySha: process.env.CI_POLICY_SHA,
    checkpoints: JSON.parse(process.env.CHECKPOINT_CONFIG || '{}'),
  });
  console.log(`Source policy passed for ${count} actual contributed commit(s).`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main().catch((error) => {
    console.error(
      error instanceof CheckError
        ? error.message
        : 'Source policy could not read or validate required metadata.',
    );
    process.exitCode = 1;
  });
