import { CheckError } from './policy.mjs';

const checkpoints = ['CP1', 'CP3', 'CP5', 'CP7'];
const sha = (value) => typeof value === 'string' && /^[a-f0-9]{40}$/.test(value);
const digest = (value) => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);

export function requiredCheckpoints(stage, intent = 'change') {
  if (!Number.isInteger(stage) || stage < 1 || stage > 15)
    throw new CheckError('Unknown implementation stage.');
  if (!['change', 'publication'].includes(intent)) throw new CheckError('Unknown CI intent.');
  return checkpoints.filter(
    (name, index) => stage > [2, 6, 11, 14][index] || (name === 'CP7' && intent === 'publication'),
  );
}

// Approval is an explicit JSON comment by a configured person. A quoted example,
// label, bot review or timeout is never approval. The issue describes the current
// evidence identity; changing it invalidates comments for an older identity.
export function checkpointEvidence(issue, comments, approvers, checkpoint, intent = 'change') {
  if (
    !issue ||
    issue.pull_request ||
    issue.state !== 'open' ||
    !Array.isArray(comments) ||
    !Array.isArray(approvers) ||
    !approvers.length ||
    !checkpoints.includes(checkpoint)
  )
    throw new CheckError('Checkpoint evidence or approver configuration is unavailable.');
  let evidence;
  try {
    evidence = JSON.parse(issue.body);
  } catch {
    throw new CheckError('Checkpoint issue must identify current evidence as JSON.');
  }
  if (
    evidence.checkpoint !== checkpoint ||
    !sha(evidence.source) ||
    !Array.isArray(evidence.artifacts) ||
    !evidence.artifacts.every(digest) ||
    new Set(evidence.artifacts).size !== evidence.artifacts.length ||
    !digest(evidence.decisions) ||
    (intent === 'publication' && !evidence.artifacts.length)
  )
    throw new CheckError('Checkpoint source, artifact or decision identity is invalid.');
  const allowed = new Set(approvers.map((name) => name.toLowerCase()));
  const eligible = comments
    .filter(
      (comment) => comment.user?.type === 'User' && allowed.has(comment.user?.login?.toLowerCase()),
    )
    .sort((a, b) => b.id - a.id);
  let approval;
  for (const comment of eligible) {
    let value;
    try {
      value = JSON.parse(comment.body);
    } catch {
      continue;
    }
    if (value.checkpoint !== checkpoint) continue;
    // The latest explicit decision wins, including rejection or revocation.
    approval = value;
    break;
  }
  if (
    !approval ||
    approval.status !== 'approved' ||
    approval.source !== evidence.source ||
    approval.decisions !== evidence.decisions ||
    JSON.stringify(approval.artifacts) !== JSON.stringify(evidence.artifacts)
  )
    throw new CheckError('Current explicit checkpoint approval is missing or stale.');
  return evidence;
}

export function checkpointFix(comments, approvers, checkpoint, head, intent = 'change') {
  if (intent !== 'change' || !sha(head) || !Array.isArray(approvers)) return false;
  const allowed = new Set(approvers.map((name) => name.toLowerCase()));
  for (const comment of [...comments].sort((a, b) => b.id - a.id)) {
    if (comment.user?.type !== 'User' || !allowed.has(comment.user?.login?.toLowerCase())) continue;
    let value;
    try {
      value = JSON.parse(comment.body);
    } catch {
      continue;
    }
    if (value.checkpoint !== checkpoint || value.kind !== 'checkpoint-fix') continue;
    return value.status === 'approved' && value.source === head;
  }
  return false;
}
