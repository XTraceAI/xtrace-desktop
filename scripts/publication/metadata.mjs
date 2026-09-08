export const ATTESTATION =
  'I reviewed the final PR text, linked issues, comments, and attachments for public disclosure.';

export class PublicationError extends Error {}

export function requireValue(condition, message) {
  if (!condition) throw new PublicationError(message);
}

export function sha(value) {
  requireValue(
    typeof value === 'string' && /^[a-f0-9]{40}$/.test(value),
    'Missing or invalid source commit.',
  );
  return value;
}

export function repositoryName(value) {
  requireValue(
    typeof value === 'string' &&
      /^[A-Za-z0-9][A-Za-z0-9-]*\/[A-Za-z0-9_.-]+$/.test(value) &&
      !['.', '..'].includes(value.split('/')[1]),
    'Missing or invalid repository.',
  );
  return value;
}

export function prNumber(value) {
  requireValue(Number.isSafeInteger(value) && value > 0, 'Missing or invalid pull request number.');
  return value;
}

export function requireAttestation(body) {
  requireValue(typeof body === 'string', 'Pull request body is unavailable.');
  // Preserve spacing when removing comments so hidden fragments cannot create
  // a checked box. Attestations belong outside raw HTML and fenced examples.
  const blank = (text) => text.replace(/[^\r\n]/g, ' ');
  const lines = body
    .replace(/<!--[\s\S]*?(?:-->|$)/g, blank)
    .replace(/<(pre|code|script|style|textarea)\b[^>]*>[\s\S]*?(?:<\/\1\s*>|$)/gi, blank)
    .replace(/<([a-z][a-z0-9-]*)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, blank)
    .split(/\r?\n/);
  let fence = null;
  let htmlBlock = false;
  const declarations = [];
  for (const line of lines) {
    if (htmlBlock) {
      if (line.trim() === '') htmlBlock = false;
      continue;
    }
    // CommonMark raw block HTML lasts to the next blank line, including an
    // unclosed div/table. Void tags followed by a blank line remain harmless.
    if (
      !fence &&
      (/^ {0,3}<\/?(?:address|article|aside|blockquote|body|caption|center|col|colgroup|dd|details|dialog|div|dl|dt|fieldset|figcaption|figure|footer|form|h[1-6]|head|header|hr|html|iframe|li|main|menu|nav|ol|p|section|summary|table|tbody|td|tfoot|th|thead|title|tr|ul)(?:\s|\/?>|$)/i.test(
        line,
      ) ||
        /^ {0,3}<\/?[a-z][a-z0-9-]*(?:\s[^<>]*)?\/?>\s*$/i.test(line))
    ) {
      htmlBlock = true;
      continue;
    }
    const marker = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (marker) {
      if (!fence) fence = marker[1];
      else if (
        marker[1][0] === fence[0] &&
        marker[1].length >= fence.length &&
        line.slice(marker[0].length).trim() === ''
      )
        fence = null;
      continue;
    }
    if (fence) continue;
    const declaration = line.match(/^ {0,3}- \[([ xX])\] (.+?)\s*$/);
    if (declaration?.[2] === ATTESTATION) declarations.push(declaration[1]);
  }
  requireValue(
    declarations.length === 1 && declarations[0].toLowerCase() === 'x',
    'Public-disclosure review attestation must be checked exactly once in the PR body.',
  );
}

export function validatePullRequest(pr, repository, expectedHead) {
  prNumber(pr?.number);
  requireValue(pr?.base?.repo?.full_name === repository, 'Source PR must target this repository.');
  requireValue(
    pr.state === 'open' && typeof pr.title === 'string' && typeof pr.body === 'string',
    'Current source PR metadata is unavailable.',
  );
  sha(pr.base.sha);
  sha(pr.head?.sha);
  requireValue(
    typeof pr.base.ref === 'string' && typeof pr.head.ref === 'string',
    'Source branch metadata is unavailable.',
  );
  requireValue(
    pr.head.sha === expectedHead,
    'Source PR changed during this run; run checks for its current commit.',
  );
  requireAttestation(pr.body);
  return pr;
}

