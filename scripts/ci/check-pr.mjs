import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Parser } from 'commonmark';
import { githubApi, pages } from '../publication/api.mjs';
import {
  repositoryName,
  resolveEvent,
  sha,
  validatePullRequest,
} from '../publication/metadata.mjs';
import { CheckError, validateDco } from './policy.mjs';

export function validateDescription(body) {
  const tree = new Parser().parse(body);
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
      const value = text.join('').trim();
      if (
        value &&
        !/^(?:Describe |List commands |Follow |Disclosure snapshot: |\[[ xX]\] I reviewed the final PR text)/.test(
          value,
        )
      )
        sections.set(section, true);
    }
  }
  if (!sections.get('Change') || !sections.get('Verification'))
    throw new CheckError('PR needs substantive Change and Verification sections.');
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

export async function checkSourcePolicy({ api, repository, eventName, event }) {
  repositoryName(repository);
  if (eventName === 'push') {
    if (
      event.repository?.full_name !== repository ||
      typeof event.repository?.default_branch !== 'string' ||
      !event.repository.default_branch ||
      event.ref !== 'refs/heads/' + event.repository.default_branch
    )
      throw new CheckError('Unsupported push event.');
    sha(event.before);
    sha(event.after);
    // DCO certifies contributed PR commits before merge. A provider-created
    // squash commit has a different author/message and is not a contribution.
    // Main CI validates the resulting source; repository rules require PR merges.
    return null;
  }
  const selection = await resolveEvent(eventName, event, repository, api);
  const snapshots = [];
  let count = 0;
  for (const member of selection.members) {
    const route = `/repos/${repository}/pulls/${member.number}`;
    // Contribution policy is automatic. The solo-maintainer publication review
    // runs separately before the repository or a release becomes public.
    const pr = validatePullRequest(await api(route), repository, member.head, {
      attestation: false,
    });
    validateDescription(pr.body);
    const commits = await sourceCommits(api, repository, pr);
    for (const commit of commits) validateDco(commit);
    count += commits.length;
    snapshots.push({ route, pr });
  }
  for (const snapshot of snapshots) {
    const current = await api(snapshot.route);
    if (
      current.head?.sha !== snapshot.pr.head.sha ||
      current.base?.sha !== snapshot.pr.base.sha ||
      current.body !== snapshot.pr.body ||
      current.title !== snapshot.pr.title ||
      current.state !== 'open'
    )
      throw new CheckError('Source PR metadata changed during validation.');
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
  });
  console.log(
    count === null
      ? 'Post-merge validation: contribution policy belongs to the source PR, not its squash commit.'
      : `Source policy passed for ${count} actual contributed commit(s).`,
  );
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
