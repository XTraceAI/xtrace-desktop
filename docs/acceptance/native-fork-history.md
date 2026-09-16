# Native fork history

Claude session files can contain inherited records whose `sessionId` names an
older container, or copied records whose `sessionId` was rewritten to the fork.
The native UUID identifies the work. Indexing these files must count that work
once, without discarding a fork's new records or moving an existing work record.

`native_record_copies` records additional session contexts for matching work.
`session_work_records` exposes the original context and those references. Session
history counts can overlap; overall work counts must deduplicate record UUIDs.
Usage aggregation must also retain the API-message representative rules. The primary record's `session_id` remains its storage partition and must
not be interpreted as proof of which fork originally authored the work.

Only discovered native Claude imports may create these references. Known
measurement fields must agree apart from the container and session-local parent
links. Missing facts may be enriched. Retained content must agree when retention is
on. Contradictory identity labels, different record types or conflicting measured
facts remain errors. Generic imports and plugin receipts retain their existing
ownership checks. Existing conflict flags are not automatically cleared.

Copied parent links are preserved on their session reference. A changed parent
within an already recorded copy context remains explicit rather than silently
replacing that relationship.

For the observed fork snapshot format with four zero aggregate counters and one
same-model `message` iteration, the native adapter recovers the four counters
from that explicit iteration. It does not treat arbitrary zeros as missing or
calculate multi-model billing. Other known measurements retain normal conflict
checks. See [Claude API iteration usage](https://platform.claude.com/docs/en/build-with-claude/compaction).

The migration adds references and invalidates old Claude checkpoints for a
one-time replay. During replay, a validated same-model iteration may correct a
legacy all-zero aggregate only when the remaining known work facts agree.
That proof is internal to the native adapter and cannot be supplied through JSON.
Arbitrary zero-versus-positive disagreements remain conflicts. No transcript
content is copied, ownership never transfers, and historical flags stay intact.
Sessions reads the shared view; timestamp enrichment refreshes every referencing
session's range in the same transaction.

## Verification

- `cargo test -p xt-ingest --test claude_fs --locked`: inherited prefixes and
  rewritten prefixes pass in either file order. Two original records plus one
  fork-only record produce three work records, five session-context memberships,
  and unchanged results on replay. Source hashes stay unchanged; no content is
  retained. Conflicting measurements stay rejected. An injected copy-write
  failure rolls back both a preceding new record and destination metadata.
- Upgrade regression: unchanged v3 checkpoints are invalidated; explicit iteration
  evidence repairs legacy zeroed usage in either scan order, without clearing
  historical conflict flags. Unproven zero-versus-positive counters stay rejected.
- Shared timestamp regression: enriching the owning record refreshes copied-session
  ranges; a later full replay of sparse copied metadata remains accepted.
- `cargo test --workspace --all-features --locked`: existing generic ownership,
  receipt, checkpoint, migration, retention and native watcher contracts pass.
- `pnpm check`: generated fixture/schema parity and existing UI checks pass.
- `pnpm check:native --base <reviewed-base-sha>`: full local native validation.

Keep real-history validation in a separate local metadata database. Do not reset
an installed index or erase historical conflict flags as part of this change.
