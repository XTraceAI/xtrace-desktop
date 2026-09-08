import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  linkedIssues,
  prNumber,
  PublicationError,
  repositoryName,
  requireValue,
  resolveEvent,
  sha,
  validatePullRequest,
} from './metadata.mjs';

function options(args) {
  const result = { repo: process.cwd(), repository: process.env.GITHUB_REPOSITORY };
  if (args[0] === '--') args.shift();
  while (args.length) {
    const flag = args.shift();
    requireValue(
      ['--repo', '--repository', '--pr'].includes(flag) && args[0],
      'Usage: publication:check [--repo PATH] [--repository OWNER/REPO --pr NUMBER]',
    );
    result[flag.slice(2)] = args.shift();
  }
  return result;
}

async function main() {
  const config = options(process.argv.slice(2));
  const repo = resolve(config.repo);
  const repository = repositoryName(config.repository);
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  requireValue(token, 'A read-only GITHUB_TOKEN or GH_TOKEN is required.');
  const api = async (path, body) => {
    let response;
    try {
      response = await fetch(`https://api.github.com${path}`, {
        method: body ? 'POST' : 'GET',
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: 'application/vnd.github+json',
          'X-GitHub-Api-Version': '2022-11-28',
          'Content-Type': 'application/json',
        },
        body: body ? JSON.stringify(body) : undefined,
        redirect: 'error',
        signal: AbortSignal.timeout(30_000),
      });
    } catch {
      throw new PublicationError('GitHub metadata request failed.');
    }
    requireValue(response.ok, `GitHub metadata request failed (HTTP ${response.status}).`);
    try {
      return await response.json();
    } catch {
      throw new PublicationError('GitHub metadata response was invalid.');
    }
  };
  const git = (...args) => {
    try {
      return execFileSync('git', ['-c', 'credential.helper=', '-c', 'core.askPass=', ...args], {
        cwd: repo,
        encoding: 'utf8',
        timeout: 120_000,
        stdio: ['ignore', 'pipe', 'pipe'],
        env: {
          ...process.env,
          GIT_TERMINAL_PROMPT: '0',
          GIT_TRACE: '0',
          GIT_TRACE_CURL: '0',
          GIT_CURL_VERBOSE: '0',
          GIT_CONFIG_COUNT: '1',
          GIT_CONFIG_KEY_0: `http.https://github.com/${repository}.git.extraheader`,
          GIT_CONFIG_VALUE_0: `AUTHORIZATION: basic ${Buffer.from(`x-access-token:${token}`).toString('base64')}`,
        },
      }).trim();
    } catch {
      throw new PublicationError('Required source history could not be fetched or verified.');
    }
  };
  requireValue(
    git('rev-parse', '--is-shallow-repository') === 'false',
    'Publication checks require a full clone.',
  );
  let event;
  let eventName = process.env.GITHUB_EVENT_NAME;
  if (config.pr) {
    requireValue(!process.env.GITHUB_ACTIONS, 'CI must use the actual event payload.');
    const pr = await api(`/repos/${repository}/pulls/${prNumber(Number(config.pr))}`);
    event = { repository: { full_name: repository }, pull_request: pr };
    eventName = 'pull_request';
  } else {
    requireValue(
      process.env.GITHUB_EVENT_PATH,
      'Use --repository OWNER/REPO --pr NUMBER outside GitHub Actions.',
    );
    event = JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, 'utf8'));
  }
  const selection = await resolveEvent(eventName, event, repository, api);
  const expectedCheckout = process.env.GITHUB_ACTIONS
    ? sha(process.env.GITHUB_SHA)
    : selection.head;
  requireValue(
    git('rev-parse', 'HEAD') === expectedCheckout,
    'Checkout does not match the commit being checked.',
  );
  if (eventName === 'merge_group')
    requireValue(expectedCheckout === selection.head, 'Checkout does not match the merge group.');
  const temporary = await mkdtemp(join(tmpdir(), 'xtrace-publication-'));
  try {
    const scanArgs = ['--repo', repo, '--diff', `${selection.base}..${selection.head}`];
    const allowedRepositories = new Set([repository.toLowerCase()]);
    const snapshots = new Map();
    const snapshot = (item) =>
      JSON.stringify([
        item.title,
        item.body,
        item.head?.sha,
        item.head?.ref,
        item.base?.sha,
        item.base?.ref,
      ]);
    let contentCount = 0;
    for (const member of selection.members) {
      const pr = validatePullRequest(
        await api(`/repos/${repository}/pulls/${member.number}`),
        repository,
        member.head,
      );
      git(
        'fetch',
        '--no-tags',
        '--force',
        `https://github.com/${repository}.git`,
        `+refs/pull/${member.number}/head:refs/publication/pr-${member.number}`,
      );
      requireValue(
        git('rev-parse', `refs/publication/pr-${member.number}`) === member.head,
        'Source PR changed while its history was fetched; rerun checks.',
      );
      // Source heads may not be ancestors of squash/rebase queue commits.
      git('cat-file', '-e', `${pr.base.sha}^{commit}`);
      scanArgs.push('--diff', `${pr.base.sha}..${member.head}`);
      snapshots.set(`/repos/${repository}/pulls/${member.number}`, snapshot(pr));
      const texts = [
        `${pr.title}\n\nBase branch: ${pr.base.ref}\nSource branch: ${pr.head.ref}\n\n${pr.body}`,
      ];
      for (const issue of linkedIssues(pr.body, repository)) {
        if (!allowedRepositories.has(issue.repository.toLowerCase())) {
          const linkedRepository = await api(`/repos/${issue.repository}`);
          requireValue(
            linkedRepository.private === false,
            'Linked issue must be publicly accessible for publication review.',
          );
          allowedRepositories.add(issue.repository.toLowerCase());
        }
        const linked = await api(`/repos/${issue.repository}/issues/${issue.number}`);
        requireValue(
          typeof linked.title === 'string' &&
            (linked.body === null || typeof linked.body === 'string'),
          'Linked issue text is unavailable.',
        );
        texts.push(`${linked.title}\n\n${linked.body ?? ''}`);
        snapshots.set(`/repos/${issue.repository}/issues/${issue.number}`, snapshot(linked));
      }
      for (const text of texts) {
        const path = join(temporary, `public-text-${++contentCount}.md`);
        await writeFile(path, text, { mode: 0o600 });
        scanArgs.push('--content', path);
      }
    }
    const scanner = fileURLToPath(new URL('../security/scan-secrets.mjs', import.meta.url));
    const result = spawnSync(process.execPath, [scanner, ...scanArgs], {
      cwd: repo,
      stdio: 'inherit',
      env: { ...process.env, GITHUB_TOKEN: '', GH_TOKEN: '' },
    });
    requireValue(
      !result.error && result.status === 0,
      'Secret scan did not pass; publication remains blocked.',
    );
    for (const [path, before] of snapshots) {
      requireValue(
        snapshot(await api(path)) === before,
        'PR or linked-issue text changed during the scan; review the current content and rerun checks.',
      );
    }
    if (eventName === 'merge_group') {
      const current = await resolveEvent(eventName, event, repository, api);
      requireValue(
        JSON.stringify(current) === JSON.stringify(selection),
        'Merge queue changed during the scan; rerun checks.',
      );
    }
    console.log(
      `Publication check passed for ${selection.members.length} source PR(s) and ${contentCount} public text item(s). Human disclosure attestation verified; attachments and semantics require human review.`,
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

main().catch((error) => {
  // Only deliberately authored summaries may reach public logs.
  console.error(
    error instanceof PublicationError
      ? error.message
      : 'Publication check failed while reading required metadata or temporary content.',
  );
  process.exitCode = 1;
});
