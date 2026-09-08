# Publication check activation

The repository uses reviewed, serialized squash merging while private.
This runbook prepares activation; it does not change settings or authorize merges.

1. Review and merge the immutable `publication-review.yml` relay prerequisite.
   Verify that the actual default branch contains its reviewed regular-file blob.
2. Retarget the publication checks PR to that updated default branch, validate
   the exact combined result and complete its review. The base and source must
   both contain the unchanged relay. Merge the trusted content workflow only
   after these checks pass. Until the trusted producer is installed, the
   candidate `publication-checks` result covers Git source and tests only; run the
   reviewed local disclosure check and inspect its evidence before this bootstrap
   merge. Do not treat candidate CI as metadata attestation. Queue certification
   remains disabled.
3. Dispatch `Publication content` on the default branch. Record its source SHA,
   matrix jobs and custom check IDs. Exercise an early scan timeout with later
   PRs present: later jobs must run, the failed scan must remain terminally
   failed, and an unchanged reviewed PR must pass. Verify issue edits, review
   comments and scheduled reconciliation invalidate and recheck current content.
4. Require `publication-checks` and `publication-content` on the default branch
   only after their actual installed producers report. Pin the expected GitHub
   Actions app identity. Keep squash-only PRs, deletion/force-push prevention
   and existing bypass policy. A failed check must block an actual test PR.
5. Review the FND-02 stage map at an exact immutable source SHA, configure
   `CI_POLICY_SHA` to that reviewed SHA while it is absent from the default
   branch, and rerun its CI. This does not approve any later-stage checkpoint.
   Review and merge FND-02 when its requirements pass. After `ci-ok` is installed
   and demonstrated, replace the temporary `publication-checks` requirement with
   `ci-ok`, keeping `publication-content` separately required. Remove the bootstrap
   variable only once the default branch contains the reviewed stage map.

Manual merge review must revalidate the exact current head/base result and
current disclosure/checkpoint evidence. The status-check rule may remain loose
(`strict_required_status_checks_policy: false`) because this procedure supplies
combined-result review; it is not an automated guarantee that a base advance was
retested. CI status names or an app identity alone do not prove workflow source
trust: also review the workflow and its execution source.

Do not activate a merge queue as part of this sequence. A separately approved
public launch, trusted combined-tree scan and live queue acceptance are required.
Configure checkpoint issue/approver identities when the relevant demonstration
is ready; no synthetic approval or permissive fallback may unblock a later stage.

Before applying settings, reread the current ruleset and preserve unrelated
rules. Store the reviewed request and prior configuration for rollback. Rollback
requires maintainer review; stop merges during a broken required-check producer
instead of treating a timeout or missing result as approval.

GitHub documents [matrix failure isolation and parallelism](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
and [ruleset status-check configuration](https://docs.github.com/en/rest/repos/rules).
