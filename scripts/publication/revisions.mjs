import { createHash } from 'node:crypto';
import { normalizedBody } from './markdown.mjs';
import { prNumber, repositoryName, requireValue, revisionTimestamp, sha } from './metadata.mjs';

const hash = (text) => createHash('sha256').update(text).digest('hex');

// REST submitted_at is immutable. Bind each submitted review to mutable GraphQL
// metadata and retained edits, without normalizing another record's controls.
export async function readReviewRevisions(api, repository, number, reviews) {
  if (!reviews.length) return [];
  const [owner, name] = repositoryName(repository).split('/');
  prNumber(number);
  const query = `query ReviewRevisions($owner:String!,$name:String!,$number:Int!,$after:String) {
    repository(owner:$owner,name:$name) { pullRequest(number:$number) {
      reviews(first:100,after:$after) {
        totalCount pageInfo { hasNextPage endCursor }
        nodes { fullDatabaseId body state updatedAt lastEditedAt
          userContentEdits(first:100) {
            totalCount pageInfo { hasNextPage }
            nodes { id editedAt deletedAt diff }
          }
        }
      }
    } }
  }`;
  const expected = new Map(reviews.map((review) => [String(review.id), review]));
  const found = new Map();
  const cursors = new Set();
  let after = null;
  for (let page = 0; ; page++) {
    requireValue(page < 10, 'Review revision history exceeds the supported limit.');
    const response = await api('/graphql', { query, variables: { owner, name, number, after } });
    const connection = response.data?.repository?.pullRequest?.reviews;
    requireValue(
      !response.errors &&
        connection?.totalCount === reviews.length &&
        Array.isArray(connection.nodes) &&
        typeof connection.pageInfo?.hasNextPage === 'boolean',
      'Review revision pagination is incomplete or changed.',
    );
    for (const node of connection.nodes) {
      const id = String(node?.fullDatabaseId);
      const review = expected.get(id);
      requireValue(
        review && !found.has(id) && node.body === review.body && node.state === review.state,
        'Review changed or its revision identity is unavailable.',
      );
      const updated = revisionTimestamp(node.updatedAt);
      const edited = node.lastEditedAt === null ? null : revisionTimestamp(node.lastEditedAt);
      const history = node.userContentEdits;
      requireValue(
        Array.isArray(history?.nodes) &&
          history.pageInfo?.hasNextPage === false &&
          history.totalCount === history.nodes.length &&
          history.nodes.length <= 100,
        'Review edit history is incomplete or exceeds 100 retained edits.',
      );
      const seen = new Set();
      const edits = history.nodes.map((edit) => {
        requireValue(
          typeof edit?.id === 'string' && edit.id.length > 0 && !seen.has(edit.id),
          'Review edit identity is missing or duplicated.',
        );
        seen.add(edit.id);
        const deleted = edit.deletedAt === null ? null : revisionTimestamp(edit.deletedAt);
        requireValue(deleted || typeof edit.diff === 'string', 'Review edit text is unavailable.');
        return {
          id: edit.id,
          edited: revisionTimestamp(edit.editedAt),
          deleted,
          body: deleted ? null : edit.diff,
        };
      });
      requireValue(
        edits.length
          ? edits[0].edited === edited && edits[0].body === review.body
          : edited === null,
        'Review edit history is not current.',
      );
      found.set(id, { ...review, updated_at: updated, revisions: { edited, edits } });
    }
    if (!connection.pageInfo.hasNextPage) break;
    after = connection.pageInfo.endCursor;
    requireValue(
      typeof after === 'string' && after && !cursors.has(after),
      'Review revision cursor is incomplete.',
    );
    cursors.add(after);
  }
  requireValue(found.size === expected.size, 'Review revision history is incomplete.');
  return reviews.map((review) => found.get(String(review.id)));
}

