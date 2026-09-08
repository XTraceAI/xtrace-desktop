import assert from 'node:assert/strict';
import test from 'node:test';
import { PublicationError } from './metadata.mjs';
import { bodyRevisionIdentity, readReviewRevisions, readSourceRevisions } from './revisions.mjs';

const time = '2026-01-01T00:00:00Z';
const current = { body: 'Public description.', title: 'Public title' };
const edit = (id, diff = current.body) => ({ id, diff, editedAt: time, deletedAt: null });

function fixture() {
  const state = {
    body: [edit('newest'), edit('oldest', 'Previous description.')],
    title: [
      {
        id: 'rename',
        createdAt: time,
        previousTitle: 'Previous title',
        currentTitle: current.title,
      },
    ],
    transform: () => {},
    calls: [],
  };
  state.api = async (path, request) => {
    assert.equal(path, '/graphql');
    const kind = request.query.includes('userContentEdits(') ? 'body' : 'title';
    const index = request.variables.after === null ? 0 : Number(request.variables.after);
    const field = kind === 'body' ? 'userContentEdits' : 'timelineItems';
    state.calls.push({ kind, index });
    const response = {
      data: {
        repository: {
          pullRequest: {
            ...current,
            lastEditedAt: time,
            [field]: {
              totalCount: kind === 'body' ? state.body.length : 999,
              nodes: structuredClone(state[kind].slice(index, index + 1)),
              pageInfo: {
                hasNextPage: index + 1 < state[kind].length,
                endCursor: String(index + 1),
              },
            },
          },
        },
      },
    };
    state.transform(response, kind, index, field);
    return response;
  };
  return state;
}
const read = (state) => readSourceRevisions(state.api, 'example/project', 3, current);

test('revision pagination exhausts both connections and ignores unfiltered timeline totals', async () => {
  const state = fixture();
  const result = await read(state);
  assert.equal(result.identity.body.length, 2);
  assert.equal(result.identity.title.length, 1);
  assert.ok(result.texts.includes('Previous description.'));
  assert.ok(result.texts.includes('Previous title'));
  assert.deepEqual(state.calls, [
    { kind: 'body', index: 0 },
    { kind: 'body', index: 1 },
    { kind: 'title', index: 0 },
  ]);
});

test('deleted body revisions invalidate identity without disclosing inaccessible text', async () => {
  const before = [edit('newest'), edit('oldest', 'Earlier body')];
  const after = [before[0], { ...before[1], deletedAt: time, diff: null }];
  assert.notDeepEqual(
    bodyRevisionIdentity(before, current.body),
    bodyRevisionIdentity(after, current.body),
  );
  const state = fixture();
  state.body = after;
  const result = await read(state);
  assert.equal(result.identity.body[0].deleted, time);
  assert.deepEqual(
    result.texts.filter((text) => text === null),
    [],
  );
});

