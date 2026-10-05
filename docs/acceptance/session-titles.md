# Host session titles for visible rows

Sessions and the Dashboard's Sessions card show the title a host gave a
session — the name its own UI shows — when the session's original local source
holds one. The title is read on demand for the rows being shown, held in memory
by that view and dropped with it. It is never written to the app database, a
cache, a log or any file, and it is not searchable. A row without a host title
keeps what it showed before: a title the index already saved, else its
identifier.

## What counts as a host title

- **Claude Code** writes `custom-title` records (`customTitle`, a user's
  rename) and `ai-title` records (`aiTitle`, generated). The last valid rename
  outranks the last valid generated title whatever order they were written in,
  because the client keeps writing a stale generated title after a rename.
  Within each kind the last valid record wins. A value that is not a nonblank
  string, a sidechain record and a record naming another session are ignored.
- **Codex** names a thread with `event_msg`/`thread_name_updated` in its
  rollout; the last valid one wins. Only when the verified rollout names none,
  the last matching `id`/`thread_name` row in the last 4 MiB of
  `~/.codex/session_index.jsonl` is used, read at most once per batch. The
  sidecar never overrides a rollout name — its rows carry no chronology this
  read could weigh against a rollout rename — and it never names a session
  whose rollout was missing, replaced, ambiguous or named another session in
  its own `session_meta`.
- **Codex, paginated or inherited.** A thread whose history spans several
  rollout files — a root `rollout-<ts>-<thread>.jsonl` and continuations
  `rollout-<ts>-<thread>_<rollout>.jsonl`, whose opening `session_meta` says
  `history_mode: "paginated"` or carries a `history_base` — is named **only**
  by the last valid sidecar row whose `id` is exactly the thread. A rename
  inside one segment is never used: a later continuation may supersede it, and
  proving which rename is latest would need every segment read in order, which
  this read does not do. Only each indexed file's opening line is read, so the
  thread's identity is proven without its body. `history_base.thread_id` names
  the rollout a history continues from, not the thread: a fork or sibling is
  never given the name of the thread it continues from, and a sidecar row keyed
  by a rollout identity names nothing. With no matching row, the thread keeps
  its identifier.
- **Cursor** has no host title here, and CLI or headless sessions often have
  none; those rows keep their fallback and nothing is read for Cursor at all.
- A title is never derived from a prompt, a summary or any other text, for any
  host. The shared readers' prompt-derived fallbacks are not used.

A title is shown as its first nonblank line, trimmed, with control and
bidirectional-override characters replaced, and at most 200 characters.

## Which file, and when it is trusted

The view sends canonical session identifiers and a read name, nothing else. The
command resolves each identifier against the index under the store lock — host,
native identity and the locators the index recorded (a Claude session's own
transcript with its checkpoint; a Codex rollout recorded under exactly that
native identity) — then releases the lock before any file is opened. A title is
read only from a source that:

- is a recorded locator, never a name match elsewhere (a transcript that moved
  to another project is `missing` until the index records its new place). For
  Codex this is the thread's root rollout or a continuation whose name is
  exactly `<thread>_<rollout>` with both halves UUIDs; another thread's
  continuation whose rollout identity equals this thread is not one;
- sits directly in its host's history root (`<home>/.claude/projects/<project>/`
  or below `<home>/.codex/sessions/`, as spelled or where a symlinked root
  leads) with no alias on the way and none at the file (`O_NOFOLLOW`);
- is the only distinct file among the session's locators — except that a
  paginated or inherited Codex history may have several, each of which must be
  verified (below);
- for Claude, still holds the generation its checkpoint recorded (a replaced,
  truncated or rewritten transcript is refused; one that only grew is read; one
  with no recorded checkpoint is identified by its recorded name alone, as the
  detail view does);
- for Codex, opens with a `session_meta` naming this session within its first
  200 lines. The opening line of every recorded rollout is read first (at most
  1 MiB, in 16 KiB chunks; the bytes after it in the last chunk are never
  parsed). A rollout of a flat history is then streamed whole, as before, only
  when it is the one locator the index recorded for the thread: beside any
  other recorded file — live, gone, an alias or outside the root — its rename
  may be one a later segment superseded, so the row keeps its identifier and
  the sidecar does not name it either. A paginated or inherited history
  instead needs every live recorded file to open with this thread's
  paginated or inherited `session_meta` (`payload.id`, and
  `payload.session_id` when present, equal to the thread; a `history_base`, if
  any, an object whose `thread_id` is a UUID), each to be a distinct rollout
  identity, at most one to continue from nothing, and none of the recorded
  files to be an alias or outside the root. A continuation name over a flat or
  foreign header, a flat rollout beside another file of the thread, a torn,
  empty or overlong opening line, or a header declared later than the opening
  line leaves the identifier. A recorded file that is gone is tolerated only
  beside another that was verified; a sidecar row alone never names a thread.
  Each header's file must be the one observed — device, inode, length and
  change time — when it is opened, after its line is read and by name after
  that, so an append or replacement in between refuses it;
