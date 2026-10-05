# Native PR witnesses

A Claude transcript can hold an explicit `pr-link` line naming the pull request a
session opened or referenced. The native Claude scan now consumes these lines,
which it previously ignored. Each one becomes an exact link through
[`xt_store::pr_link`](storage.md#pull-request-evidence): one content-free
`pull_requests` stub per canonical pull request and one `pr_links` row per
session and pull request. Nothing else is claimed.

## Contract

- **Exact only.** A witness is `exact` evidence. The pull request must reconcile
  to one canonical identity from the line's `prUrl`, `prRepository` and
  `prNumber` through `PrIdentity::reconcile`. ASCII case differences in the owner
  or repository reconcile. Any other disagreement, an unsupported host, an alias
  spelling or a malformed number stops the file. No SHA matching, inference, Git,
  GitHub, network, subprocess or refresh runs, and no `gh` command is paired with
  its output.
- **Its own time.** `first_seen_at` and `last_seen_at` are both the witness's own
  `timestamp` in UTC milliseconds. The scan's `observed_at` is never used as an
  event time. A missing or invalid timestamp stops the file.
- **Identity like a record.** A witness is reconciled by the rules that already
  apply to records and hook summaries. Its conversation, native session,
  platform, surface, start instant and source must agree with the file and the
  records around it. Its two native-session spellings must agree, and a blank
  label is no label. Unlike a record, a witness must name its session: without
  `sessionId` nothing says whose evidence it is. A disagreement stops the file
  before the batch covering the witness commits. No record, stub, link or
  checkpoint from that batch commits.
- **Atomic with its batch.** The writer's `WriteBatch::pr_witnesses` becomes the
  store's `IngestBatch::pr_links`. Links are written inside the batch's one
  immediate transaction, beside its records, structural events and checkpoint.
  A conflicting pull-request row, such as a legacy case variant or URL alias
  that `record_pr_link` reports, fails the whole batch. The checkpoint covering
  the witness then does not advance. A batch whose only storable lines are
  witnesses still carries its checkpoint. Only discovered native Claude
  transcript history may carry witnesses, and generic imports, reader streams
  and plugin receipts cannot.
- **Copied context.** A fork's inherited prefix still names its original session
  in `sessionId`. That witness is the original session's evidence and never
  becomes the fork's link. It attaches to the named session when the index
  already holds that session as Claude history. Otherwise it is
  ownership-rejected and attaches nowhere: not to the named session, which is
  not manufactured, and not to the fork in its place. The named session's own
  file links it when scanned, so either scan order ends with one link. A skipped
  copy is not a gap in the fork, so the fork's checkpoint still advances. A
  witness whose `sessionId` was rewritten to the fork is indistinguishable from
  the fork's own witness and is linked to the fork. That link is one row per
  session and pull request, not another work record.
- **Convergence.** Duplicate witnesses, reversed arrival and full replays keep
  the earliest first-seen time, the latest last-seen time and the single `exact`
  confidence. An identical replay writes nothing.
- **Metadata only.** A stub holds the repository, number and canonical URL. Title,
  state, merge time, sizes, branch and refresh time stay unknown. A witness is
  no record, tool event or content. Source files are only ever read.

## Unchanged boundaries

- The shared reader stream still treats a `pr-link` line as outside its
  contract, and vendored readers are unchanged.
- The local server's import endpoint still ignores `pr-link` lines. Plugin and
  server command pairing is not claimed.
- No PR metric, app command, UI, refresh, publication or installer change is
  part of this work.

## Migration 7

Store migration 7 deletes existing `transcript` checkpoints once. The next
ordinary resume replays unchanged Claude files and links the witnesses an
earlier build skipped. It adds no table or column. Reader-host checkpoints,
sessions, records, tool events, receipts, coverage, discovered identities,
pull-request rows and links are left as they are, and migrations 5 and 6 are
unchanged. A checkpoint recreated after the upgrade survives every later open,
so a restart does not replay again.

## Verification

- `cargo test -p xt-ingest --test pr_witness --locked`:
  - one exact content-free stub and link at the witness's own time;
  - duplicate, reversed and replayed witnesses converge;
  - two pull requests give two stubs and two links;
  - an inherited copy gives one canonical link in either scan order;
  - a copy of an unindexed session attaches nowhere;
  - each identity, source, surface or session disagreement, and each missing or
    invalid timestamp or malformed identity, commits no record, stub, link or
    checkpoint;
  - a link conflict rolls back the whole batch and its checkpoint;
  - a restart does not replay, and a witness-only tail advances its checkpoint;
  - a v6 index replays once after migration 7;
  - only discovered Claude history may carry witnesses;
  - the reader stream still rejects `pr-link`;
  - sources stay byte-identical.
- `cargo test -p xt-store --test native_checkpoints --locked`: migration 7
  resets only transcript checkpoints, preserves every other table including
  `pull_requests` and `pr_links`, and a recreated checkpoint survives reopening.
- `cargo test --workspace --all-features --locked`,
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  `cargo fmt --all --check` and `pnpm check`: version literals, fixture parity
  (`scripts/fixtures/export.sh` reports schema version 7) and existing contracts.
