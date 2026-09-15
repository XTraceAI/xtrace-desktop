# Native source import acceptance

`xtrace-core import-native` performs an initial import of Claude, Codex and Cursor
history into the existing local index. It needs no installed capture plugin or
cloud login. Codex/Cursor use a supplied checkout of the shared readers named by
`.plugin-pin`; bundled reader delivery and automatic app integration are later work.

```sh
xtrace-core import-native --db PATH --home DIR [--host claude|codex|cursor]... \
  [--pin FILE] [--plugin-root DIR] [--python EXE]
```

The command prints a per-host/per-session JSON report. Exit 0 means complete
coverage, 2 means an explicit missing, unreadable, malformed or partial source,
and 1 means the command could not run. Readable sessions remain available when
other sessions fail. Every import reads from the beginning and deduplicates through
the existing writer.

## Import and persistence

- Claude enumerates main transcripts and nested subagent files into their parent
  session. Hidden entries, memory directories and unrelated files are ignored.
- Codex/Cursor use one streaming parser for both production and fixture tests:
  a decoded session header followed by canonical user/assistant records.
- Writes use bounded batches and the persisted retention policy. With no content
  preference, transcript text, prompt-derived titles and tool inputs remain absent.
  IDs, counts, available usage and source metadata are retained; unknown usage
  remains unknown. Viewing or import does not enable archival.
- Invalid JSON or non-record stream entries abandon the affected session without
  claiming complete coverage. Later decoded headers resume import. Already committed
  batches survive a later failure; the trailing batch waits for producer exit.
- Rejected or UUID-less records yield partial coverage. Conflicting identities and
  discovery metadata fail explicitly; invalid batches cannot persist discovery labels.
  Reader headers initially register only identity; labels persist with a successful
  batch or successful completion of a header-only session.
- Successful imports retain private source locators in the existing `source_cursors`
  table with **position zero**. The timestamp is only a last-seen observation; a
  locator never authorizes skipping input. Empty scans update an existing locator
  but create no new one. Failed/partial scans do not replace a locator. Reimports
  preserve indexed history when sources shrink.
- Resume is authorized only by a **checkpoint** in `native_checkpoints`, which binds a
  position to the source generation it was read under (see below). A checkpoint
  commits inside the transaction of the batch whose rows it covers, and only when
  every record of that batch was stored; a batch with a rejected or UUID-less
  record keeps the earlier checkpoint, so progress never passes unimported input.

## Source safety and scope

- The CLI validates database and SQLite-sidecar destinations before opening SQLite.
  Paths inside native history and existing multiply-linked destinations are rejected.
  Links in known native session directories pointing back at the index or its sidecars
  are also rejected before opening SQLite.
- Claude source roots, project aliases and transcript aliases are diagnosed instead
  of deliberately followed. Missing, unreadable and undecodable entries have explicit
  coverage results; blank session identities cannot enter discovery.
- On Unix, transcript opens use no-follow/nonblocking flags and check the enumerated
  device/inode before copying. A bounded anonymous snapshot must contain exactly the
  captured source length. Validation and import share its bytes; later appends wait
  for the next scan. Temporary contents are removed on close.
- Surface consistency is checked within each Claude file before writing. Dropped
  records still contribute known surface facts; unusable explicit labels on ordinary
  and dropped records are errors.
  Separate files with contradictory identities remain explicit conflicts; changing
  source files does not automatically replace historical index identities.
- Reader code executes from a temporary export of the verified Git commit, excluding
  ignored Python files/caches. On Unix, the probed interpreter is resolved to an
  absolute path before changing the reader's working directory.
- These checks do not provide a security boundary against another local process
  concurrently replacing ancestor directories. Directory-handle-relative traversal
  is not implemented. Source file bytes are never intentionally modified by import.

## Incremental scanning and watching

- **Source generation contract.** A Claude transcript's generation is its device and
  inode, the inode's change time, a SHA-256 digest of every byte before the checkpoint
  position, a digest of the last 4 KiB before it, and the count of complete lines
  consumed (for reporting). A scan resumes behind the position only when the same
  inode still holds at least that many bytes and the whole prefix digests to the
  recorded value. A file whose length and change time both still match is proven
  unchanged by the trailing digest alone, since every write moves the change time and
  no ordinary tool sets it back; a file whose change time moved is proven by its whole
  prefix even when its length did not. Another inode at the path is a replacement, a
  shorter file a truncation, a differing digest an in-place rewrite: each starts a new
  generation and the file is read from the beginning again. Records dedupe by UUID, so
  a new generation costs a re-read, never a gap or a duplicate. History is kept when a
  source shrinks. A partial trailing line is never consumed; the checkpoint stops before
  it and the completed line is read next time.
