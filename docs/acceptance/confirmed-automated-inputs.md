# Confirmed automated inputs

A saved user input may have been submitted by another agent rather than a
person: a dispatcher launches or resumes a session with a prompt, and the host
records that prompt exactly like one a person typed. Producer labels such as
`origin.kind=human`, `promptSource=typed`, `source=exec` or a surface name do
not distinguish the two, so none of them is evidence here.

This slice (Stage A) adds the storage and metric semantics for a **structural
confirmation** of one such input. It does not find confirmations: a caller that
has proven the relationship supplies it. Automatic correlation of dispatcher
tool calls (Stage B) is separate work and will produce these same facts. Until
then, no input is confirmed by the app on its own, and every unmatched or
unknown input keeps exactly its previous treatment. Nothing here claims an
unmatched input is verified human.

## What a confirmation is

`Store::apply_automated_input_confirmations` takes up to 1,000
`AutomatedInputProof`s and applies them in one immediate transaction. A proof
names only structural identities:

- the target record UUID, its owning canonical session and that session's
  native identity;
- the dispatching parent: host, canonical session, tool call ID, the zero-based
  operation index inside that call, and the paired result's ID when it has one;
- a closed evidence kind (`agent_dispatch`) and the matcher version.

No prompt, command, output body, digest or free-form evidence is accepted or
stored. Identifiers are bounded (256 bytes, no control characters); a malformed
proof rejects the whole call before anything is written.

Migration 10 adds `confirmed_automated_inputs`, keyed by record UUID with a
foreign key to the record in its owning session. Rows are immutable: a trigger
refuses updates and deletes. The parent need not be indexed, so it is not a
foreign key. The migration requests no replay and touches no checkpoint,
record, receipt or coverage row.

Each proof receives one disposition:

| Disposition                          | When                                                                                                               |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| `Confirmed`                          | The target exists in the named session, the native identity matches, and it is a human-classified user text input. |
| `AlreadyConfirmed`                   | The identical proof is already stored. Nothing changes.                                                            |
| `Abstained(MissingTarget)`           | No record has this UUID.                                                                                           |
| `Abstained(SessionMismatch)`         | Another session owns the record, including one that holds it only as a copied native context.                      |
| `Abstained(NativeIdentityMismatch)`  | The session's native identity is unknown or different.                                                             |
| `Abstained(NotUserInput)`            | Assistant record, non-user role, meta/context, sidechain, tool-result carrier, or a judge session.                 |
| `Abstained(NotHumanClassified)`      | The stored classification is not human, or is unknown. A confirmation never decides an unknown input.              |
| `Abstained(ConflictingConfirmation)` | A different proof already holds this input, or this parent operation already confirms another input.               |
| `Abstained(AmbiguousInBatch)`        | The same call names this input, or this parent operation, again with different facts. None of them applies.        |

The report lists the sessions whose effective classification changed so the
caller can invalidate their measurements after the commit.

## Effective classification

`records.is_human` keeps ingestion's raw classification, so replay stays
fill-only: a later native or plugin replay that knows nothing of the
confirmation can neither restore the input to human nor remove the fact, and
the record's UUID, session, order, usage and tool calls never change.

`v_records` (and therefore `v_session_events`) exposes:

- `is_human`: the effective value, `0` for a confirmed input;
- `raw_is_human`: ingestion's value, unchanged;
- `confirmed_automated_input`: `1` for a confirmed input.

A confirmed input stays in the projection. It is actual record data, and the
transient transcript still shows it.

## Metric semantics

Every metric that folds human messages reads the confirmation flag itself. A
confirmed input is not merely a non-human event: the folds treat `is_human=0`
as agent work, which would be wrong here.

