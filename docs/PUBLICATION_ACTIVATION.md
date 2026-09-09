# Publication diagnostics and manual merge review

Publication enforcement remains manual. Automated checks are advisory evidence,
including results emitted by reviewed default-branch code. Candidate workflows
can create automatic job checks with the same name and GitHub Actions app identity;
repository status rules do not validate a custom check's `external_id`. Do not
require `publication-content`, `publication-content-advisory` or `publication-checks`
as proof of trusted disclosure approval.

1. Install the reviewed immutable `publication-review.yml` relay prerequisite.
   Verify its regular-file blob on the actual default branch.
2. Retarget the publication PR to the updated default branch and validate the
   combined result. Both source and base must contain the unchanged relay.
   Review the exact scripts and dependencies before providing the local checker
   with a read-only metadata token. Run the current local disclosure check and
   inspect source, history, PR text, discussions, linked records and attachments
   before the maintainer performs the bootstrap merge.
3. After installation, dispatch `Publication content advisory` on the default
   branch. Record its source SHA, matrix jobs and advisory check IDs. Exercise an
   early timeout with later PRs present: later jobs must run, the failed scan must
   remain failed, and an unchanged reviewed PR must pass. Verify issue/review
   events and scheduled reconciliation refresh diagnostic results. These tests
   validate diagnostics, not merge authorization.
4. For every subsequent merge, serialize the decision: verify the actual current
   head/base, review workflow provenance and current public content, refresh the
   disclosure snapshot, and run the local checker from reviewed code. Inspect its
   result immediately before merging. If content or source changes, repeat the
   review. A green Actions check alone is insufficient. Preserve existing
   squash-only merges, deletion/force-push prevention and bypass policy.
5. Review the contribution validator at an exact source SHA and follow the
   bootstrap procedure in [CI.md](CI.md). Its build and DCO diagnostics do not
   replace manual disclosure review.

Automated disclosure enforcement is deferred. A future, separately reviewed
change must provide a publisher identity that candidate workflows cannot obtain,
such as a dedicated GitHub App, and pin that identity in the required rule. A
required trusted workflow is another option where supported, but its metadata
invalidation semantics must also be demonstrated. Merely renaming a job or
pinning the shared GitHub Actions app does not solve producer spoofing.

Before enabling enforcement, demonstrate a malicious candidate job with the same
check name and GitHub Actions identity: its delayed success must not satisfy the
required disclosure rule when the trusted result is failed or absent. Also test
credential isolation, edits after success, source/base advances, cancellation,
missing results and rollback. Until those live cases pass, maintain manual review.
No App setup, ruleset change, visibility change or merge is authorized here.

Do not activate a merge queue in this sequence. Trusted combined-tree scanning
and live queue acceptance remain deferred. Synthetic regression tests alone do
not establish that the live queue is ready.

Before any future settings change, reread the current ruleset, preserve unrelated
rules and save a rollback request. Stop merges if trustworthy evidence is missing.
GitHub documents [expected status-check sources](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches#require-status-checks-before-merging)
and [required workflows](https://docs.github.com/en/enterprise-cloud@latest/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets).
