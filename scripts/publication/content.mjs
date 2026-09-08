import { createHash } from 'node:crypto';
import { pages } from './api.mjs';
import { disclosureControls, normalizedBody } from './markdown.mjs';
import { readReviewRevisions, readSourceRevisions } from './revisions.mjs';
import {
  linkedIssues,
  prNumber,
  PublicationError,
  repositoryName,
  requireAttestation,
  revisionTimestamp,
  requireValue,
  sha,
  validatePullRequest,
} from './metadata.mjs';

export async function closingIssues(api, repository, number) {
  const [owner, name] = repositoryName(repository).split('/');
  prNumber(number);
  const query = `query($owner:String!,$name:String!,$number:Int!,$after:String) {
    repository(owner:$owner,name:$name) { pullRequest(number:$number) {
      closingIssuesReferences(first:100,after:$after) {
        totalCount pageInfo { hasNextPage endCursor }
        nodes { number repository { nameWithOwner } }
      }
    } }
  }`;
  const result = [];
  const cursors = new Set();
  let after = null;
  for (let page = 0; page < 10; page++) {
    const response = await api('/graphql', { query, variables: { owner, name, number, after } });
    requireValue(!response.errors, 'GitHub could not resolve linked issue relationships.');
    const connection = response.data?.repository?.pullRequest?.closingIssuesReferences;
    requireValue(
      Array.isArray(connection?.nodes) &&
        Number.isSafeInteger(connection.totalCount) &&
        typeof connection.pageInfo?.hasNextPage === 'boolean',
      'Linked issue relationship response is incomplete.',
    );
    for (const item of connection.nodes)
      result.push({
        repository: repositoryName(item?.repository?.nameWithOwner),
        number: prNumber(item?.number),
      });
    if (!connection.pageInfo.hasNextPage) {
      requireValue(result.length === connection.totalCount, 'Linked issue pagination changed.');
      return result;
    }
    after = connection.pageInfo.endCursor;
    requireValue(
      typeof after === 'string' && after && !cursors.has(after),
      'Linked issue pagination is incomplete.',
    );
    cursors.add(after);
  }
  throw new PublicationError('Too many linked issue relationships.');
}

function discussion(items, includeDiff = false) {
  const seen = new Set();
  return items
    .map((item) => {
      requireValue(
        Number.isSafeInteger(item.id) && !seen.has(item.id),
        'Discussion pagination is incomplete.',
      );
      seen.add(item.id);
      requireValue(typeof item.body === 'string', 'Discussion text is unavailable.');
      if (includeDiff)
        requireValue(
          typeof item.diff_hunk === 'string',
          'Review comment diff context is unavailable.',
        );
      return {
        id: item.id,
        author: item.user?.login ?? null,
        body: item.body,
        ...(includeDiff ? { diff_hunk: item.diff_hunk } : {}),
        state: item.state ?? null,
        path: item.path ?? null,
        line: item.line ?? null,
        updated: revisionTimestamp(item.updated_at),
        ...(item.revisions ? { revisions: item.revisions } : {}),
        commit: item.commit_id ?? null,
      };
    })
    .sort((a, b) => a.id - b.id);
}

async function conversations(api, repository, number, isPullRequest, repositoryComments) {
  const path = '/repos/' + repository;
  const result = {
    comments: discussion(await pages(api, path + '/issues/' + number + '/comments')),
  };
  if (isPullRequest) {
    result.reviews = discussion(
      await readReviewRevisions(
        api,
        repository,
        number,
        await pages(api, path + '/pulls/' + number + '/reviews'),
      ),
    );
    result.reviewComments = discussion(
      await pages(api, path + '/pulls/' + number + '/comments'),
      true,
    );
    const pr = await api(path + '/pulls/' + number);
    const commits = await pages(api, path + '/pulls/' + number + '/commits');
    const ids = new Set(commits.map((commit) => sha(commit.sha)));
    // GitHub caps this endpoint at 250 commits. Never accept a truncated list.
    requireValue(
      Number.isSafeInteger(pr.commits) &&
        pr.commits > 0 &&
        ids.size === pr.commits &&
        commits.length === ids.size,
      'Source commit pagination is incomplete.',
    );
    result.commitComments = discussion(
      repositoryComments.filter((comment) => ids.has(sha(comment.commit_id))),
    );
  }
  return result;
}

