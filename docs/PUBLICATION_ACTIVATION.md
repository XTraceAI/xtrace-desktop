# Publication diagnostics and manual merge review

Disclosure review is manual. Automatic publication scans and their review-event
relay are disabled. Existing advisory checks are historical observations and do
not invalidate themselves when comments, relationships or source change.

Before changing the repository to public, and again before the first
downloadable release:

1. Review the exact current source/base, source history and outbound text,
   discussions, linked records, attachments and retained revisions.
2. Review the local checker's scripts, imports and dependencies before giving it
   a read-only metadata token. Candidate code must not receive that token unless
   it has been separately reviewed and trusted.
3. Run the local publication checker from a clean full clone. See
   [PUBLICATION.md](PUBLICATION.md). Repeat after content or source changes.
   Record native validation separately from the Linux CI result.
4. Verify DCO, current CI provenance and all unresolved findings. Preserve the
   existing squash-only procedure, deletion/force-push prevention and bypass policy.

An optional `Publication content advisory` audit can be dispatched manually on the
default branch. It scans all open PRs using reviewed code and reports each result
on the relevant PR. Other branches and tags cannot run the audit jobs. Before a
first manual use, enable that workflow if it was disabled during migration from
automatic scans. Do not enable a legacy review-signal workflow. No audit starts
merely by enabling the manual-only workflow.

During migration, pause the old `publication-content.yml` and
`publication-review.yml` workflows before further PR activity, and rely on the
current local disclosure check. After the manual-only workflow is merged, it can
be enabled for explicit audits. Existing run/check history remains visible.
This procedure does not waive source tests, DCO or the final publication review.

Automatic enforcement remains deferred. A future change needs an independently
trusted publisher identity, demonstrated edit/source invalidation and fail-closed
behavior. Merely pinning the shared Actions app or renaming a check is insufficient.
Do not require `publication-content-advisory` as proof of disclosure approval.

Merge queues and public visibility remain separately authorized work. Synthetic
regressions do not establish live queue readiness. No App credential, required
rule or public release is installed by these documents.
