# Structural tool kinds

Ingestion classifies every tool call before content retention decides whether
the block that carried it survives, so a metadata-only index still knows what
was called. `tool_uses` keeps `name`, `kind`, `server`, `tool` and `skill`
beside the existing `uuid`/`block_index` identity; `input_json` stays a content
column and is still dropped under the default policy.

## Kinds

`xt_store::tool_use` is the single owner of the mapping, and its kind strings
are the ones the schema already accepts:

| Name shape                      | Kind       | Details                                     |
| ------------------------------- | ---------- | ------------------------------------------- |
| `mcp__<server>__<tool>`         | `mcp`      | `server`, and the whole remainder as `tool` |
| `Skill`                         | `skill`    | `skill`, from the explicit input field only |
| `Agent`, `Task`                 | `subagent` | none                                        |
| any other name (`Bash`, `Edit`) | `builtin`  | none                                        |
| a `<command-name>` user record  | `command`  | the slash command's own name (user role)    |
| a `stop_hook_summary` record    | `hook`     | none                                        |

The tool part of an MCP name may itself contain `__` and is not split again. A
malformed detail stays unknown rather than being guessed: `mcp__server` is still
an MCP call, with no server or tool. Nothing is read from a shell command, an
argument list, a prompt or any other free text, and a `Skill` call whose input
names no skill simply has no skill name.

## Structural events

A slash command and a hook summary are not assistant tool calls. They are
stored as structural rows with no record foreign key, so they never enter a
record's `tool_use_count` or its tool-call rows, and the command record itself
remains an ordinary record. One `stop_hook_summary` is one `hook` event whatever
`hookCount` it reports: M-17 counts summaries, not the invocations a summary
describes. Claude's native scan now consumes these summaries; the shared reader
stream's contract is unchanged and still rejects them.

A slash command is stated by a user record, and only by one: the record's type
and its message role must both say `user`. An assistant record quoting the
marker, and a record whose role is unstated, state no command. A hook summary is
reconciled through the same identity-agreement rules an ordinary record is,
before anything it implies is persisted: its native session, conversation,
platform, surface, start instant and source must agree with the file and the
records around it, and a summary that disagrees commits neither its event nor
the checkpoint covering it.

An event's identity is the native record or event UUID it came from. A summary
carrying no UUID is unsupported and is never given an arrival-ordered identity
instead. The event is stored under one session, resolved in a fixed order: the
canonical record holding the UUID owns it, otherwise an event already stored
under the identity keeps its session, otherwise the identity is new and this
batch's session holds it. A native fork repeating the same line in several files
therefore counts one event, stored once, exactly as its copied work record is
one record with an additional session context.

Only the first step is provenance. A hook summary is no record, so nothing
establishes which session originally produced it; the second step is a stable
storage convention, and which context happens to be scanned first is the one
that holds it. What it guarantees is that the identity has exactly one home and
that a later context does not move it, not that the home is the summary's
origin.

Within that owning session the search is by `source_event_id` **regardless of
source** under the batch's immediate transaction, so the same native event
delivered by the plugin and by the native scan, in either order, is one row, a
plain retry adds nothing, and the same identity replayed with different facts
fails the whole batch. Events commit inside the batch that covers them, beside
its records, cursor and checkpoint.

A record this batch could not store contributes no event at all, whoever else
owns the UUID. A rejected occurrence is not a trustworthy account of the record
it claims to be, so it may not write a command onto the session that does own
it, any more than onto the session that rejected it. An accepted native copy is
not rejected and still resolves to the canonical record's owner, which is why a
fork still counts one event rather than none.

## Replay

An index written by an earlier build holds tool rows with names and positions
but no classification. A replay of the same call fills the missing columns.
Identity is the record UUID plus the block index: a differing name or position
is a different call, and known structural metadata that disagrees is recorded as
a conflict instead of overwriting the stored value or duplicating the row.

