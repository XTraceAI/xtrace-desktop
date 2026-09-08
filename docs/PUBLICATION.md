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
links. Only the review checkbox and snapshot values are normalized to avoid
hashing their own values. The digest establishes which content was attested;
it does not establish semantic approval or inspect attachment bytes.

Comments from people and bots are included. The gate writes check results only,
so its own operation does not change the discussion or create an attestation
loop. Wait for review comments to settle before preparing the final snapshot.
Manual Development-sidebar issue links are included through GitHub's API.
Issue-shaped tokens in code examples and HTML comments are excluded. Fetched
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
annotated tag messages/tagger metadata and tracked working files. Supply
the actual PR base/head range with `--diff`; both refs must be present locally.
Add `--content /path/to/outbound.md` to scan a prepared outbound text file.
`--diff` and `--content` can be repeated for multiple inputs. These scans do not
inspect image contents, image metadata or GitHub edit history.

The scan fails if it exceeds 20,000 commits, 50,000 prepared inputs, 32 MiB per
input or 256 MiB in total. Inputs include commit metadata, file versions, diffs
and supplied outbound text. Reaching a limit is an incomplete scan, not a clean
result; unsupported repository entries, including submodules, also fail.

The scanner exits 0 for no detected findings, 1 for findings and 2 for a tooling,
configuration or Git error. Either nonzero result prevents a successful gate.
Diagnostics must omit matched values and source lines. Fixture directories are
not exempt from scanning, and a value deleted from the current tree can still
be detected in history.

## CI boundary

Two checks form the publication gate: `publication-checks` covers source/tests
and `publication-content` covers the current disclosure snapshot. Require both
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
and review-comment events run the read-only Publication workflow; its completion
signals the trusted workflow. Runs reconcile current API state, rather than
trusting an old event's PR content, and reject changes observed during scanning.
They execute only the default branch, use `checks:write` solely to update the
named content check, and never run PR scripts or consume workflow artifacts.
Pending events can coalesce because each run rechecks every open PR and queue
head. A manual workflow dispatch retries an incomplete run.

The event workflow becomes active only after it lands on the default branch.
Live issue/comment-event delivery, check creation and queue success remain
integration checks until then; synthetic API-shaped tests cover their failure
paths. GitHub must deliver an event and start the run to invalidate an earlier
result. Repeat the review and update the snapshot when manually changing linked
relationships. No repository rule is changed by these files.

Comments on the source PR's commits are included, with complete source-commit
pagination required. New commit comments trigger reconciliation. GitHub exposes
only the `created` activity for `commit_comment`, so edits/deletions and manual
relationship changes also use a 15-minute scheduled reconciliation. Scheduled
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