| Rule                  | Confirmed input                                                                                                                                                                                                                                                                        |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M-02, M-15            | Not a human message; contributes no characters to the typing estimate. Session, record, assistant-record and tool counts are unchanged.                                                                                                                                                |
| M-03                  | Neither opens a turn nor answers one: a human followed only by a confirmed input has no assistant turn, and a confirmed input inside a turn does not split it. Favorite-model turn attribution uses the same boundaries.                                                               |
| M-07                  | No Human characters or estimated input effort. Other eligible inputs use their full counted character lengths independently of message timing.                                                                                                                                         |
| M-09, M-20            | Neither starts nor ends a stretch and is never its endpoint or tool evidence. Human, tool, confirmed input, tool, human is one stretch ending at the last tool event. A session whose only input is confirmed has no human-seeded stretch; none is invented.                           |
| M-05, M-06            | Unchanged. The all-event timeline keeps the confirmed input's timestamp, so agent hours and concurrency are exactly what they were. A stretch's M-05 active time keeps it for the same reason. This is the current all-event basis, not a claim that a prompt timestamp is agent work. |
| M-09 timestamp health | Unchanged: the user/assistant population still includes the record.                                                                                                                                                                                                                    |
| M-04, cost, coverage  | Unchanged.                                                                                                                                                                                                                                                                             |

The displayed definitions of M-02, M-03, M-07 and M-09 in
[`design/rule-contract.json`](../../design/rule-contract.json) state these
semantics, including that an unconfirmed input is not thereby proven human.

Dashboard and Sessions (per-session counts, hands-off medians, stretches and
repeats) read the same shared projections and folds, so their human totals and
stretch boundaries agree for one snapshot. PR effort in this build reads token
usage only and has no human total. No UI filter exists.

## Measurement revision

The content-free measurement projection is now digest schema version 2: version
1's 28 fields plus `confirmed_automated_input`, present only for a confirmed
input. New receipts are sealed with version 2. An incoming canonical record
cannot state the field, so a capture payload never matches a confirmed record's
current measurement: a corrected input is not verified capture.

Existing receipts are immutable and are compared under the version they were
sealed with. A version 1 coverage row matches only a record no confirmation has
changed, using version 1's own bytes; for a confirmed record, or under an
unknown version, it never matches. Earlier revisions are never recomputed,
rewritten or reinterpreted.

A retried plugin batch is encoded under the version its existing receipt was
sealed with, so an unchanged retry of a version 1 receipt still matches exactly
and is acknowledged without touching the receipt; a new receipt uses version 2.
A differing payload, or a receipt sealed under an unknown version, is rejected.
A retry after a confirmation is still idempotent, but the incoming payload
cannot state the confirmation, so it never verifies the corrected input.

## Historical Guardian turn confirmations

A Codex Guardian reviewer thread receives each review request as a saved user
input in its own turn, and ingestion classifies that input as human. The parent
link is a turn of another thread, not a tool call, so it is a second evidence
kind with its own table rather than a relabelled `agent_dispatch`.

Migration 12 adds `guardian_turn_inputs`, a metadata-only sibling of
`confirmed_automated_inputs`, which it leaves untouched (every installed row
keeps its rowid and bytes). One row holds only:

- the target record UUID, its owning Codex session and that session's native
  identity;
- the reviewer's own turn ID;
- the parent thread's native identity and the parent turn that started it;
- the closed evidence kind `guardian_turn_dispatch`, the matcher version and the
  confirmation instant.

Every identity has its closed canonical shape: lowercase hyphenated UUIDs, with
the session `codex-` followed by its native identity. The parent is never the
reviewer thread or turn itself. A foreign key binds the row to the record in its
owning session, one Guardian turn confirms at most one input, and triggers
refuse updates and deletes. No prompt, log line, body or digest is stored.

`Store::apply_guardian_turn_confirmations` takes up to 1,000
`GuardianTurnProof`s in one immediate transaction, with the same all-or-nothing
validation, dispositions and affected-session report as the tool-call path. A
proof confirms only a human-classified, unconflicted (`has_conflict=0`) user
text input of Codex history read by the native reader (`host=codex`,
`source=readers_cli`) in a user session whose native identity matches. It adds
two closed abstentions, `NotCodexReaderHistory` and `ConflictedRecord`. An input
already held by either table, or a Guardian turn already confirming another
input, is `ConflictingConfirmation`; the tool-call path also refuses an input a
Guardian turn already holds. A repeated identical proof is `AlreadyConfirmed`.
Each proof names one record: a reviewer session's other inputs keep their
classification until a proof names each of them.

`v_records` and the typed record read treat a row in either table the same way:
the effective `is_human` is `0` and `confirmed_automated_input` is `1`. The
measurement field keeps its meaning (a structural confirmation that another
agent submitted this input) and its digest version, and every rule in the table
above applies unchanged.

