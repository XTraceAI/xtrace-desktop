# Native Claude response ordering

Repeated usage snapshots can share a timestamp while their UUIDs sort in the
opposite order. The native importer now preserves the observed order of accepted
Claude response snapshots, so a token query can select the later observation
without treating UUID order as chronology.

Ordering is stored as adjacent before/after UUID relationships, not absolute
line numbers. Copied prefixes can shift line numbers and split ownership while
preserving the same immutable records. Relations combine across those contexts;
replayed UUIDs do not reverse a previously observed order or create cycles.
Unrelated response keys never acquire an ordering relation. Missing ordering
remains missing; this change does not infer a global order for unrelated branches.

Two indexed metadata tables hold proven relationships and each file's last
response snapshot for continuation. A proven append retains continuation state;
a full reread resets it. Only the discovered Claude file adapter supplies this
state, never JSON input, plugin receipts or generic imports. Rejected/conflicting
records cannot acquire ordering evidence. Records, relationships, continuation
state and carried checkpoints commit or roll back together. New relationships
invalidate measurements for the importing session and affected record owners.
An empty/inert full read clears its obsolete continuation state before its
standalone checkpoint is recorded.

Migration 5 adds these metadata tables and clears only Claude resume checkpoints,
so unchanged files are read once to supply missing ordering evidence. Existing
canonical records and usage stay intact. Source files are never edited; ordering evidence contains no
transcript content.

`cargo test -p xt-ingest --test native_response_order` verifies:

- equal timestamps with the later native UUID sorting lower;
- append/resume, full replay and shifted copied prefixes without duplicate work;
- copied prefixes in either arrival order and overlapping sources whose combined
  relationships prove an order neither source proves alone;
- checkpoint failure rolling back new records, order and continuation together;
- inert rewrites clearing continuation without losing proven historical order;
- upgrading an existing index and rereading unchanged sources;
- generic imports being refused access to native ordering.

Token selection consumes this evidence in the dependent metrics PR; this
prerequisite does not change the Dashboard or expose a new user setting.
