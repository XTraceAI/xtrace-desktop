import { PublicationError, requireValue } from './metadata.mjs';

export function githubApi(token) {
  requireValue(token, 'A GitHub token is required.');
  return async (path, body, method = body ? 'POST' : 'GET') => {
    let response;
    try {
      response = await fetch('https://api.github.com' + path, {
        method,
        headers: {
          Authorization: 'Bearer ' + token,
          Accept: 'application/vnd.github+json',
          'X-GitHub-Api-Version': '2022-11-28',
          'Content-Type': 'application/json',
        },
        body: body ? JSON.stringify(body) : undefined,
        redirect: 'error',
        signal: AbortSignal.timeout(30_000),
      });
      requireValue(response.ok, 'GitHub metadata request failed (HTTP ' + response.status + ').');
      const text = await response.text();
      requireValue(text.length <= 16 * 1024 * 1024, 'GitHub response exceeds the supported limit.');
      return JSON.parse(text);
    } catch (error) {
      throw error instanceof PublicationError
        ? error
        : new PublicationError('GitHub metadata request failed.');
    }
  };
}

export async function pages(api, path) {
  const results = [];
  for (let page = 1; page <= 10; page++) {
    const items = await api(path + (path.includes('?') ? '&' : '?') + 'per_page=100&page=' + page);
    requireValue(Array.isArray(items), 'GitHub pagination response is incomplete.');
    results.push(...items);
    if (items.length < 100) return results;
  }
  throw new PublicationError('GitHub pagination exceeds the supported limit.');
}
