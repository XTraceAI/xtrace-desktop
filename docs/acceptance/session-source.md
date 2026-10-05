# Session source resolution acceptance

Opening one session in the detail view reads that session's **original local
source** on demand, through the paths the index already uses. Nothing is
persisted by opening it: no store row, no cache, no copy of the bytes on disk,
no log of what the session said. The source is opened for reading only, and the
records live in memory until the caller drops them.

`xt_ingest::native::session_source::load_session_source` answers one of two
things: the session's records, or the one reason they may not be shown. The
measurements the index already holds are never affected by that answer — an
unavailable source means the text is unavailable, not that the work is in
doubt.

**Codex and Cursor through the pinned reader.** Their sources are read by the
pinned producer's exact-detail mode — one identified JSONL session, whole, in
memory, nothing staged on disk — run from the bundled readers verified against
the compiled pin at each open, with an interpreter resolved under that open's
own cancel token. [Codex and Cursor](#codex-and-cursor-through-the-pinned-reader)
below says what is accepted and what is refused. The import path that indexes
them is untouched.

Run `cargo test -p xt-ingest --test session_source --locked --offline`,
`cargo test -p xt-ingest --test exact_detail --locked --offline`,
`cargo test -p xtrace-desktop --test session_transcript --locked --offline`,
`cargo test -p xt-ingest --lib session_source --locked --offline`,
`cargo test -p xt-store --test source_locators --locked --offline` and
`cargo test -p xtrace-desktop --lib session_source --locked --offline`. Every
transcript in these suites is written by the test: short synthetic lines in the
canonical shape, never a real conversation.

## Resolving the source

- A Claude session owns its files by name. The lookup lists the project
  directories under `~/.claude/projects`, takes `<project>/<session>.jsonl`
  where it exists, and adds the subagent files under
  `<project>/<session>/subagents/`. Nothing is opened while candidates are
  gathered.
- **No alias is followed** — not a project directory's, not a transcript's, and
  not the session directory's, whose target could file a foreign tree's
  transcripts under this session.
- **What could not be listed is remembered, and where it was matters.**
  Somewhere that could hold a transcript named after this session — a project
  directory, or an entry bearing that name that could not be stat'ed — leaves
  two questions unanswered: whether the session is absent, and whether the one
  candidate found is the only one. Neither is answered by reading, so the
  source is **unavailable**, not shown with a caveat: a candidate that may have
  a twin is the ambiguity rule with the ambiguity merely unseen. The session's
  own subagent tree is different — which session those files belong to is not
  in doubt, only how many there are — so that is a **gap** in what loads.
- **What the index read is reconciled against what is there.** A subagent
  transcript the index read and this read cannot see is named as a missing
  gap: its records are inside the saved measurements, so a session without it
  is not the session those numbers describe. It is reported, never hunted for
  somewhere else. Subagent files are matched by where they sit below
  `<session>/subagents/`, so a project directory renamed above them is not
  mistaken for a session that lost a file, while a file that is really gone
  still is. Only a subagent tree that was listed completely can say a file is
  not in it: when it could not be, the gap above already says the set may be
  incomplete, and calling an unseen file a lost one would claim more than was
  seen.
- A **session identifier** becomes a file name, so one that could name
  something else — empty, a path, a traversal step, a control character, a
  leading dot, or the reader's own `latest` keyword — is refused before any
  lookup.
- The **recorded locator** (`source_cursors`, position zero) says where the
  index read the session from. It is read through
  `Store::source_cursors_like`, which matches locator keys only: a key names a
  local path, never content. Wildcards inside an identifier are escaped, so one
  session cannot claim another's locator.

## Reading only what was verified

Each file of the session is read in this order, and the order is the contract:

1. Open it the way the importer does — a regular file, no alias followed, still
   the entry that was observed — and capture its device, inode, length and
   change time from the open descriptor.
2. Charge the session's cumulative **byte ceiling** from that captured length,
   **before** anything is allocated for the file and before its generation is
   proven. An oversized session costs a stat, not a read.
3. Prove the generation against the checkpoint the index recorded.
4. Read exactly the captured length into memory, in bounded chunks a cancel can
   interrupt. A read that ends early means the file shrank under it.
5. Observe both the **descriptor and the name** again. The descriptor must
   still describe the file that was read, and the name must still hold it, with
   the same length and change time.
6. Parse those verified bytes, charging the cumulative **record ceiling**, and
   check for a cancel before returning.

Any mutation observed across the read — a concurrent append included — makes
the source a replaced one rather than a transcript to show. An append that had
_completed_ before the open is not a mutation: it is part of the captured
length, and the checkpoint reports it as a session that has grown.

A session's generation is the weakest of its files': one file with no recorded
checkpoint leaves the whole session `unrecorded`, and otherwise one file that
has grown makes the session a grown one.

## Outcomes

| Case                                            | Setup and action                                                                                                              | Expected result                                                                                                                                                                                                                 |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source available                                | Index a synthetic home, then open the session.                                                                                | Its own records load in read order, with the jump target U-09 needs present. Generation is `indexed`. Every record carries this session's identity.                                                                             |
| Append completed before the open                | Append a turn after indexing, then open.                                                                                      | The session loads whole and reports `indexed { appended: true }`: the measured bytes are still its first bytes.                                                                                                                 |
| Append during the read                          | Append continuously while a large source is read.                                                                             | Either the read completed before the writer, or it is `replaced`. A loaded session is never spliced and never short.                                                                                                            |
| Replaced during the read                        | Rename another file over a large source while it is read.                                                                     | Either generation may be read, always whole and always under this session's identity, or the read reports `replaced`.                                                                                                           |
| Missing                                         | Open a session no file under the home names, and open one under a home with no Claude history.                                | `missing`. No error, no empty transcript.                                                                                                                                                                                       |
| History that could not be listed                | A project directory that cannot be listed could hold this session.                                                            | `unreadable`, never `missing`: an absent session is not a claim this read is entitled to make.                                                                                                                                  |
| Found beside a project that could not be listed | The session is found; another project directory that really does hold a second file of the same name cannot be listed.        | `unreadable`. One candidate beside a place that could hold its twin is not known to be the only one.                                                                                                                            |
| Subagent the index read that is now gone        | Index a session with two subagent files, then delete one.                                                                     | The session loads with a `missing` gap naming that the text is short of what the numbers count. Nothing is written and nothing is looked for elsewhere.                                                                         |
| Files that moved together                       | Rename the whole project directory and index it again, so both spellings of every file are recorded.                          | No `missing` gap: a session whose files moved together did not lose one.                                                                                                                                                        |
| Subagent files sharing a name                   | Two subagent transcripts with identical file names under one session.                                                         | Each is measured against its own recorded generation, not its namesake's: both load with no gaps.                                                                                                                               |
| Moved                                           | Rename the project directory after indexing, then open.                                                                       | `moved`. The new place is known but is not read: the measurements belong to the place it was read. After the index reads it where it now is, both spellings are recorded and it loads again.                                    |
| Unreadable                                      | Remove read permission from the transcript, then open.                                                                        | `unreadable`, and the store's counts are unchanged.                                                                                                                                                                             |
| Replaced, truncated, rewritten                  | Rename another file over the transcript; shorten it; rewrite it to the same length.                                           | `replaced` in all three: the source is no longer the generation that was measured.                                                                                                                                              |
| Ambiguous                                       | Two project directories holding a file with the same session name.                                                            | `ambiguous`; neither is shown.                                                                                                                                                                                                  |
| Copied history identity                         | A fork whose inherited prefix names another session.                                                                          | The file the session owns settles its identity; every loaded record reads as this session. Asking for the session the copied prefix names does not reach this file.                                                             |
| Disagreeing record labels                       | A line whose two spellings of its native session disagree.                                                                    | The file stops at that line, exactly where indexing stops, and the stop is a named gap. Earlier lines still load.                                                                                                               |
| Subagent files                                  | A subagent transcript filed under the session.                                                                                | It loads after the session's own transcript, under the same identity.                                                                                                                                                           |
| Subagent the index never read                   | Index the session, then add a subagent file; then index both and grow the subagent file.                                      | The session is `unrecorded` while one file is untied, and `indexed { appended: true }` once both are recorded and one has grown.                                                                                                |
| Unreadable subagent file                        | Remove read permission from a subagent transcript.                                                                            | The session loads with an `unreadable` gap, never a silent omission.                                                                                                                                                            |
| Subagent tree that cannot be listed             | Index a session with a subagent file, then remove read permission from the subagents directory.                               | The session's own transcript still loads, with a `discovery_incomplete` gap: which session those files belong to is not in doubt, only how many there are. No `missing` gap — the file is there, it simply could not be listed. |
| Changed inside its own read                     | Truncate a file, and rewrite one at the same length, in the window between its observation and that observation's validation. | `replaced`, deterministically: `truncated` for the shorter file, `rewritten` for the one whose length did not change. A refusal carries no records, so no part of either generation can be shown.                               |
| Rewritten in place while being read             | Overwrite a contended source at the same length, concurrently, while it is read.                                              | Refused, or one generation's words, whole — never a line of each.                                                                                                                                                               |
| Alias for a transcript                          | A symbolic link in place of a transcript.                                                                                     | Not followed, and not passed over: something named this session and could not be used, so `unreadable`.                                                                                                                         |
| Alias for the session directory                 | A symbolic link to another session's directory, where this session's directory would be.                                      | Only this session's own transcript is read; the foreign subagent tree is never filed under it, and the gap says discovery was incomplete.                                                                                       |
| Ceilings                                        | Lower the byte ceiling, then the record ceiling; then give a subagent file more than the session's remaining room.            | `too_large`, naming the ceiling it reached and counting in that ceiling's own unit. A ceiling reached inside any file makes the whole session too large.                                                                        |
| Cancellation                                    | Cancel before the read.                                                                                                       | `cancelled`, with no content.                                                                                                                                                                                                   |
| Codex and Cursor without a reader               | Ask for a Codex session and a Cursor session with the index disabled.                                                         | `reader_unavailable` (`index`) for each, and the home is byte-for-byte unchanged: nothing was started, staged or left behind. An unknown host is `unsupported_host`.                                                            |
| No persistence                                  | Snapshot the source bytes, its modification time, the whole home tree and the store's counts around an open.                  | All equal afterwards. No new file anywhere under the home; no row, session or usage observation added by viewing.                                                                                                               |

## Bounds and cancellation

- One opened session may read at most `MAX_SESSION_BYTES` (64 MiB) over every
  file that belongs to it, and hold at most `MAX_SESSION_RECORDS` (200,000)
  records. Both ceilings are cumulative over the session, not per file. Past
  either, the answer is `too_large`, naming the ceiling it reached and counting
  in that ceiling's own unit, rather than a session shown in part: a view that
  silently dropped the end of a session would mislabel where the time went.
- Memory for a file is reserved fallibly, so a source larger than this machine
  can hold is an unavailable source rather than an aborted process.
- Cancellation reuses the existing `CancelToken`. A cancel is observed before
  the read, between chunks of it, between parsed lines and before the result is
  returned. **A cancelled read returns no content at all** — never a partial
  transcript that could read as a whole one. A load should be given its own
  token, since a cancel is permanent for the token it lands on.

Refusing a file that changed inside its own read is proven **by
construction**, not by winning a race: the crate's own tests act on a file in
the window between its observation and the validation of that observation, and
assert the refusal and its reason. That seam exists only in this crate's test
build — in every other build it is an empty function with no state behind it,
so the contract, the behaviour and the surface of the module are the same
without it, and the integration cases run against the production path. Those
cases are properties, not proofs: whichever way the timing falls, a read that
was **not** refused carries one generation of the file, whole, and not a word
of the other's. They say so by the words the records carry, because
identifiers alone cannot tell two generations apart when only the text
changed.

## What crosses to the view

`SessionSourceStatus` carries the resolution, its ceilings and its
cancellation. Three things are deliberately absent. **Local paths**, as in
`NativeIndexStatus`: where a session's history lives is index metadata, not
view data, and a `moved` or `replaced` source says that it moved, not where to.
**Free text of any kind** — no failure message, nothing a transcript could have
reached: every state is a closed value the view renders on its own terms. **The
transcript itself**: the shape the view renders belongs with the transcript
components it renders through.

## Limits of this stage

- Codex and Cursor have no detail source here at all. Serving them would need a
  reader that streams without staging disk copies; that is not this stage's to
  add, and the existing import path for them is unchanged.
- Structural hook summaries and PR witnesses are counted as non-record lines,
  not rendered as turns.
- A session is held whole in memory, which is what a detail view needs and what
  the ceilings make safe. Indexing keeps draining into bounded batches instead,
  and is not changed by any of this.

## Codex and Cursor through the pinned reader

The app's transcript command resolves the session's host and stored native
identity from the index, as for Claude, and — for Codex and Cursor only, and
only for an identity that could name one session — asks the index for a
reader: the bundle is verified against the compiled pin and the interpreter
probed, both for this open. A missing prerequisite is `reader_unavailable`
with its cause (`interpreter`, `readers` or `index`); fixture startup never
reaches a reader at all.

The reader runs as `--exact-detail --session=<stored id>` under
`xt_ingest::native::readers_cli::run_exact_detail`, which is the child's only
reaper: the open's token is polled, both pipes drain into buffers bounded
before allocation, a pipe past its bound or the hard deadline (the producer's
30 s plus 5 s) kills the reader's process group, and the group is killed
before the leader is reaped on every path. Output is kept only as a complete
success — exit `0`, nothing on stderr, both pipes closed — and then only if
the existing stream parser finds exactly the requested session, from a source
under one of the host's anchored roots (`~/.codex/sessions`;
`~/.cursor/chats`, `~/.cursor/projects`, each as spelled or where a symlinked
root led before the read). A cancel after the last parsed line, and after
translation, still returns nothing.

| Case                                                                                                                                      | Answer                                                         |
| ----------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| A declared producer ceiling                                                                                                               | `reader_limit`, naming it                                      |
| The producer's deadline or the hard deadline                                                                                              | `reader_deadline`                                              |
| A Cursor session whose selected source is a SQLite store (a committed write-ahead log included)                                           | `store_unsupported`; its transcript is never read in its place |
| A source changed while it was read                                                                                                        | `replaced`                                                     |
| Not found by the producer                                                                                                                 | `missing`                                                      |
| A nonzero exit after a whole session, a truncated or malformed stream, another session, a source outside the roots, output past the bound | `reader_protocol`, with nothing kept                           |
| Cancelled, including by app shutdown                                                                                                      | `cancelled`, the reader's group killed                         |

`conformance_exact_detail` runs the real pinned producer over F18, a paginated
Codex group and configured roots that are symlinks: every JSONL session equals
the ordinary export record for record, the store is refused, and nothing on
disk changes.