- is the same file — device, inode, length and change time — through the
  descriptor and by name after it was streamed; a write during the read refuses
  it.

## Bounds

| Bound            | Value                                                                                                                                            |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Rows per request | 50 distinct identifiers (one Sessions page); more, or a repeat, is refused whole                                                                 |
| Per source       | 64 MiB; a larger source keeps its fallback                                                                                                       |
| Per batch        | 512 MiB charged before reading; a Claude transcript with a checkpoint is charged twice its length (its prefix is proven, then streamed)          |
| Per line         | 1 MiB inspected; a longer complete line leaves its source untitled. Only a line containing a title marker is parsed                              |
| Per batch time   | 5 s, cooperative: heard between reads, checkpoint proof included; rows not read by then keep their fallback                                      |
| Sidecar          | last 4 MiB, once per batch, only when a verified Codex rollout names no thread or a verified history is paginated or inherited                   |
| Codex locators   | at most 8 recorded rollout files per thread (one more is fetched, so a thread with more is declined whole before any file is opened)             |
| Locator lookup   | at most 256 index keys matching a Codex thread's name patterns, one more fetched; more declines the thread (no locator, no file opened)          |
| Codex headers    | the opening line of each recorded rollout, charged at up to 1 MiB against the batch budget before it is read                                     |
| Concurrency      | one read per view at a time, streamed in 256 KiB chunks on the blocking pool; at most eight title reads app-wide, separate from transcript reads |

A torn final line is not read, however long. A complete line longer than the
bound could hold the latest title, so the source keeps its fallback rather
than showing an earlier one. A cancel at any point returns no title for any
row of the batch; shutdown cancels running title reads and waits for them. The
deadline and a cancel are checked around every bounded read, including each
64 KiB chunk of a Claude checkpoint's prefix proof, but a single slow read from
the file system is not interrupted, so the deadline is not an absolute
wall-clock bound.

## Views

- **Sessions** reads the loaded rows a page (at most 50) at a time, in list
  order, one read at a time. The scope of a read is the range, search, host
  filter and PR filter: changing any of them cancels the running read, and an
  answer for an earlier scope — or for a read that was replaced — is dropped.
  A row whose indexed record count changes is read again; until the new answer
  it shows its saved title or identity, never the title read for its earlier
  version. A running read that named the row at its earlier version is
  cancelled and replaced; one that only gained rows (a new page) keeps running.
  Leaving the page cancels the read.
- **Dashboard** reads only the lane rows inside the card's own scroll area,
  observed with an `IntersectionObserver` rooted at that region; rows scrolled
  into view are read next, rows never in view are never named. The scope is the
  selected range. A row whose latest span moves is read again. Unlike Sessions,
  it keeps the title last read for that same row in this range while the new
  read waits, and while a read the next refresh cancels or replaces is
  pending: a cancelled read decides nothing. When the new read completes, a
  new title replaces the old one; no title, or a failed read, clears it to the
  fallback. Nothing about
  the card's geometry changes: the name cell already ends in an ellipsis, so the
  no-page-scroll layout and internal scrolling are unaffected.
- **PRs** reads no host title: the merged-PR report's linked-session drawer
  names each member by its saved title or identity.
- **Search** is unchanged and runs over the index only. Because a transient
  title is not indexed, the search box no longer offers to search titles
  ("Search ID, repo or branch…"); a title the index already saved is still
  matched as before.
- A failed or refused read leaves the identifiers and is not retried until the
  rows change. The fixture preview has no local history and reads nothing.

## Known limits

- A rename that adds no indexed record (Claude title lines are not turns) is
  picked up when the row's version next changes, the scope changes or the view
  is opened again, not instantly.
- A Codex session whose history is paginated or inherited across rollout files
  shows only its sidecar name. A rename recorded only inside its rollouts, and
  not (or no longer, beyond the 4 MiB tail) in the sidecar, is not shown. That
  the sidecar row is the name Codex's own UI shows is an inference from Codex's
  session-index contract, to be confirmed by a blinded native comparison; if
  they disagree, those rows should stay at their identifier. A rollout whose
  `session_meta` line is longer than 1 MiB keeps its identifier.
