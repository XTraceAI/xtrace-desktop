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
  no ordinary tool sets it back, but only once the checkpoint was recorded at least
  2 s after that change time, so a coarse filesystem clock cannot hide a rewrite in
  the same tick; otherwise, and whenever the change time moved, the whole prefix is
  proven even at equal length, and a file proven unchanged that way has its
  checkpoint refreshed with the current identity so the next proof is the cheap one. Another inode at the path is a replacement, a
  shorter file a truncation, a differing digest an in-place rewrite: each starts a new
  generation and the file is read from the beginning again. Records dedupe by UUID, so
  a new generation costs a re-read, never a gap or a duplicate. History is kept when a
  source shrinks. A partial trailing line is never consumed; the checkpoint stops before
  it and the completed line is read next time. A file proven unchanged, or read behind
  its checkpoint, is reported with the surface the index holds for its session, and a
  record behind the checkpoint that names another surface stops the file as it would
  in a whole-file read.
- A reader host's generation is the instant of its last scan that covered every
  session (`scan:<host>:<home>`), bound to the pinned producer commit and version
  that ran it and to the inventory of sessions it covered: each session's path with
  its update clock and the identity (size, change time, device and inode, observed
  without following aliases) of the file behind it and of every sidecar the
  producer's own revision covers, whose clocks its session clock also folds in: a
  store's `store.db-wal`, `store.db-journal` and `meta.json`, and a Cursor
  session's hook state pin under `.config/memhub-plugin/cursorflush`. The next scan by the same
  producer first inventories the host with a headers-only producer run and stamps
  every session before the producer reads anything; only if every session older
  than the cutoff is in the recorded inventory, unchanged in every respect, with a
  change time (the latest of the file's and its sidecars') that had settled at least
  2 s before the generation started (so a
  coarse filesystem clock cannot hide a rewrite in the same tick), does it pass the
  instant, less a 2 s margin for coarse clocks, as the producer's `--since`. A session restored or moved in with an old clock, one replaced or
  rewritten with its clock preserved, one whose file cannot be identified (every
  file, on a platform without inodes), an inventory that cannot be taken, or a different producer (a moved pin) all mean a
  full scan. A completed scan records as covered only the sessions whose file is
  the same after the scan as it was before the producer ran; one replaced
  meanwhile, one that appeared meanwhile, or one that cannot be identified is left
  out and read again next time, and without a pre-scan inventory no generation is
  recorded. A scan with any gap (skipped or partial
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
  root that appeared is watched before the reconciliation that first enumerates it.
  The Cursor hook's state pin directory (`.config/memhub-plugin/cursorflush`), whose
  pins fold into a session's clock and stamp and change on their own, is watched like
  a root, through its nearest existing ancestor until it appears; like the host
  history directories, that area (`.config/memhub-plugin`) is refused as the index
  destination, so the index's own writes never read as source changes. A
  watched root that vanished, or that was deleted and recreated at the same path
  (its directory identity changed), is forgotten and watched again, so a watch tied
  to the old directory never leaves the replacement unwatched; a root the platform
  reports removed, renamed or recreated (or an ancestor of one, the home itself
  included) is registered again whatever its identity says, and its host reconciled,
  since a watch that dies with its directory (inotify) may see the inode reused by
  the recreation. A watcher error drops
  every watch, degrades freshness and rescans every host; the roots are watched again
  on that pass and freshness is live again only once every root is covered. Changes
  queued during the scan
  are coalesced (250 ms quiet period) and reconciled before `ready`, repeatedly until
  the queue is quiet, so an append or new file written during the scan with no later
  event is indexed before ready; the readiness report counts what the initial scan and
  those passes indexed together, so it reads the same whether or not the platform
  also delivered an event for a change made just before the watch was registered
  (FSEvents may). A session the initial scan indexed that a later pass no longer saw (its file
  renamed or removed meanwhile) stays in the readiness report as it was imported,
  as does a diagnostic it raised that a later pass neither repeated nor resolved by
  importing the session at its very path (a directory's diagnostic is never resolved
  by what was imported below it),
  a session the initial scan could not import fully keeps that failure even if a
  later pass imported it (the source may have been replaced in between, and what the
  failure left unread is then gone; the counts still add up), a host the initial
  scan could not scan at all stays incomplete even if a later pass completed, its
  reason kept, and a host stays incomplete around any of these. After ready, the tailer is idle only once every
  change delivered to it, also one delivered while a reconciliation ran, has been
  taken off its queue and reconciled (the delivery count and the idle flag change
  under one lock). A root that appears during a pass, after its
  watches were decided, is watched after the scan and its host scanned again, until
  a pass finds every root watched, so no change below a root falls between its
  enumeration and its watch. Event kinds are never trusted: any path under a host
  root, a directory-level or coalesced event, a rescan request or a watcher error
  marks the host dirty, and reconciliation re-enumerates it; an unchanged Claude file
  costs a stat and a 4 KiB read and touches the index not at all. Reader hosts
  reconcile through the incremental producer scan. A watcher that cannot be created
  or a root that cannot be watched leaves freshness `degraded` with the reason; the
  tailer still scans and reports ready, never silently as live. A stop request lets
  changes already received, and any that arrive within one quiet period, reach the
  index before the tailer stops. Restarting resumes from checkpoints: only appended
  input is read.
- `xtrace-core watch-native --db PATH --home DIR [--once | --for SECONDS]` runs the
  tailer headlessly and prints one JSON line per event (`ready`, `reconciled`,
  `stopped`), each carrying the freshness as of that moment; it exits 0 only when
  the final freshness is live and every scan was complete, so a root that could not
  be watched after readiness still fails the run. An event that cannot be written
  (the consumer of the stream is gone) stops the tailer and exits 1 with the reason
  on stderr, so the watcher never runs on with no observable output. Ctrl-C ends an
  unbounded run through the tailer's stop, after its final reconciliation, also
  with a single runtime worker. App-wide orchestration (start order, UI events,
  scheduling) belongs to ING-13.
- Scope kept out: scheduling frameworks, plugin delivery, cloud synchronization,
  full-content storage, automatic historical-identity repair and UI. The importer's
  per-file safety checks remain as documented above; the tailer adds no traversal
  boundary against a hostile local process.

## Verification

| Area                         | Required checks and expected result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude filesystem            | Main/subagent mapping; ignored entries; partial lines; metadata-only default; repeated import without duplicates; unchanged originals; missing/permission/alias/name errors; cross-batch label consistency; corrected invalid files import successfully.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Stream and lifecycle         | Native identity/usage fixtures; malformed headers/records; no cross-session attribution; bounded batches; draining after stream failure; incomplete producer exit; partial rejection reports; exact-batch boundaries.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Checkpoints and tailing      | `claude_tail` suite: changes during the initial scan with no later event are indexed before ready; appends, completed partial lines, duplicate events, new files and whole new directories converge within seconds; truncation, replacement and in-place rewrite start new generations without duplicates; restart reads only appended input; a failed record transaction keeps the checkpoint behind the failed batch and the retry converges; originals unchanged; metadata-only rows; zero-position locators migrate by one full replay; an appended scan reports the surface the index holds and stops a record that disagrees with it; a watcher failure reports degraded; a root that appears later is watched before it is enumerated; a root created after its watches were decided is watched and scanned again before ready; the absent Cursor sibling root is seen through the parent watch; changes received before a stop request are reconciled; a root deleted and recreated at the same path is watched again; a root the platform reports removed is registered again; the Cursor hook's state pins are watched and a pin change alone is reconciled; a watcher error rebuilds every watch and rescans. Pinned conformance: a Codex session restored with an old clock forces a full scan and is read; a clock-preserving in-place rewrite forces a full scan; a store's `meta.json` rewritten with its clock put back forces a full scan although the store is untouched; once the source set is back, the cutoff is used again. Store: `native_checkpoints` suite (a batch without records cannot carry a checkpoint) and the atomicity failure list. CLI: `watch_native_once_reports_ready_and_resumes_on_the_next_run`; a closed event stream stops the run with exit 1 after the change was indexed; Ctrl-C with a single runtime worker still stops cleanly with `stopped`. Pinned conformance: the second import runs with `--since` and stays complete. |
| Native locators              | Position remains zero for repeated, older and tied observations; old development offsets reset safely; empty undiscovered sessions create no locator; historical records survive shorter/empty replacements.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Storage ownership            | Mismatched host/native/conversation/surface/start facts reject before writes; existing batch atomicity and retention checks remain in force.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Source and executable safety | Database/source aliases reject before mutation; opened-file replacement and short-copy rejection; fixed snapshot under append; ignored Python modules excluded; stable interpreter selection.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Pinned conformance           | All four inventoried tests execute, including the real pinned reader-to-index import, deduplication and unchanged-source evidence.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |

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
