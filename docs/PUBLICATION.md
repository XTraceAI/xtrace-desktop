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
outside examples or HTML containers. Prepare and review the final PR description
locally, then publish that exact description with the snapshot set to `pending`.
Wait for discussion to settle, then run:

```sh
pnpm publication:check --repository OWNER/REPO --pr NUMBER --snapshot --body /path/to/pr-body.md
```

This read-only command prints a SHA-256 snapshot line. Replace the placeholder
with that line and check the disclosure checkbox after reviewing the content.
Update only those two controls in the published body. A local `--body` file must
match the already-published prose; changing prose requires publishing it first
and preparing another snapshot. This two-step process binds GitHub-assigned
revision identities without making the snapshot hash itself.

The digest covers the current source head, exact PR prose, linked issue
relationships and text, and discussion/review content, including attachment
links. Only the source PR’s review checkbox and snapshot values are normalized to avoid
hashing their own values. Linked issue and PR bodies are hashed verbatim, including
their disclosure controls. The digest establishes which content was attested;
it does not establish semantic approval or inspect attachment bytes. Version 2
snapshots also bind retained source PR body-edit identities and title-change
events. Existing version 1 snapshots must be refreshed once after upgrading.

Force-push timeline events bind both former and replacement head identities.
The local checker and advisory workflow fetch retained source heads into owned
temporary refs and scan their reachable history without checking out their code.
The refs are removed after success or failure. Current Git refs alone cannot
establish this retained-history coverage; use the metadata-aware checker.

Commit comments and attachment links on retained head ancestry remain included
for source and linked PRs even after those commits leave the current PR list.
This conservatively includes ancestor comments. Inaccessible history fails closed.
The reader supports up to 100 distinct retained heads, 1,000 ancestors per head
and 10,000 distinct retained commits, with bounded timeline pagination. Provider
omissions cannot establish that inaccessible history is clean.

Submitted reviews bind GraphQL `updatedAt`, `lastEditedAt` and retained edit IDs/text,
including edit-and-revert pairs with unchanged submission timestamps. Source and
linked-PR reviews use the same reader. Missing or inconsistent history fails
closed; more than 100 retained edits on one review also fails closed. Deleted
versions contribute opaque deletion identities. API omissions still limit history
coverage.

Comments from people and bots are included. The gate writes check results only,
so its own operation does not change the discussion or create an attestation
loop. Wait for review comments to settle before preparing the final snapshot.
Manual Development-sidebar issue links are included through GitHub's API.
Source PR body history is read through `userContentEdits`, and title changes
through `RenamedTitleEvent` timeline records. Adjacent body revisions that differ
only in the source disclosure controls share one content identity; an intervening
prose edit followed by a revert remains a distinct revision. Retained body and
title text is also scanned. Deleted body revisions retain an opaque deletion
marker. Missing, inconsistent, or truncated API history fails validation. GitHub
may omit or coalesce history; this is not proof of a complete audit trail.

Linked issue/PR update timestamps and available discussion revision timestamps
are required and hashed, so an edit followed by restored text invalidates the old
snapshot when GitHub advances that timestamp. API timestamp precision and fields
limit this signal; it does not detect every same-timestamp revision or review edit,
and it does not replace inspection of earlier GitHub edits.
Review-comment diff hunks are hashed and scanned as raw text, including retained
context from commits no longer reachable from the current PR. Missing diff context
fails the metadata check; current source alone cannot certify an outdated comment.
Markdown links and HTML anchor links are included. GitHub issue shorthand accepts
`#123` and case-insensitive `GH-123`, including punctuation around a reference.
Issue-shaped tokens in code examples, HTML comments, ordinary URL paths and
external link labels are excluded. Fetched
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
annotated tag messages/tagger metadata, Git ref names, filenames and tracked working files.
Direct blob ref targets are scanned too; direct tree refs and unsupported target
types fail closed without traversing their names.
Supply the actual PR base/head range with `--diff`; both refs must be present locally.
Add `--content /path/to/outbound.md` to scan a prepared outbound text file.
`--diff` and `--content` can be repeated for multiple inputs. These scans do not
inspect image contents, image metadata or GitHub edit history.