test('incomplete or inconsistent source revision responses fail closed', async (t) => {
  const cases = {
    'API error': (response) => {
      response.errors = [{ message: 'Unavailable' }];
    },
    'changed body': (response) => {
      response.data.repository.pullRequest.body = 'Changed';
    },
    'changed title': (response) => {
      response.data.repository.pullRequest.title = 'Changed';
    },
    'missing edit time': (response) => {
      delete response.data.repository.pullRequest.lastEditedAt;
    },
    'mid-pagination edit': (response, kind, index) => {
      if (kind === 'body' && index)
        response.data.repository.pullRequest.lastEditedAt = '2026-01-02T00:00:00Z';
    },
    'edit between body and title reads': (response, kind) => {
      if (kind === 'title')
        response.data.repository.pullRequest.lastEditedAt = '2026-01-02T00:00:00Z';
    },
    'stale edit timestamp': (response, kind, index, field) => {
      if (kind === 'body' && !index)
        response.data.repository.pullRequest[field].nodes[0].editedAt = '2025-12-31T00:00:00Z';
    },
    'truncated body list': (response, kind, index, field) => {
      if (kind === 'body') response.data.repository.pullRequest[field].totalCount++;
    },
    'missing cursor': (response, kind, index, field) => {
      response.data.repository.pullRequest[field].pageInfo.endCursor = null;
    },
    'repeated ID': (response, kind, index, field) => {
      if (kind === 'body' && index)
        response.data.repository.pullRequest[field].nodes[0].id = 'newest';
    },
    'stale newest revision': (response, kind, index, field) => {
      if (kind === 'body' && !index)
        response.data.repository.pullRequest[field].nodes[0].diff = 'Stale body';
    },
    'unavailable historical body': (response, kind, index, field) => {
      if (kind === 'body' && index)
        response.data.repository.pullRequest[field].nodes[0].diff = null;
    },
    'missing title revision': (response, kind, index, field) => {
      if (kind === 'title') response.data.repository.pullRequest[field].nodes[0] = null;
    },
  };
  for (const [name, transform] of Object.entries(cases)) {
    await t.test(name, async () => {
      const state = fixture();
      state.transform = transform;
      await assert.rejects(read(state));
    });
  }
});

test('review revisions paginate by review and reject incomplete history', async (t) => {
  const reviews = [1, 2].map((id) => ({ id, body: 'Current review.', state: 'COMMENTED' }));
  const makeApi =
    (mutate = () => {}) =>
    async (_path, request) => {
      const index = request.variables.after === null ? 0 : 1;
      const review = reviews[index];
      const node = {
        fullDatabaseId: String(review.id),
        body: review.body,
        state: review.state,
        updatedAt: time,
        lastEditedAt: time,
        userContentEdits: {
          totalCount: 1,
          nodes: [edit('edit-' + index, review.body)],
          pageInfo: { hasNextPage: false },
        },
      };
      const connection = {
        totalCount: 2,
        nodes: [node],
        pageInfo: { hasNextPage: !index, endCursor: 'next' },
      };
      const response = { data: { repository: { pullRequest: { reviews: connection } } } };
      mutate(node, connection, response, index);
      return response;
    };
  const readReviews = (api) => readReviewRevisions(api, 'example/project', 3, reviews);
  const result = await readReviews(makeApi());
  assert.deepEqual(
    result.map((r) => r.id),
    [1, 2],
  );
  assert.equal(result[0].updated_at, time);
  assert.equal(result[1].revisions.edits[0].id, 'edit-1');
  const cases = {
    'API error': (_n, _c, r) => (r.errors = [{}]),
    'changed body': (n) => (n.body = 'Different'),
    'changed state': (n) => (n.state = 'DISMISSED'),
    'missing mutable timestamp': (n) => delete n.updatedAt,
    'missing edit timestamp': (n) => delete n.lastEditedAt,
    'wrong review identity': (n) => (n.fullDatabaseId = '99'),
    'missing review': (_n, c) => (c.nodes = []),
    'changed count': (_n, c) => c.totalCount++,
    'missing cursor': (_n, c) => delete c.pageInfo.endCursor,
    'repeated cursor': (_n, c) => (c.pageInfo.hasNextPage = true),
    'partial edit history': (n) => (n.userContentEdits.pageInfo.hasNextPage = true),
    'truncated edits': (n) => n.userContentEdits.totalCount++,
    'duplicate edits': (n) => {
      n.userContentEdits.nodes.push(n.userContentEdits.nodes[0]);
      n.userContentEdits.totalCount++;
    },
    'stale edit body': (n) => (n.userContentEdits.nodes[0].diff = 'Stale'),
    'stale edit time': (n) => (n.lastEditedAt = '2026-01-02T00:00:00Z'),
    'unavailable retained text': (n) => (n.userContentEdits.nodes[0].diff = null),
  };
  for (const [name, mutate] of Object.entries(cases))
    await t.test(name, async () => assert.rejects(readReviews(makeApi(mutate)), PublicationError));
});
