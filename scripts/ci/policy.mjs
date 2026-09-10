import { execFileSync } from 'node:child_process';

export class CheckError extends Error {}

// Only final Git trailers certify a commit. A quote or example in its body
// must not count as a contributor's certification.
export function validateDco(commit) {
  if (!commit || !/^[a-f0-9]{40}$/.test(commit.sha ?? ''))
    throw new CheckError('Commit metadata is missing or invalid.');
  const { name, email } = commit.author ?? {};
  if (typeof name !== 'string' || !name.trim() || typeof email !== 'string' || !email.trim())
    throw new CheckError(`Author metadata is missing for ${commit.sha}.`);
  if (typeof commit.message !== 'string')
    throw new CheckError(`Commit message is missing for ${commit.sha}.`);
  const finalParagraph = commit.message
    .trimEnd()
    .split(/\r?\n\s*\r?\n/)
    .at(-1);
  if (/^ {0,3}(?:`{3,}|~{3,})/m.test(finalParagraph))
    throw new CheckError(`DCO sign-off is inside an example for ${commit.sha}.`);
  let trailers;
  try {
    trailers = execFileSync('git', ['interpret-trailers', '--parse'], {
      input: commit.message,
      encoding: 'utf8',
      timeout: 10_000,
      stdio: ['pipe', 'pipe', 'pipe'],
    }).split(/\r?\n/);
  } catch {
    throw new CheckError(`DCO trailers could not be parsed for ${commit.sha}.`);
  }
  const signed = trailers.some((line) => {
    const match = line.match(/^Signed-off-by:\s+(.+?)\s+<([^<>\s]+)>\s*$/i);
    return (
      match && match[1].trim() === name.trim() && match[2].toLowerCase() === email.toLowerCase()
    );
  });
  if (!signed) throw new CheckError(`Author-matching DCO sign-off is missing for ${commit.sha}.`);
}

export function requireSuccessfulJobs(results, required, inapplicable = []) {
  if (
    !Array.isArray(required) ||
    required.length === 0 ||
    new Set(required).size !== required.length
  )
    throw new CheckError('Required job policy is invalid.');
  if (inapplicable.some((name) => !required.includes(name)))
    throw new CheckError('Inapplicable job is outside the required job policy.');
  for (const name of required) {
    const result = results?.[name]?.result;
    if (result === 'success') continue;
    if (result === 'skipped' && inapplicable.includes(name)) continue;
    throw new CheckError(`Required job ${name} did not succeed.`);
  }
}
