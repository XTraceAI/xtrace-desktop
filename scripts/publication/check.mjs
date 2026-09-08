import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  prNumber,
  PublicationError,
  repositoryName,
  requireValue,
  resolveEvent,
  sha,
} from './metadata.mjs';
import { githubApi } from './api.mjs';
import { readPublicContent, requireDisclosure } from './content.mjs';
import { disclosureControls } from './markdown.mjs';

function options(args) {
  const result = { repo: process.cwd(), repository: process.env.GITHUB_REPOSITORY };
  if (args[0] === '--') args.shift();
  while (args.length) {
    const flag = args.shift();
    if (flag === '--snapshot') {
      result.snapshot = true;
      continue;
    }
    requireValue(
      ['--repo', '--repository', '--pr', '--body'].includes(flag) && args[0],
      'Usage: publication:check [--repo PATH] [--repository OWNER/REPO --pr NUMBER] [--snapshot --body FILE]',
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
  const api = githubApi(token);
  if (config.snapshot) {
    requireValue(
      !process.env.GITHUB_ACTIONS && config.pr,
      'Snapshot preparation requires a local PR selection.',
    );
    const body = config.body ? await readFile(config.body, 'utf8') : undefined;
    const review = await readPublicContent(api, repository, prNumber(Number(config.pr)), body);
    requireValue(
      disclosureControls(review.pr.body).filter((item) => item.snapshot).length === 1,
      'Include one visible Disclosure snapshot: pending line before preparing a snapshot.',
    );
    console.log('Disclosure snapshot: ' + review.digest);
    return;
  }
  requireValue(!config.body, 'A candidate body is only accepted when preparing a snapshot.');
  const git = (...args) => {
    try {
      return execFileSync(
        'git',
        [
          '--no-replace-objects',
          '-c',
          'core.fsmonitor=false',
          '-c',
          'core.hooksPath=/dev/null',
          '-c',
          'credential.helper=',
          '-c',
          'core.askPass=',
          ...args,
        ],
        {
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
        },
      ).trim();
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
    const snapshots = new Map();
    let contentCount = 0;
    for (const member of selection.members) {
      const review = await readPublicContent(api, repository, member.number);
      const pr = review.pr;
      requireValue(pr.head.sha === member.head, 'Source PR changed during this run.');
      requireDisclosure(review);
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
      snapshots.set(member.number, review.digest);
      const texts = review.texts;
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
    for (const [number, before] of snapshots) {
      const current = await readPublicContent(api, repository, number);
      requireDisclosure(current);
      requireValue(
        current.digest === before,
        'Public content changed during the scan; review current content and rerun checks.',
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
      `Publication check passed for ${selection.members.length} source PR(s) and ${contentCount} public text item(s). Explicit disclosure review attestation verified; attachments and semantics remain the reviewer’s responsibility.`,
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