export async function readPublicContent(api, repository, number, candidateBody) {
  repositoryName(repository);
  prNumber(number);
  const pr = await api('/repos/' + repository + '/pulls/' + number);
  validatePullRequest(pr, repository, pr.head?.sha, { attestation: false });
  if (candidateBody !== undefined) {
    requireValue(
      typeof candidateBody === 'string' &&
        typeof pr.body === 'string' &&
        normalizedBody(candidateBody) === normalizedBody(pr.body),
      'Publish the reviewed PR prose with a pending snapshot before preparing its revision-bound attestation.',
    );
  }
  const revisions = await readSourceRevisions(api, repository, number, pr);
  if (candidateBody !== undefined) pr.body = candidateBody;
  const repositoryComments = await pages(api, '/repos/' + repository + '/comments');
  // Detect duplicate pages even for comments outside this PR's commit set.
  discussion(repositoryComments);
  const references = new Map();
  for (const reference of [
    ...linkedIssues(pr.body, repository),
    ...(await closingIssues(api, repository, number)),
  ]) {
    const key = reference.repository.toLowerCase() + '#' + reference.number;
    if (key !== repository.toLowerCase() + '#' + number) references.set(key, reference);
  }
  requireValue(references.size <= 100, 'Too many linked issues to review in one PR.');
  const linked = [];
  const texts = [pr.title, pr.body, pr.base.ref, pr.head.ref, ...revisions.texts];
  for (const [key, reference] of [...references].sort(([a], [b]) => a.localeCompare(b))) {
    requireValue(
      reference.repository.toLowerCase() === repository.toLowerCase(),
      'Cross-repository issue references are unsupported because their edits cannot invalidate this gate.',
    );
    const issue = await api('/repos/' + reference.repository + '/issues/' + reference.number);
    requireValue(
      typeof issue.title === 'string' && (issue.body === null || typeof issue.body === 'string'),
      'Linked issue text is unavailable.',
    );
    texts.push(issue.title, issue.body ?? '');
    linked.push({
      key,
      title: issue.title,
      body: issue.body ?? '',
      updated: revisionTimestamp(issue.updated_at),
      ...(await conversations(
        api,
        reference.repository,
        reference.number,
        Boolean(issue.pull_request),
        repositoryComments,
      )),
    });
  }
  const content = {
    version: 2,
    revisions: revisions.identity,
    repository: repository.toLowerCase(),
    number,
    head: pr.head.sha,
    baseBranch: pr.base.ref,
    headBranch: pr.head.ref,
    title: pr.title,
    body: normalizedBody(pr.body),
    ...(await conversations(api, repository, number, true, repositoryComments)),
    linked,
  };
  for (const item of [content, ...linked])
    for (const group of [
      item.comments,
      item.reviews ?? [],
      item.reviewComments ?? [],
      item.commitComments ?? [],
    ])
      for (const comment of group) {
        texts.push(comment.body);
        for (const edit of comment.revisions?.edits ?? [])
          if (edit.body !== null) texts.push(edit.body);
        if (typeof comment.diff_hunk === 'string') texts.push(comment.diff_hunk);
      }
  texts.push(JSON.stringify(content));
  const digest = createHash('sha256').update(JSON.stringify(content)).digest('hex');
  return { pr, digest, texts, content };
}

export function requireDisclosure(review) {
  requireAttestation(review.pr.body);
  const snapshots = disclosureControls(review.pr.body).filter((control) => control.snapshot);
  requireValue(
    snapshots.length === 1 && snapshots[0].snapshot[1] === review.digest,
    'Disclosure snapshot is missing or stale; review current content and refresh the snapshot.',
  );
}