// Keep chronological content-changing revisions, including edit-and-revert pairs.
// Consecutive edits of only our own two controls belong to the same revision.
export function bodyRevisionIdentity(edits, currentBody) {
  if (!edits.length) return [{ id: 'initial', body: hash(normalizedBody(currentBody)) }];
  requireValue(edits[0].diff === currentBody, 'Source PR revision history is not current.');
  const groups = [];
  for (const edit of [...edits].reverse()) {
    const deleted = edit.deletedAt === null ? null : revisionTimestamp(edit.deletedAt);
    requireValue(deleted || typeof edit.diff === 'string', 'Source PR edit text is unavailable.');
    const body = deleted ? null : hash(normalizedBody(edit.diff));
    if (!deleted && groups.at(-1)?.body === body) continue;
    groups.push({
      id: groups.length ? edit.id : 'initial',
      ...(groups.length ? { edited: revisionTimestamp(edit.editedAt) } : {}),
      ...(deleted ? { deleted, edit: edit.id } : {}),
      body,
    });
  }
  return groups;
}

export async function readSourceRevisions(api, repository, number, current) {
  const [owner, name] = repositoryName(repository).split('/');
  prNumber(number);
  const records = {};
  let observedEditedAt;
  for (const kind of ['body', 'title']) {
    const field = kind === 'body' ? 'userContentEdits' : 'timelineItems';
    const selection =
      kind === 'body'
        ? 'totalCount nodes { id editedAt deletedAt diff }'
        : 'nodes { ... on RenamedTitleEvent { id createdAt previousTitle currentTitle } }';
    const query = `query SourceRevisions($owner:String!,$name:String!,$number:Int!,$after:String) {
      repository(owner:$owner,name:$name) { pullRequest(number:$number) {
        body title lastEditedAt
        ${field}(first:100,after:$after${kind === 'title' ? ',itemTypes:[RENAMED_TITLE_EVENT]' : ''}) {
          ${selection} pageInfo { hasNextPage endCursor }
        }
      } }
    }`;
    const result = [];
    const ids = new Set();
    const cursors = new Set();
    let after = null;
    let count;
    for (let page = 0; ; page++) {
      requireValue(page < 10, 'Source PR revision history exceeds the supported limit.');
      const response = await api('/graphql', { query, variables: { owner, name, number, after } });
      const pr = response.data?.repository?.pullRequest;
      requireValue(
        !response.errors && pr?.body === current.body && pr?.title === current.title,
        'Source PR changed or its revision history is unavailable.',
      );
      const edited = pr.lastEditedAt === null ? null : revisionTimestamp(pr.lastEditedAt);
      requireValue(
        observedEditedAt === undefined || observedEditedAt === edited,
        'Source PR changed while reading revision history.',
      );
      observedEditedAt = edited;
      const connection = pr[field];
      requireValue(
        Array.isArray(connection?.nodes) && typeof connection.pageInfo?.hasNextPage === 'boolean',
        'Source PR revision pagination is incomplete.',
      );
      if (kind === 'body') {
        requireValue(
          Number.isSafeInteger(connection.totalCount) &&
            connection.totalCount >= 0 &&
            (count === undefined || count === connection.totalCount),
          'Source PR edit count changed.',
        );
        count = connection.totalCount;
      }
      for (const item of connection.nodes) {
        requireValue(
          typeof item?.id === 'string' &&
            item.id.length > 0 &&
            item.id.length <= 256 &&
            !ids.has(item.id),
          'Source PR revision identity is missing or duplicated.',
        );
        ids.add(item.id);
        if (kind === 'body') revisionTimestamp(item.editedAt);
        else {
          revisionTimestamp(item.createdAt);
          requireValue(
            typeof item.previousTitle === 'string' && typeof item.currentTitle === 'string',
            'Source PR title revision text is unavailable.',
          );
        }
        result.push(item);
      }
      if (!connection.pageInfo.hasNextPage) break;
      after = connection.pageInfo.endCursor;
      requireValue(
        typeof after === 'string' && after && !cursors.has(after),
        'Source PR revision cursor is incomplete.',
      );
      cursors.add(after);
    }
    if (kind === 'body') {
      requireValue(
        result.length === count &&
          (result.length ? result[0].editedAt === observedEditedAt : observedEditedAt === null),
        'Source PR body revision history is incomplete.',
      );
    }
    // Filtered timeline totalCount includes unrelated event types on GitHub.
    // Exhaust cursor pagination and reject duplicate IDs instead of using it.
    records[kind] = result;
  }
  return {
    identity: { body: bodyRevisionIdentity(records.body, current.body), title: records.title },
    texts: [
      ...records.body.filter((edit) => edit.deletedAt === null).map((edit) => edit.diff),
      ...records.title.flatMap((edit) => [edit.previousTitle, edit.currentTitle]),
    ],
  };
}