- A flat Codex rollout's opening line is read twice (probed, then streamed),
  and both reads count against the batch budget.
- Any source with a complete line longer than 1 MiB (for example a pasted
  image carried inline) keeps its identifier, whatever titles it also holds.
- Titles are for display. They are not searchable, exported or persisted; that
  needs a separate privacy and indexing decision.

## Tests

- `crates/xt-ingest/tests/session_titles.rs` (synthetic sources only): rename
  outranking a later stale generated title; the last valid title of each kind;
  malformed, blank, sidechain and other-session records; no title; torn final
  line; an overlong complete line (also across chunks) leaving the source
  untitled while a torn overlong one is ignored; an indexed transcript that grew; replaced and rewritten
  transcripts; a moved transcript not found by name elsewhere; aliased
  transcript, aliased project, outside and other-session locators; ambiguous
  and duplicate-spelling locators; cancel, oversized, budget and deadline; Codex
  rollout rename, sidecar fallback, precedence and prompt exclusion; the sidecar
  never naming a missing or foreign rollout, or one declaring a paginated
  history after its opening line; aliased and torn sidecars; aliased Codex
  directories; Cursor reading nothing; outcome order and bytes read; locator
  lookup excluding subagent files and near-miss rollout names; sources,
  database and tree unchanged by a read; and a 50-session page within the batch
  bounds. Paginated and inherited Codex histories: a single paginated root
  named by its last exact-ID sidecar row, reading only its header and a chunk
  past it even beyond the per-source ceiling, and untitled without a sidecar; a
  continuation-only locator; a root and two continuations, where neither the
  root's stale rename nor a later continuation's rename is shown and the
  sidecar row is; a fork whose `history_base` names its parent never borrowing
  the parent's name, and a rollout-keyed sidecar row naming nothing; foreign
  and `session_id`-mismatched headers under a continuation name, a malformed
  `history_base`, a flat or late header under a continuation name, two roots,
  one rollout twice, two originals and a flat rollout beside a paginated root;
  a flat root with its own rename beside a missing, aliased or outside recorded
  continuation, named by neither its rename nor the sidecar, while a lone flat
  root keeps its rename ahead of the sidecar;
  a missing old locator tolerated beside a live one but never alone, an alias
  among the recorded files, and a root replaced by another thread's file;
  overlong, torn and empty headers; no sidecar, wrong or differently cased ID,
  torn, overlong and aliased sidecars, and a torn last row not hiding the one
  before it; more locators than the cap (nothing opened), a header budget,
  cancel and deadline; continuation locator lookup with its near misses and
  cap; more exact locators than the cap declined before any file is opened;
  near misses sorted ahead of a thread's locators not hiding them below the
  lookup's row ceiling, and one row past it naming no locator and opening no
  file while another row of the batch is still titled; and sources, database
  and tree unchanged by a paginated read.
- `apps/desktop/src-tauri/tests/session_titles.rs`: exact request-order mapping
  with absent rows omitted, the 50-row and repeat refusals, cancellation, the
  list, paging, search and stored rows unchanged, and fixture mode reading
  nothing.
- `apps/desktop/ui/src/app/session-titles.test.tsx`: titles in Sessions for the
  page's rows, the next page read on its own, a changed filter cancelling and a
  late answer ignored, a failed read keeping identities without retry, the
  search placeholder, Dashboard reads limited to rows in the scroll area and
  rows scrolled into view, a range change cancelling and ignoring the late
  answer, unmount cancelling, a Sessions row whose version changes before the
  answer replacing the read (its late answer never shown) and showing its
  fallback until the new version is read, and a read that only gained rows
  kept. A Dashboard lane whose activity moves while its new read is delayed
  keeping its last title (also when the next refresh cancels that read, whose
  late answer is never shown), then taking a rename, then clearing to its
  fallback on an answer with no title, and a new range starting with no
  titles; and, at the hook, a kept title cleared by a failed read and never
  lent to another row.
- Unit tests in `checkpoint.rs` and `session_titles.rs`: the checked prefix
  proof asked before and after every chunk and the trailing read, deciding as
  the importer's proof does, and reading no further once told to stop; a cancel
  and the deadline landing mid-proof ending that row as `Cancelled` or
  `Deadline`; a rollout appended to or replaced between being observed and its
  header being read refused as `Replaced`; a cancel landing between two
  headers, and before the sidecar, reading nothing more and naming nothing;
  and the continuation-name parser's near misses.
