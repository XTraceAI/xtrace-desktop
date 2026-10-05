# Human classification replay

Indexes written before the writer derived `is_human` hold Claude rows whose
classification and text length are unknown. Their unchanged transcript files
resume behind checkpoints, so a normal scan never reads them again.

Store migration 5 deletes only `native_checkpoints` rows whose source is
`transcript`. It adds no table, worker, queue or manual action, and it never
writes classification facts. A new store has no checkpoint rows to delete.
Reader-host checkpoints, sessions, records, usage, conflicts, receipts, coverage
and retained content are left as they are.

The desktop app opens and migrates the store before starting its watcher. The
watcher's normal initial scan finds no transcript checkpoint, replays each
Claude file from byte zero through the existing writer and commits a new
checkpoint at the end of the file. The writer's merge rules fill only unknown
facts that agree with the retained inputs; conflicting observations stay
unclassified. Sources remain read-only and the default metadata-only policy
still stores no transcript content. Later opens are already past version 5, so
restarts resume from the new checkpoints without rereading. A later migration
may reset transcript checkpoints again for its own reason; see
[structural tool kinds](structural-tool-kinds.md).

Receipt coverage stays immutable. A receipt sealed over a legacy measurement
keeps its mask and revision after the record is enriched; replay cannot
retroactively establish capture coverage. Codex and Cursor history are not
affected by this migration.

## Verification

| Area             | Action                                                                                                                                                                                                                                                 | Expected result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Migration        | `cargo test -p xt-store --test native_checkpoints migration_5` builds a schema-4 history with transcript and reader checkpoints, canonical rows and a sealed receipt, then opens it twice.                                                             | The pending migrations are recorded. Only transcript checkpoints are removed; reader checkpoints and sessions, records, cursors, receipts and coverage are unchanged. A checkpoint recreated after the upgrade survives later opens.                                                                                                                                                                                                                                                                                                                                                                                |
| Native replay    | `cargo test -p xt-ingest --test claude_tail claude_tail_v4_upgrade` seeds unknown `is_human` and `text_len` behind an end-of-file checkpoint and a receipt, confirms a resuming scan leaves them unknown, then starts the watcher on a schema-4 store. | The initial scan reports five enriched records and no new ones. UUIDs, sessions and row count are unchanged. A real prompt is human; the assistant row, tool result, command and interruption rows are not, although most have a user role. Text lengths match a fresh import. No content, tool input or title is stored, and a marker string is absent from every index file. Receipt masks and revisions are unchanged and differ from the enriched projection. The new checkpoint sits at the end of the file, sources are unchanged, and a restart reports no new or enriched records with the same checkpoint. |
| Version literals | `cargo test -p xt-store`, `cargo test -p xtrace-desktop --test fixture_mode`, `cargo test -p xtask`                                                                                                                                                    | Migration history and the generated F1 shell export report the current schema version.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