**This is a historical correction path, not automatic proof acquisition and not
the forward first-ingestion witness pipeline.** The app never creates these rows
on its own. A separate, privately run promoter checks the saved reviewer rollout
and the host logs, and supplies the identities; the Store accepts them as
structure and does not claim to have verified those external sources. Old
native imports carry no first-ingestion digest, so the promoter's check is a
correction-time comparison. This build ships no manifest, record IDs, paths or
counts, and applies nothing to a real index.

## Injected context proofs (Codex selected skills)

When a person selects a skill, Codex saves the skill's instructions as a
second user message beside the person's own `$skill` request, and ingestion
classifies both as human. The native reader can prove, from the row's declared
native content kinds and identifiers alone, that one canonical record is that
injected body (`codex_selected_skill_instructions`). The person's request,
pasted text that reads like a skill body, Guardian inputs and heartbeats are
never claimed.

Migration 13 adds `injected_context_inputs`, a third metadata-only table that
leaves both confirmation tables untouched. One row holds only the record UUID,
its owning Codex session and native identity, the closed contract
`memhub.codex.origin_evidence`, version `1` and kind, the rollout segment
(`flat` with no rollout ID, or `paginated` with one), the native row index and
ordinal, the native `msg_` item ID, the turn ID and the discovery instant of the
read. CHECKs fix every shape; a foreign key binds the record in its session; one
native item, and one native row, name at most one input; a trigger refuses a
proof for an input either confirmation table already holds; rows are immutable.
No text, text digest, path or title is stored. The confirmation tables gain no
trigger, so SQL alone does not stop a later raw insert into either of them for
an input a proof holds; that order is refused by the Store's confirmation
paths, as below.

**A proof is accepted only together with its record's first insertion.** There
is no standalone API. A strict importer supplies it through
`IngestBatch::injected_context`, aligned with the input it names, on a native
Codex reader batch (`native_codex`, discovered, no receipt); ordinary import
leaves the field empty and any other batch carrying one is refused. Inside the
batch's one transaction the proof is kept only when its own input's outcome is
`Inserted`, the UUID occurs once in the batch, the row is unconflicted, and the
stored input is a human-classified user text input of Codex reader history in a
user session whose stored native identity matches. A replay, enrichment,
identical retry, copy owned by another session, rejected occurrence or
conflicted row leaves the record as ingestion stored it and reports a closed
abstention (`NotInserted`, `ConflictedRecord`, `Ineligible`, `ConflictingProof`);
a later exact read never upgrades a row the index already held. Malformed
shapes, a proof bound to another input, session or native identity, a repeated
UUID, item or row, or misalignment reject the whole batch before anything is
written. Both Store confirmation paths refuse an input a proof holds.

`v_records` and the typed record read treat a proof exactly like a
confirmation: effective `is_human` is `0`, `confirmed_automated_input` is `1`,
`raw_is_human` keeps ingestion's value, and every metric rule above applies
unchanged. Historical correction of rows already indexed is out of scope.

- `cargo test -p xt-store --test injected_context_inputs`: a proof committed
  with its new record excludes exactly that input and survives retry, replay
  and reopen with raw facts identical to a read without it; replay,
  enrichment, copy, rejected-type and conflicted records never acquire one;
  ineligible inputs and sessions abstain; malformed, misbound, ambiguous,
  misaligned and non-Codex batches write nothing; the Store's confirmation
  paths refuse a proven input, and SQL refuses a proof for a confirmed one;
  columns are closed and rows immutable.
- `cargo test -p xt-metrics --test injected_context_inputs`: a schema 12 index
  with 621 Guardian and 8 tool-call confirmations upgrades with identical rows,
  projection, metrics and no content; one proven skill body moves only its
  human message, characters and M-07 share, while the request beside it and a
  pasted lookalike still count and tokens, M-05 and M-06 never move.

## Task notification proofs (Claude Code)

When a background agent, command or monitor that Claude Code started finishes,
Claude Code writes a `type: "user"` line announcing it
(`<task-notification>…<summary>…</summary>…`), so ingestion classifies it as
human. Claude Code marks that line itself: its top-level `origin.kind` is
`"task-notification"`. The canonical parser carries only that marker, as a
flag beside the record (`ParsedRecord::task_notification`); the canonical
record is unchanged and the text is never the evidence. A user message that
merely starts with `<task-notification>` and has no marker stays a person's.
On the author's machine, all 296 notification lines found under
`~/.claude/projects` (279 distinct UUIDs) carried the marker and no unmarked
line started with the tag, so there is no text fallback.

