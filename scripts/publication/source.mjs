import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { cleanEnvironment } from '../security/gitleaks.mjs';
import { prNumber, PublicationError, repositoryName, requireValue, sha } from './metadata.mjs';

// Fetch objects into the trusted checkout's object database, never its worktree
// or index. The temporary ref is removed even when fetching/scanning fails.
export async function withCandidateSource(
  repo,
  source,
  scan,
  { run = spawnSync, token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN } = {},
) {
  const repository = repositoryName(source?.repository);
  const number = prNumber(source?.number);
  const head = sha(source?.head);
  const base = sha(source?.base);
  requireValue(
    typeof token === 'string' && token.length > 0,
    'Candidate source fetch requires a scoped read token.',
  );
  const ref = 'refs/publication-content/' + randomUUID();
  const relayPath = '.github/workflows/publication-review.yml';
  const git = (args, authenticate = false) => {
    const env = {
      ...cleanEnvironment(),
      GIT_CONFIG_NOSYSTEM: '1',
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_TERMINAL_PROMPT: '0',
    };
    if (authenticate)
      Object.assign(env, {
        GIT_CONFIG_COUNT: '1',
        GIT_CONFIG_KEY_0: `http.https://github.com/${repository}.git.extraheader`,
        GIT_CONFIG_VALUE_0: `AUTHORIZATION: basic ${Buffer.from(`x-access-token:${token}`).toString('base64')}`,
      });
    const result = run(
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
        '-c',
        'http.followRedirects=false',
        ...args,
      ],
      {
        cwd: repo,
        env,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
        timeout: 120_000,
        maxBuffer: 1024 * 1024,
      },
    );
    if (result.error || result.signal || result.status !== 0)
      throw new PublicationError('Candidate source objects could not be fetched or verified.');
    return result.stdout.trim();
  };
  try {
    git(
      [
        'fetch',
        '--no-tags',
        '--no-recurse-submodules',
        '--no-write-fetch-head',
        `https://github.com/${repository}.git`,
        `+refs/pull/${number}/head:${ref}`,
      ],
      true,
    );
    requireValue(
      git(['rev-parse', '--verify', ref]) === head,
      'Candidate source changed while its objects were fetched.',
    );
    git(['cat-file', '-e', `${base}^{commit}`]);
    const relay = (revision) => {
      const entry = /^100644 blob ([a-f0-9]{40})\t(.+)$/.exec(
        git(['ls-tree', revision, '--', relayPath]),
      );
      requireValue(
        entry?.[2] === relayPath,
        'The trusted review relay must exist as a regular file in the base and source commits.',
      );
      return entry[1];
    };
    const trustedRelay = relay('HEAD');
    // Review events execute the merge-tree workflow: both inputs must preserve
    // the trusted relay, including its jobs and conditions, byte for byte.
    requireValue(
      relay(base) === trustedRelay && relay(head) === trustedRelay,
      'Source or base changed the trusted review relay; install an approved replacement before updating this gate.',
    );
    return await scan({ base, head });
  } finally {
    git(['update-ref', '-d', ref]);
  }
}
