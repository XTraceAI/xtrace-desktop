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
4. Complete the PR template's disclosure checkbox only after reviewing its final
   text, linked issues, comments and attachments. Repeat this review whenever
   those materials change, including after a previous check passed.

People or agents must perform the semantic review: a pattern scanner cannot
understand whether a conversation is private or whether its inclusion is
appropriate. An attestation records that review; it is not automated proof of it.

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
`security:scan` (also available as `secrets:check`) checks reachable Git history
and tracked working files. Supply
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

The `publication-checks` workflow check combines its synthetic tests with a scan
for pull requests and merge-queue groups. It checks actual PR diffs, reachable
history, current PR and linked-issue text, and the checked disclosure attestation.
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

Comments, attachments and earlier GitHub edits require the separate review above.
Review changes to the scanner and workflow themselves; a candidate branch can
change its check implementation. Requiring the check through repository rules
is a separate maintainer setting.

These checks do not publish content, change repository visibility or merge a PR.
They cover disclosure prevention; the broader build/test CI, dependency-license
inventory, DCO automation and release checks remain separate work.