Migration 16 adds `task_notification_inputs`, a fourth metadata-only table: the
record UUID, its owning session, the closed kind `claude_task_notification` and
rule version `1`. A foreign key binds the record in its session; rows are
immutable; no text, summary, path or digest is stored. The writer hands the
flags to the Store through `IngestBatch::task_notifications`, aligned with the
inputs. Unlike an injected-context proof, a task-notification proof binds to
the stored record whether this input inserted it or the index already held it:
the marker belongs to the very line the UUID names. It binds only where the
input was accepted and the stored row is an unconflicted, human-classified
Claude user input; otherwise it abstains. A newly bound proof names the owning
session as affected, so its measurements are read again.

Because `records.is_human` stays fill-only, a changed classifier could never
correct a row already indexed (the replay would mark it conflicted instead).
Migration 16 therefore also clears the Claude transcript checkpoints once, as
migrations 5 to 7 did: the next native scan reads every Claude transcript
again and binds a proof to each notification it already held. `v_records`,
`v_human_inputs` and the typed record read treat a proof exactly like a
confirmation, so every metric rule above applies unchanged.

- `cargo test -p xt-metrics --test task_notifications`: marked inputs leave
  M-02 and M-07 while an unmarked lookalike and the person's request stay; an
  index written without markers is corrected by reading the same lines again,
  with no stored record changed or conflicted, and a second read changes
  nothing; the span bubble's automatic line picks the latest notification in
  the span and its summary.
- `cargo test -p xt-ingest --test canonical` and `--test writer`: only an
  object `origin` whose `kind` is exactly `task-notification` is a marker, a
  malformed `origin` is no marker and no error, and a written marker becomes
  one proof.

## Privacy and limits

Metadata-only storage is unchanged. Proofs carry identities only. Original
transcripts are never modified. Applying confirmations to a real index is a
separately authorized step on a copy first; this slice ships no manifest,
session IDs or private paths. Stage B, scheduler detection, typing versus paste
and any reclassification of unmatched inputs are out of scope.

## Verification

| Area                       | Command                                                | Expected result                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Store                      | `cargo test -p xt-store --test confirmed_automation`   | First confirmation corrects one saved human input; repeats are no-ops; replay and reopen keep it with unchanged records, usage, tools and sessions. Every ineligible, unguarded, conflicting and ambiguous case abstains. Rows are structural and immutable.                                                                                                                                                |
| Native replay and receipts | `cargo test -p xt-ingest --test confirmed_automation`  | A full native replay, restart and append keep the confirmation, identities, work and sources. Version 1 and 2 coverage stop matching a corrected input and keep matching untouched records; sealed coverage is unchanged; the version 1 golden still verifies. An unchanged version 1 or 2 receipt retry is acknowledged before and after a confirmation; a changed payload or unknown version is rejected. |
| Metrics                    | `cargo test -p xt-metrics --test confirmed_automation` | Every sequence above, measured unconfirmed, confirmed and with a naive non-human rewrite; unknown classification stays unknown; M-05, tokens and concurrency are unchanged; Dashboard and per-session reads agree in one snapshot.                                                                                                                                                                          |
| Guardian Store             | `cargo test -p xt-store --test guardian_turn_inputs`   | One turn proof corrects exactly its input and survives repeat, replay and reopen. Wrong session, host, source or native identity, non-input, conflicted and unknown records abstain; turn reuse, conflicting proofs across both tables and batch contradictions never write; malformed shapes, self-linked parents and oversized calls reject the call. Columns are structural only and rows immutable.     |
| Guardian metrics           | `cargo test -p xt-metrics --test guardian_turn_inputs` | A schema 11 index with eight tool-call confirmations upgrades with identical rows, projection and metrics. One proof moves only its input's human message, characters, turn, stretch and M-07 share where nothing overlaps it; an overlapped input leaves M-07 unchanged; tokens, M-05 and M-06 never move.                                                                                                 |
| Version literals           | `cargo test -p xt-store`, `cargo test -p xtask`        | Migration history and the generated F1 shell export report schema version 13.                                                                                                                                                                                                                                                                                                                               |