// Force-push timeline entries retain both sides even after branch refs move.
export async function readHeadRevisions(api, repository, number) {
  const [owner, name] = repositoryName(repository).split('/');
  prNumber(number);
  const query = `query HeadRevisions($owner:String!,$name:String!,$number:Int!,$after:String) {
    repository(owner:$owner,name:$name) { pullRequest(number:$number) {
      timelineItems(first:100,after:$after,itemTypes:[HEAD_REF_FORCE_PUSHED_EVENT]) {
        nodes { ... on HeadRefForcePushedEvent {
          id createdAt beforeCommit { oid } afterCommit { oid }
        } } pageInfo { hasNextPage endCursor }
      }
    } }
  }`;
  const events = [];
  const ids = new Set();
  const cursors = new Set();
  const heads = new Set();
  let after = null;
  for (let page = 0; ; page++) {
    requireValue(page < 10, 'Retained head timeline exceeds the supported limit.');
    const response = await api('/graphql', { query, variables: { owner, name, number, after } });
    const connection = response.data?.repository?.pullRequest?.timelineItems;
    requireValue(
      !response.errors &&
        Array.isArray(connection?.nodes) &&
        typeof connection.pageInfo?.hasNextPage === 'boolean',
      'Retained head timeline is unavailable.',
    );
    for (const event of connection.nodes) {
      requireValue(
        typeof event?.id === 'string' && event.id && !ids.has(event.id),
        'Retained head event identity is missing or duplicated.',
      );
      ids.add(event.id);
      const before = sha(event.beforeCommit?.oid);
      const next = sha(event.afterCommit?.oid);
      events.push({
        id: event.id,
        created: revisionTimestamp(event.createdAt),
        before,
        after: next,
      });
      heads.add(before);
      heads.add(next);
      requireValue(heads.size <= 100, 'Too many retained source heads.');
    }
    if (!connection.pageInfo.hasNextPage) break;
    after = connection.pageInfo.endCursor;
    requireValue(
      typeof after === 'string' && after && !cursors.has(after),
      'Retained head timeline cursor is incomplete.',
    );
    cursors.add(after);
  }
  const commits = new Set();
  const historyQuery = `query RetainedHeadCommits($owner:String!,$name:String!,$head:String!,$after:String) {
    repository(owner:$owner,name:$name) { object(expression:$head) { ... on Commit {
      oid history(first:100,after:$after) {
        totalCount nodes { oid } pageInfo { hasNextPage endCursor }
      }
    } } }
  }`;
  for (const head of heads) {
    const seen = new Set();
    const historyCursors = new Set();
    let cursor = null;
    let count;
    for (let page = 0; ; page++) {
      requireValue(page < 10, 'Retained commit ancestry exceeds the supported limit.');
      const response = await api('/graphql', {
        query: historyQuery,
        variables: { owner, name, head, after: cursor },
      });
      const object = response.data?.repository?.object;
      const history = object?.history;
      requireValue(
        !response.errors &&
          object?.oid === head &&
          Array.isArray(history?.nodes) &&
          Number.isSafeInteger(history.totalCount) &&
          history.totalCount > 0 &&
          (count === undefined || count === history.totalCount) &&
          typeof history.pageInfo?.hasNextPage === 'boolean',
        'Retained commit ancestry is unavailable or changed.',
      );
      count = history.totalCount;
      for (const node of history.nodes) {
        const oid = sha(node?.oid);
        requireValue(!seen.has(oid), 'Retained commit ancestry has duplicate pages.');
        seen.add(oid);
        commits.add(oid);
        requireValue(commits.size <= 10_000, 'Too many retained source commits.');
      }
      if (!history.pageInfo.hasNextPage) break;
      cursor = history.pageInfo.endCursor;
      requireValue(
        typeof cursor === 'string' && cursor && !historyCursors.has(cursor),
        'Retained commit ancestry cursor is incomplete.',
      );
      historyCursors.add(cursor);
    }
    requireValue(seen.size === count && seen.has(head), 'Retained commit ancestry is incomplete.');
  }
  return { events, heads: [...heads].sort(), commits: [...commits].sort() };
}