- A reader host's generation is the instant of its last scan that covered every
  session (`scan:<host>:<home>`), bound to the pinned producer commit and version
  that ran it. The next scan by the same producer passes that instant, less a 2 s
  margin for coarse clocks, as the producer's `--since`, so it re-reads only sessions
  the producer saw modified since then; a different producer (a moved pin) starts
  with a full scan, since it may discover sessions the old one did not. A scan with any gap (skipped or partial
  session, diagnostic, producer failure) leaves the old generation in place, so the
  next scan repeats its whole range. Per-session reader locators stay at zero.
- **Migration.** An index written by the initial importer holds zero-position
  locators and no checkpoints. The first resuming scan reads every source whole
  (the locator proves nothing), adds no rows, and records checkpoints; the next scan
  then proves them. `xtrace-core import-native --replay` rereads everything
  regardless and still records checkpoints.
- **Watching.** `xt_ingest::native::watch::Tailer` registers a recursive `notify`
  watcher (FSEvents on macOS, inotify on Linux) on each existing host root before the
  initial scan; an absent root is covered by a non-recursive watch on its nearest
  existing ancestor (the `.cursor` parent, or the home) so its creation is seen, and a
  root that appeared is watched before the reconciliation that first enumerates it. A
  watched root that vanished is forgotten so its ancestor covers it again. Changes queued during the scan
  are coalesced (250 ms quiet period) and reconciled before `ready`, repeatedly until
  the queue is quiet, so an append or new file written during the scan with no later
  event is indexed before ready. Event kinds are never trusted: any path under a host
  root, a directory-level or coalesced event, a rescan request or a watcher error
  marks the host dirty, and reconciliation re-enumerates it; an unchanged Claude file
  costs a stat and a 4 KiB read and touches the index not at all. Reader hosts
  reconcile through the incremental producer scan. A watcher that cannot be created
  or a root that cannot be watched leaves freshness `degraded` with the reason; the
  tailer still scans and reports ready, never silently as live. Restarting resumes
  from checkpoints: only appended input is read.
- `xtrace-core watch-native --db PATH --home DIR [--once | --for SECONDS]` runs the
  tailer headlessly and prints one JSON line per event (`ready`, `reconciled`,
  `stopped`); it exits 0 only when live and every scan was complete. App-wide
  orchestration (start order, UI events, scheduling) belongs to ING-13.
- Scope kept out: scheduling frameworks, plugin delivery, cloud synchronization,
  full-content storage, automatic historical-identity repair and UI. The importer's
  per-file safety checks remain as documented above; the tailer adds no traversal
  boundary against a hostile local process.

## Verification

| Area                         | Required checks and expected result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude filesystem            | Main/subagent mapping; ignored entries; partial lines; metadata-only default; repeated import without duplicates; unchanged originals; missing/permission/alias/name errors; cross-batch label consistency; corrected invalid files import successfully.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Stream and lifecycle         | Native identity/usage fixtures; malformed headers/records; no cross-session attribution; bounded batches; draining after stream failure; incomplete producer exit; partial rejection reports; exact-batch boundaries.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Checkpoints and tailing      | `claude_tail` suite: changes during the initial scan with no later event are indexed before ready; appends, completed partial lines, duplicate events, new files and whole new directories converge within seconds; truncation, replacement and in-place rewrite start new generations without duplicates; restart reads only appended input; a failed record transaction keeps the checkpoint behind the failed batch and the retry converges; originals unchanged; metadata-only rows; zero-position locators migrate by one full replay; a watcher failure reports degraded; a root that appears later is watched before it is enumerated; the absent Cursor sibling root is seen through the parent watch. Store: `native_checkpoints` suite and the atomicity failure list. CLI: `watch_native_once_reports_ready_and_resumes_on_the_next_run`. Pinned conformance: the second import runs with `--since` and stays complete. |
| Native locators              | Position remains zero for repeated, older and tied observations; old development offsets reset safely; empty undiscovered sessions create no locator; historical records survive shorter/empty replacements.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Storage ownership            | Mismatched host/native/conversation/surface/start facts reject before writes; existing batch atomicity and retention checks remain in force.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Source and executable safety | Database/source aliases reject before mutation; opened-file replacement and short-copy rejection; fixed snapshot under append; ignored Python modules excluded; stable interpreter selection.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Pinned conformance           | All four inventoried tests execute, including the real pinned reader-to-index import, deduplication and unchanged-source evidence.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

```sh
cargo test -p xt-ingest --test claude_fs --test native --locked
cargo test -p xt-store --test completed_scan --locked
cargo test -p xt-server --test native_import --locked
node scripts/ci/run-hook.mjs plugin-conformance
pnpm check:native --base FULL_REVIEWED_BASE_SHA
```

Use synthetic fixtures. Record the exact source/base, actual macOS version and
results in the PR. The portable Unix invalid-name helper test runs on macOS; the
real invalid-filename filesystem test is Linux-only because macOS rejects those
names. A macOS run does not claim that Linux-only case or minimum-macOS release QA.
