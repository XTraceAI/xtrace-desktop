import { createHash } from 'node:crypto';
import { normalizedBody } from './markdown.mjs';
import { prNumber, repositoryName, requireValue, revisionTimestamp } from './metadata.mjs';

const hash = (text) => createHash('sha256').update(text).digest('hex');

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
