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
4. Before changing the repository to public, and again before publishing the
   first downloadable release, perform the comprehensive review described
   below. Ordinary solo-maintainer PRs do not require a disclosure checkbox or
   snapshot.

People or agents must perform the semantic review: a pattern scanner cannot
understand whether a conversation is private or whether its inclusion is
appropriate.

## Comprehensive review before publication

Run the following from a clean, full local clone immediately before changing
repository visibility and before the first downloadable release:

```sh
pnpm publication:test
pnpm security:scan
pnpm publication:check --repository OWNER/REPO --pr NUMBER
```

Use `publication:check` for the current release/publication PR or another PR whose
linked discussion represents the material being published. It reads the PR,
linked issues, comments, reviews and retained source revisions, then scans that
text and the relevant Git history without changing GitHub. Separately inspect the
repository's closed PRs/issues and every image or attachment that will become
public: GitHub and the scanner do not provide a complete repository-wide audit of
historical attachment bytes. Record the completed audit in the release or
visibility-change checklist. Per-PR attestations can be reconsidered if outside
contributors begin submitting work.

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

Comments from people and bots are included. The optional hosted advisory writes
check results only, so its own operation does not change the discussion.
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
issue references must belong to this repository; cross-repository disclosure
collection is unsupported. Ordinary external URLs remain prose.

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

The comprehensive local review is a publication/release gate, not a routine PR
merge requirement. CI’s `security` job supplies
candidate source/test diagnostics, and an explicitly dispatched `publication-content-advisory` supplies
current-content diagnostics from reviewed default-branch code. Neither a check
name nor the GitHub Actions app identity proves the producer: a candidate workflow
can emit an automatic job check with the same name and app. The custom
`external_id` protects worker ownership internally but repository rules do not
validate it. Do not require these contexts as a disclosure security boundary or
use their green status to authorize a merge.

Before changing repository visibility or publishing a downloadable release,
review the exact source and workflow provenance, run the local disclosure check
from reviewed code against current public-facing content, and inspect its result.
Automated enforcement is deferred; no publisher credential or repository setting
is installed by this procedure.

Candidate scripts and runtime dependencies receive no repository API token,
and their jobs have no issue or pull-request metadata permissions. The pinned
checkout uses read-only contents access without persisting credentials. Metadata
reads remain confined to explicit reviewed default-branch diagnostic jobs (or the explicitly
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

The `Publication content advisory` workflow is an optional, manually dispatched
repository-wide audit on the default branch. PR changes, issue/comment changes,
reviews, CI completion and schedules do not start it. The review-event relay and
its source-blob pin have been retired because automatic event delivery no longer
participates in disclosure validation. There is no automatic invalidation of old
hosted results. Run the current local check for the final publication/release
candidate; old green results cannot stand in for that check.

An explicit audit enumerates open PRs and queue heads. Each PR receives only its
own advisory check. Before scheduling work, the coordinator marks results failed
until that run completes the relevant scan. Setup failure, cancellation, timeout
or incomplete metadata cannot preserve a successful result for that audit.
The matrix has at most 256 PR jobs, runs four at a time and gives each worker an
independent four-minute budget. Queue certification remains disabled until live
combined-tree scanning is implemented and accepted.

The audit runs reviewed default-branch code. It fetches exact candidate and
retained source heads into temporary refs, verifies identities, and scans their
history and diffs without checking out or executing candidate code. Authentication
is transient, fixed to the validated repository URL and withheld from scanner
processes. Normal completion and handled failures remove temporary refs. Tests
verify that a candidate's replacement scanner cannot hide a source secret and
that mismatches and failed fetches do not leave refs or expose raw diagnostics.

Before writing success, the worker rereads source, content and trusted-code
identities. Separate API reads and writes are not atomic: edits after observation
require another local review or explicit audit. Success records observed content;
it does not lock it. Commit comments and manually edited relationships have the
same freshness requirement. Repositories with 1,000 or more commit comments exceed
the bounded reader and fail closed pending a paginated incremental design.

Follow [manual review and audit setup](PUBLICATION_ACTIVATION.md). Re-enabling
automatic disclosure enforcement or a merge queue requires separate design and
live acceptance; a shared Actions check identity remains insufficient.

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
