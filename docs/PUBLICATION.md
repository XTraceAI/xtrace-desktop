# Publication checks

Treat repository content, commit messages, issues, pull requests, comments and
attachments as public-facing even when the repository is private. Publish the
current technical scope and reproducible acceptance evidence. Keep private
conversations, agent session instructions, local planning cards, account-specific
operations and internal review history out of outbound material.

## Review before sending

1. Prepare the final text and files locally. Preserve the problem, resulting
   behavior, dependencies, test setup/actions/expected results and actual evidence.
   Write a public explanation; do not paste complete local source cards or agent
   conversations into an issue or PR.
2. Read the exact candidate, including linked issues and comments. Inspect logs,
   screenshots and other attachments for private content, paths, account details
   and embedded metadata. Use synthetic evidence where possible. Do not reproduce
   a detected secret or rejected private text in a review comment or report.
3. Run the local checks below and inspect their results. Review Git history as
   well as the current files, and inspect prior GitHub edits where available.
   Editing the latest text does not remove its earlier versions; GitHub provides
   a separate [comment edit-history view](https://docs.github.com/en/communities/moderating-comments-and-conversations/tracking-changes-in-a-comment).
4. After reviewing the final text, linked issues, comments and attachments,
   prepare the disclosure snapshot below and complete the template's checkbox.
   Repeat this review whenever those materials or the source commit change,
   including after a previous check passed.

People or agents must perform the semantic review: a pattern scanner cannot
understand whether a conversation is private or whether its inclusion is
appropriate. An attestation records that review; it is not automated proof of it.

## Keep the disclosure review current

Keep the checkbox and `Disclosure snapshot: pending` line in ordinary Markdown,
outside examples or HTML containers. Prepare the final PR description locally,
including that placeholder, then run:

```sh
pnpm publication:check --repository OWNER/REPO --pr NUMBER --snapshot --body /path/to/pr-body.md
```

This read-only command prints a SHA-256 snapshot line. Replace the placeholder
with that line and check the disclosure checkbox after reviewing the content.
The digest covers the current source head, exact PR prose, linked issue
relationships and text, and discussion/review content, including attachment
links. Only the source PR’s review checkbox and snapshot values are normalized to avoid
hashing their own values. Linked issue and PR bodies are hashed verbatim, including
their disclosure controls. The digest establishes which content was attested;
it does not establish semantic approval or inspect attachment bytes.

Comments from people and bots are included. The gate writes check results only,
so its own operation does not change the discussion or create an attestation
loop. Wait for review comments to settle before preparing the final snapshot.
Manual Development-sidebar issue links are included through GitHub's API.
Review-comment diff hunks are hashed and scanned as raw text, including retained
context from commits no longer reachable from the current PR. Missing diff context
fails the metadata check; current source alone cannot certify an outdated comment.
Markdown links and HTML anchor links are included. Issue-shaped tokens in code
examples and HTML comments are excluded. Fetched
issue references must belong to this repository: another repository's edits
cannot trigger local invalidation events. Ordinary external URLs remain prose.

## Local checks

From a checkout with the pinned Node and pnpm versions installed:

```sh
pnpm publication:test
pnpm security:scan
pnpm security:scan --diff origin/main..HEAD
```

Git and complete local history are required; shallow checkouts fail the scan.
The scanner downloads pinned Gitleaks 8.30.1 from its official GitHub release
on first use and verifies its checksum. Later runs can use the verified cache
offline. Download, checksum or execution errors fail the check.

`publication:test` uses synthetic cases to verify rejection and error handling.
`security:scan` (also available as `secrets:check`) checks reachable Git history,
annotated tag messages/tagger metadata, Git ref names, filenames and tracked working files. Supply
the actual PR base/head range with `--diff`; both refs must be present locally.
Add `--content /path/to/outbound.md` to scan a prepared outbound text file.
`--diff` and `--content` can be repeated for multiple inputs. These scans do not
inspect image contents, image metadata or GitHub edit history.

The scan fails if it exceeds 20,000 commits, 50,000 prepared inputs, 32 MiB per
input or 256 MiB in total. Inputs include commit metadata, ref names, unique Git paths, file versions, diffs
and supplied outbound text. Reaching a limit is an incomplete scan, not a clean
result; unsupported repository entries, including submodules, also fail.

The scanner exits 0 for no detected findings, 1 for findings and 2 for a tooling,
configuration or Git error. Either nonzero result prevents a successful gate.
Diagnostics identify only the detector rule, line number and an opaque source
label (for example, `git-blob[12]`). They omit original paths, matched values and
source lines. Original paths remain inside private temporary scanner inputs so
filename-dependent rules still apply. Fixture directories are not exempt from
scanning, and a value deleted from the current tree can still be detected in history.

## CI boundary

Two checks form the publication gate: `publication-checks` covers source/tests
and `publication-content` runs trusted candidate scanning plus current disclosure review. Require both
through repository rules after the workflows land. The former combines tests
with actual PR diffs, reachable history and current reviewed public content.
A failed, cancelled or skipped dependency cannot produce a successful aggregate.
The event-aware `pnpm publication:check` entrypoint runs in the GitHub Actions
context. Its tests cover metadata/queue-resolution helpers and real CLI error
paths without live credentials. They do not demonstrate a successful live PR
or merge-queue run.

To check an existing PR locally, use a checkout at that PR's source head and an
already configured read-only `GH_TOKEN` or `GITHUB_TOKEN`. Replace the repository
and number placeholders:

```sh
pnpm publication:check --repository OWNER/REPO --pr NUMBER
```

This reads current GitHub metadata and fetches source refs locally. CI always
uses its actual event payload; the manual PR override is unavailable there.

The trusted `publication-content` workflow rechecks open PRs and current default-
branch merge-queue heads after PR/issue edits and conversation comments. Review
and review-comment events run a dedicated `Publication review signal` workflow;
its completion signals the trusted workflow independently of candidate tests.
The relay contains no checkout, actions, script dependencies or token permissions.
GitHub runs review workflows from the PR merge commit, as described in its
[event trust model](https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target). Runs reconcile current API state, rather than
trusting an old event's PR content, and reject changes observed during scanning.
Current queue heads receive blocking pending checks before sequential PR scans;
a later read or scan failure cannot preserve an earlier successful queue result.
They execute only the default branch, use `checks:write` solely to update the
named content check, and never run PR scripts or consume workflow artifacts.
The trusted scanner fetches each exact source PR head into a temporary Git ref,
verifies its SHA and scans its base/head range without checking out candidate
files or importing candidate scanner code. Before scanning, it requires both
actual base and source commits to contain the trusted relay’s exact Git blob and
regular-file mode. This rejects deletion, renaming, trigger changes and conditional
job changes; checking trigger names alone would not protect event delivery.
Both base and head must remain unchanged through the final success check. Fetch authentication is transient and
restricted to the validated repository's GitHub URL; it is not passed to scanner
processes. Every outcome removes the temporary ref. Pre-existing reachable refs
remain in the whole-history scan, so an existing repository leak still blocks.
Queue results require the current identities of those scanned source PRs.

Disposable Git regressions verify that replacing the candidate scanner with a
success stub cannot hide a source secret, that the candidate code is not executed,
and that failure/head mismatch removes temporary refs. Relay regressions alter
real base/source Git commits, remove events, add a false job condition, rename or
symlink the file; none may reach scanning or certification. Synthetic transport tests
inspect credential handling; they do not establish live default-branch workflow
delivery. Separate empty-file tests detect secrets found only in staged, current
and removed historical Git names, without printing the name.

Pending events can coalesce because each run rechecks every open PR and queue
head. A manual workflow dispatch retries an incomplete run.

Install the dedicated relay first in a small, maintainer-reviewed prerequisite
change on the default branch. Then update the publication-gate branch so its
actual base and head both include that relay, and review/install the remaining
trusted workflow. The initial gate cannot certify a base lacking the relay;
stacking the gate on the prerequisite makes this dependency reviewable but does
not activate trusted default-branch execution. Keep activation and queue evidence
pending until those reviewed changes actually land.

The relay is immutable while it is pinned. To change it later, first add a
separately named replacement while preserving the current relay. A subsequent
maintainer-reviewed trusted-code change can switch to that installed replacement;
retire the old relay only after the switch and branch synchronization. Direct
changes to the currently pinned relay remain blocked. No automatic exception or
repository-setting change is provided.

The event workflow becomes active only after it lands on the default branch.
Live issue/comment-event delivery, check creation and queue success remain
integration checks until then; synthetic API-shaped tests cover their failure
paths. GitHub must deliver an event and start the run to invalidate an earlier
result. Repeat the review and update the snapshot when manually changing linked
relationships. No repository rule is changed by these files.

Comments on the source PR's commits are included, with complete source-commit
pagination required. GitHub Actions does not support a commit-comment trigger;
the [documented workflow triggers](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows)
are a subset of webhook events. Commit-comment creation, edits/deletions and manual
relationship changes use a 15-minute scheduled reconciliation. Scheduled
runs can be delayed by GitHub; this is eventual detection, not an instantaneous
publication barrier. Run the local current-content check immediately before a
publication decision. Repositories with 1,000 or more commit comments exceed the
bounded API reader and fail closed until a paginated incremental design is added.

Attachment bytes, embedded metadata and earlier GitHub edits require the separate
review above. Review changes to the scanner and workflows themselves. Future
FND-02 CI may fold the immutable tests/scanning into `ci-ok`; the separate current-
content check must remain required so metadata edits cannot reuse stale CI.

These checks do not publish content, change repository visibility or merge a PR.
They cover disclosure prevention; the broader build/test CI, dependency-license
inventory, DCO automation and release checks remain separate work.