Store migration 6 deletes existing `transcript` checkpoints once so the next
scan replays unchanged Claude files through the new extraction. It adds no
table, column, dependency or manual action, and writes no classification itself.
Reader-host checkpoints, sessions, records, usage, receipts, coverage, retained
content and migration 5's human classification are left as they are, and a
checkpoint recreated after the upgrade is never reset again. Codex and Cursor
retain whole scans.

## Verification

| Area                | Action                                                                                                                                                                                                                  | Expected result                                                                                                                                                                                                                                                           |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Mapping             | `cargo test -p xt-store tool_use`, `cargo test -p xt-ingest --test tool_use`                                                                                                                                            | Each name shape maps to its documented kind and details. Malformed MCP detail and an unnamed skill stay unknown. No input field other than an explicit skill name is read.                                                                                                |
| Counts              | `cargo test -p xt-ingest --test tool_use f1_keeps_its_exact`                                                                                                                                                            | F1's stored tool-call total equals its `M-03` golden, every call is classified, and adding a structural event to the same session leaves that total alone.                                                                                                                |
| Retention parity    | `cargo test -p xt-ingest --test tool_use metadata_and_content_modes_agree`                                                                                                                                              | Kinds, details and row counts are identical in metadata-only and full-content mode, and replay adds no row. Only `input_json` and `content_json` differ. No command argument or hook command text is stored in any mode.                                                  |
| Replay identity     | `cargo test -p xt-ingest --test tool_use duplicate_replay`, `copied_native_contexts`, `a_record_this_session_rejected`, `missing_identity_and_conflicting`; `--test claude_fs claude_fs_counts_one_event_for_a_summary` | Repeated delivery and reversed plugin/native arrival give one event. A UUID repeated across copied native contexts is one event, owned where its record is, in either scan order. A missing UUID and a conflicting replay both fail with the batch's records rolled back. |
| Rejected occurrence | `cargo test -p xt-ingest --test tool_use a_rejected_occurrence_writes_no_command`                                                                                                                                       | A session storing a UUID as an ordinary record gains no command when another session's rejected occurrence of that UUID carries the marker; neither session holds an event.                                                                                               |
| Command role        | `cargo test -p xt-ingest --test tool_use only_an_explicit_user_role`                                                                                                                                                    | An assistant record quoting the marker, a record with no stated role, and a record whose type and role disagree state no command; the genuine user record still does.                                                                                                     |
| Summary identity    | `cargo test -p xt-ingest --test tool_use a_disagreeing_hook_summary`, `--test claude_fs claude_fs_reconciles_hook_summary_identity`                                                                                     | A summary disagreeing on native session, surface, platform or conversation fails its batch, stores no event or record, and records no checkpoint. An agreeing summary commits.                                                                                            |
| Legacy rows         | `cargo test -p xt-ingest --test tool_use a_legacy_row_acquires`                                                                                                                                                         | Unclassified rows acquire their classification from a compatible replay; a replay whose block identity disagrees is a conflict and the stored call is retained.                                                                                                           |
| Native consumption  | `cargo test -p xt-ingest --test claude_fs claude_fs_consumes_hook_summaries`                                                                                                                                            | A Claude transcript's slash command and stop-hook summary become exactly one structural event each across two scans, its assistant calls are classified, and the source bytes are unchanged.                                                                              |
| Migration           | `cargo test -p xt-store --test native_checkpoints migration_6`                                                                                                                                                          | Version 6 is recorded. Only transcript checkpoints are removed; reader checkpoints, sessions, records, tool rows, cursors, receipts, coverage and `is_human` are unchanged. A checkpoint recreated afterwards survives.                                                   |
| Version literals    | `cargo test --workspace`, `scripts/fixtures/export.sh`                                                                                                                                                                  | Migration history, the desktop app info and the generated F1 shell export report version 6.                                                                                                                                                                               |

The receipt digest schema and its historical coverage are unchanged: these
columns are not part of a measurement projection, so a receipt sealed before
this work keeps its mask and revision.