The scan fails if it exceeds 20,000 commits, 50,000 prepared inputs, 32 MiB per
input or 256 MiB in total. Inputs include commit metadata, ref names, unique Git paths, file versions, diffs
and supplied outbound text. Reaching a limit is an incomplete scan, not a clean
result; unsupported repository entries, including submodules, also fail.

Git LFS pointer blobs are rejected with an unsupported-object error, including
historical versions and direct blob refs. Detection includes leading whitespace,
blank records, extension records before the version, and legacy version aliases
accepted by the [Git LFS parser](https://github.com/git-lfs/git-lfs/blob/v3.4.1/lfs/pointer.go).
Their external bytes are not available
through ordinary Git blob scanning. Verified LFS object retrieval and scanning
must be implemented before LFS-backed content can pass this checker.

The scanner exits 0 for no detected findings, 1 for findings and 2 for a tooling,
configuration or Git error. Either nonzero result prevents a successful gate.
Diagnostics identify only the detector rule, line number and an opaque source
label (for example, `git-blob[12]`). They omit original paths, matched values and
source lines. Original paths remain inside private temporary scanner inputs so
filename-dependent rules still apply. Fixture directories are not exempt from
scanning, and a value deleted from the current tree can still be detected in history.

## CI boundary

Manual disclosure review is the publication gate. CI’s `security` job supplies
candidate source/test diagnostics, and `publication-content-advisory` supplies
current-content diagnostics from reviewed default-branch code. Neither a check
name nor the GitHub Actions app identity proves the producer: a candidate workflow
can emit an automatic job check with the same name and app. The custom
`external_id` protects worker ownership internally but repository rules do not
validate it. Do not require these contexts as a disclosure security boundary or
use their green status to authorize a merge.

Before each serialized manual merge, review the exact source/base and workflow
provenance, run the local disclosure check from reviewed code against current
public content, and inspect the result and attestation. Automated enforcement is
deferred until a separately trusted publisher (such as a dedicated GitHub App)
and live same-name spoofing acceptance demonstrate an unforgeable required
identity. No App credential or repository setting is installed by this change.

Candidate scripts and runtime dependencies receive no repository API token,
and their jobs have no issue or pull-request metadata permissions. The pinned
checkout uses read-only contents access without persisting credentials. Metadata
reads remain confined to reviewed default-branch diagnostic jobs (or the explicitly
reviewed CI policy bootstrap described in [CI.md](CI.md)). Failed,
cancelled or skipped dependencies cannot produce a successful aggregate.

Tests cover the credential boundary, metadata/queue-resolution helpers and real
CLI error paths without live credentials. They do not demonstrate a successful
live trusted workflow or merge-queue run.

To check an existing PR locally, use a checkout at that PR's source head and an
already configured read-only `GH_TOKEN` or `GITHUB_TOKEN`. Only run this metadata
CLI from a checkout whose scripts and dependencies you have reviewed and trust:
read-only tokens can still disclose private repository content. Never supply a
token to unreviewed candidate code. Replace the repository and number placeholders:

```sh
pnpm publication:check --repository OWNER/REPO --pr NUMBER
```

This reads current GitHub metadata and fetches source refs locally. CI always
uses its actual event payload; the manual PR override is unavailable there.
Fetched refs are invocation-specific and removed after success, rejection or fetch
failure. Existing refs created by other work remain in the whole-history scan.

The `Publication content advisory` workflow rechecks open PRs and current default-
branch merge-queue heads after PR/issue edits and conversation comments. PR
lifecycle and text changes signal it through completion of `CI` and `PR metadata`.
The scan matrix runs on the default branch, so unrelated PR failures stay in
that background run. Each PR receives its own `publication-content-advisory`
check tied to its source head. Manual dispatches must select the default branch.
Review and review-comment events run a dedicated `Publication review signal`
workflow; its completion signals the trusted workflow independently of candidate
tests.
The relay contains no checkout, actions, script dependencies or token permissions.
GitHub runs review workflows from the PR merge commit, as described in its
[event trust model](https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target). Runs reconcile current API state, rather than
trusting an old event's PR content, and reject changes observed during scanning.
Current queue heads and enumerated PRs receive completed failure checks before
scan jobs are scheduled. A scan can replace its own failure with success only
after current source, content and trusted-code identities pass validation.
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
processes. Normal completion and handled errors remove the temporary ref. A hard timeout
discards the isolated job checkout rather than sharing its temporary refs. Pre-existing reachable refs
remain in the whole-history scan, so an existing repository leak still blocks.
Queue certification remains disabled: queue heads retain failure until trusted
combined-tree scanning and live queue acceptance are implemented. Scanning
individual PRs does not certify their combined result.

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

The preparation job enumerates up to 256 open PRs and emits one matrix job per
PR. Every created `publication-content-advisory` check starts as a completed failure,
labelled as an unfinished scan. Successful validation replaces only that run's
matching head/check identity with success. Setup failure, cancellation, timeout
or a failed result update leaves a terminal failure result. The check can
therefore appear red while a scan is running; this is deliberate.

Each matrix job gets an isolated checkout of the preparation job's trusted
source SHA, with at most four jobs active and `fail-fast: false`. A separate
supervisor terminates the entire scanner process group after four minutes,
including synchronous Git/scanner descendants. The job has a ten-minute limit
including setup. A slow or failed early PR cannot cancel later jobs or consume
their scan budgets. No shared per-run scan deadline or retry cursor selects only
a prefix of PRs. Default-branch advances invalidate old-policy workers; source,
base and disclosure content are reread before success. Jobs never consume
candidate artifacts or share fetched candidate refs.

Above 256 open PRs, preparation fails after creating failure results for the
enumerated PRs; it never silently truncates the matrix. The existing bounded
API pagination limit still applies. GitHub event delivery, runner availability
and successful API writes remain prerequisites: an API outage cannot guarantee
invalidation of checks whose creation was never reached. Rerun reconciliation
and verify current results immediately before a merge or publication decision.

Regression tests run multiple real blocking child processes beyond one scan
budget and prove later jobs still receive an attempt. They also verify descendant
termination, no delayed write, terminal failure without a worker, failed result
updates, run/head/context ownership, matrix overflow and trusted-code advances.
These local tests do not claim live GitHub matrix scheduling evidence; activation
must exercise that behavior on the installed default-branch workflow.

Install the dedicated relay first in a small, maintainer-reviewed prerequisite
change on the default branch. Then update the publication-gate branch so its
actual base and head both include that relay, and review/install the remaining
trusted workflow. The initial gate cannot certify a base lacking the relay;
stacking the gate on the prerequisite makes this dependency reviewable but does
not activate trusted default-branch execution. Follow [the activation sequence](PUBLICATION_ACTIVATION.md); keep activation
evidence pending until reviewed changes land. Queue activation remains separately
deferred until a public launch and its required combined-tree checks.

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

Before writing success, the worker rereads the complete disclosure digest after
checking trusted-code identity, including titles, discussions, linked records
and retained revisions. It compares that digest with the scanned snapshot and
performs no intervening API work before the check update. GitHub content reads
and check writes are separate requests: edits during or after that final read
still depend on subsequent reconciliation, including workflow scheduling delay.
Success records the observed snapshot; it cannot lock public content against edits.

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
review above. Review changes to the scanner and workflows themselves. CI includes source
tests and scanning in `ci-ok`; manual current-content review remains required
because metadata edits cannot be authorized by old CI.

Public documentation uses descriptive feature names and observable acceptance
criteria. Keep private roadmap identifiers, scheduling maps, conversation
excerpts and internal review logs outside the repository. Review filenames,
source comments, command help, PR and issue text, branch names and retained
history as well as document bodies. Renaming or deleting current files does not
remove previous commits or hosted revisions.

These checks do not publish content, change repository visibility or merge a PR.
They cover disclosure prevention; build/test CI, dependency-license inventory and DCO validation are described
in [CI.md](CI.md). Release packaging needs separate acceptance.