// Use the API's exact synthetic-commit chain, never a PR number guessed from a
// queue branch name or the synthetic commit message. Unknown queue shapes fail.
export function queueMembers(group, entries) {
  const base = sha(group?.base_sha);
  let cursor = sha(group?.head_sha);
  requireValue(base !== cursor && Array.isArray(entries), 'Merge queue metadata is unavailable.');
  const members = [];
  const seen = new Set();
  while (cursor !== base) {
    requireValue(!seen.has(cursor), 'Merge queue metadata contains a cycle.');
    seen.add(cursor);
    const matches = entries.filter((entry) => entry?.headCommit?.oid === cursor);
    requireValue(
      matches.length === 1,
      'Cannot resolve every source PR in the merge group; retry with a current queue entry.',
    );
    const entry = matches[0];
    prNumber(entry.pullRequest?.number);
    sha(entry.pullRequest?.headRefOid);
    members.unshift({ number: entry.pullRequest.number, head: entry.pullRequest.headRefOid });
    cursor = sha(entry.baseCommit?.oid);
    requireValue(members.length <= 1000, 'Merge group exceeds the supported resolution limit.');
  }
  requireValue(
    new Set(members.map((member) => member.number)).size === members.length,
    'Merge group contains duplicate source PRs.',
  );
  return members;
}

export function linkedIssues(body, repository) {
  repositoryName(repository);
  const references = new Map();
  const add = (repo, number) => {
    repositoryName(repo);
    const value = prNumber(Number(number));
    references.set(`${repo.toLowerCase()}#${value}`, { repository: repo, number: value });
  };
  let remaining = body.replace(
    /(?:https?:\/\/github\.com|(?<![\w/]))\/([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)\/(?:issues|pull)\/(\d+)\b/gi,
    (_, repo, number) => {
      add(repo, number);
      return '';
    },
  );
  remaining = remaining.replace(
    /\b([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#(\d+)\b/g,
    (_, repo, number) => {
      add(repo, number);
      return '';
    },
  );
  for (const match of remaining.matchAll(/(?:^|[\s(])#(\d+)\b/g)) add(repository, match[1]);
  requireValue(references.size <= 100, 'Too many linked issues to review in one PR.');
  return [...references.values()];
}

export async function readQueue(api, repository, branch) {
  const [owner, name] = repositoryName(repository).split('/');
  const query = `query($owner:String!,$name:String!,$branch:String!,$after:String) {
    repository(owner:$owner,name:$name) { mergeQueue(branch:$branch) {
      entries(first:100,after:$after) {
        totalCount pageInfo { hasNextPage endCursor }
        nodes { baseCommit { oid } headCommit { oid } pullRequest { number headRefOid } }
      }
    } }
  }`;
  const entries = [];
  let after = null;
  const cursors = new Set();
  for (let page = 0; page < 10; page++) {
    const response = await api('/graphql', { query, variables: { owner, name, branch, after } });
    requireValue(!response.errors, 'GitHub could not resolve the merge queue.');
    const connection = response.data?.repository?.mergeQueue?.entries;
    requireValue(
      Array.isArray(connection?.nodes) &&
        typeof connection.pageInfo?.hasNextPage === 'boolean' &&
        Number.isSafeInteger(connection.totalCount),
      'Merge queue response is incomplete.',
    );
    entries.push(...connection.nodes);
    if (!connection.pageInfo.hasNextPage) {
      requireValue(
        entries.length === connection.totalCount,
        'Merge queue changed during pagination; rerun checks.',
      );
      return entries;
    }
    after = connection.pageInfo.endCursor;
    requireValue(
      typeof after === 'string' && after.length > 0 && !cursors.has(after),
      'Merge queue pagination is incomplete.',
    );
    cursors.add(after);
  }
  throw new PublicationError('Merge queue exceeds the supported resolution limit.');
}

export async function resolveEvent(eventName, event, repository, api) {
  repositoryName(repository);
  requireValue(
    event.repository?.full_name === repository,
    'Event repository does not match the checkout.',
  );
  if (eventName === 'pull_request') {
    const number = prNumber(event.pull_request?.number);
    const head = sha(event.pull_request?.head?.sha);
    return { base: sha(event.pull_request?.base?.sha), head, members: [{ number, head }] };
  }
  requireValue(
    eventName === 'merge_group' && event.action === 'checks_requested',
    'Unsupported publication event.',
  );
  const group = event.merge_group;
  requireValue(
    typeof group?.base_ref === 'string' && group.base_ref.startsWith('refs/heads/'),
    'Merge queue base branch is unavailable.',
  );
  const entries = await readQueue(api, repository, group.base_ref.slice('refs/heads/'.length));
  return {
    base: sha(group.base_sha),
    head: sha(group.head_sha),
    members: queueMembers(group, entries),
  };
}
