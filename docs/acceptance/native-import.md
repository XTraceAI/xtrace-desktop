# Native source import acceptance

`xtrace-core import-native` performs an initial import of Claude, Codex and Cursor
history into the existing local index. It needs no installed capture plugin or
cloud login. Codex/Cursor use the shared readers named by `.plugin-pin`: the CLI
runs them from a supplied checkout verified through Git; the app runs them from
its bundled copy of the pinned sources, verified by object identity without Git
(see [app integration](#app-integration)).

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

- The CLI and the app validate database and SQLite-sidecar destinations before
  creating or opening SQLite. Paths inside native history and existing
  multiply-linked destinations are rejected. Links in known native session
  directories pointing back at the index or its sidecars are also rejected; an
  entry there the process cannot read is passed over (it cannot be followed
  either), a source root that cannot be resolved (a dangling alias, an
  untraversable parent) is compared as spelled, and the scan reports both as
  diagnostics, so a source-access problem never prevents the app from starting.
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
  ignored Python files/caches, or, for the bundled copy, in place from a tree whose
  every pinned object (the scripts tree, `readers/`, `readers_cli.py`,
  `cursor_flush.py`) hashes to the pinned Git identity (blob bytes; tree entries
  with their modes, sorted as Git sorts them; bytecode caches and symlinks
  refused, since Python would load a cache in place of a pinned module): an edited, incomplete or foreign bundle is a pin mismatch, never
  executed. On Unix, the probed interpreter is resolved to an absolute path before
  changing the reader's working directory. With no interpreter named, `python3` on
  `PATH` is tried first, then the install directories a GUI process's `PATH` does
  not reach (Homebrew, MacPorts, `~/.local/bin`); every candidate must be 3.10+
  with assertions enabled, and the reasons none qualified are reported together.
  Each version probe is a child process like the reader: it is killed after 10 s
  (an interpreter that cannot print its version by then is reported, not
  waited for) and at once by a cancel, so a wrapper that never returns holds
  neither a scan nor a shutdown.
- A cancelled scan (`CancelToken`) kills the reader running and reaps it; the
  sessions it completed stay committed, the session it had open is reported as
  ended early without a cursor, hosts not yet read are reported `cancelled`, and
  a Claude scan ends between files, its checkpoints kept. Nothing a cancelled
  scan wrote is in doubt; the next scan reads the rest. Readers and interpreter
  probes start as leaders of their own process group (Unix), and a cancel or
  deadline kills the whole group, so an interpreter launcher that spawned the
  real interpreter without replacing itself cannot leave a descendant holding
  the stream; a descendant that left the group on its own is beyond this.
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
  it and the completed line is read next time. A file whose session the index knows is
  reported with the surface the index holds, whether proven unchanged, read behind its
  checkpoint or read whole again (replaced, truncated, rewritten, replayed), and a
  record that names another surface stops the file as in the first read of a file.
- **Reader hosts are read whole.** Codex and Cursor have no checkpoint: every scan,
  the initial one and every reconciliation that touches the host, runs the pinned
  producer over the whole host and passes no `--since`; Desktop keeps no cutoff,
  inventory or file fingerprint for them, and their per-session locators stay at
  zero. Records dedupe by UUID, so a repeated scan adds nothing, and a session
  restored with an old clock, replaced, rewritten in place, or changed only in a
  sidecar (`meta.json`, a hook state pin) is simply read again like every other.
  The cost is one full producer run per reconciliation of that host; the measured
  cost on synthetic histories is recorded in the pull request.
- **Migration.** An index written by the initial importer holds zero-position
  locators and no checkpoints. The first resuming scan reads every transcript whole
  (the locator proves nothing), adds no rows, and records checkpoints; the next scan
  then proves them. `xtrace-core import-native --replay` rereads every transcript
  regardless and still records checkpoints. Reader hosts have nothing to migrate.
  Store migration 5 deletes existing transcript checkpoints once so the next
  scan replays unchanged Claude files through the human classifier; see
  [human classification replay](human-classification-replay.md). Migration 6
  does the same once more for structural tool extraction; see
  [structural tool kinds](structural-tool-kinds.md).
- **Watching.** `xt_ingest::native::watch::Tailer` registers a recursive `notify`
  watcher (FSEvents on macOS, inotify on Linux) on each existing host root before the
  initial scan; an absent root is covered by a non-recursive watch on its nearest
  existing ancestor (the `.cursor` parent, or the home; a symlinked home, accepted
  unlike a source-root alias, is followed to the directory it names) so its creation
  is seen, and a root that appeared is watched before the reconciliation that first
  enumerates it.
  The Cursor hook's state pin directory (`.config/memhub-plugin/cursorflush`), whose
  pins carry saved usage for a session and change on their own, is watched like a
  root, through its nearest existing ancestor until it appears; like the host
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
  a session the initial scan imported that a later pass could not read reads partial, the earlier counts, dropped records and rejections carried and the later failure added as a rejection; a session the initial scan could not import fully keeps that failure even if a later pass imported it (the source may have been replaced in between, and what the
  failure left unread is then gone; the counts still add up), a host the initial
  scan could not scan at all (a reader, runtime or pin failure, or an incomplete
  status with no session or diagnostic to say why) stays incomplete even if a later
  pass completed, its reason kept, and a host stays incomplete around any of these. After ready, the tailer is idle only once every
  change delivered to it, also one delivered while a reconciliation ran, has been
  taken off its queue and reconciled (the delivery count and the idle flag change
  under one lock, and an event is counted, queued and clears idle under that lock
  too), ready is published only if nothing is queued at that moment (checked under
  the same lock; a later delivery repeats the startup pass instead), and a stop
  request is honored only once every event delivered before it has been taken off
  the queue and reconciled. A root that appears during a pass, after its
  watches were decided, is watched after the scan and its host scanned again, until
  a pass finds every root watched, so no change below a root falls between its
  enumeration and its watch. Event kinds are never trusted: any path under a host
  root, a directory-level or coalesced event, a rescan request or a watcher error
  marks the host dirty, and reconciliation re-enumerates it; an unchanged Claude file
  costs a stat and a 4 KiB read and touches the index not at all. A reader host is
  read whole again through the pinned producer, whose output dedupes into the index. A watcher that cannot be created
  or a root that cannot be watched leaves freshness `degraded` with the reason; the
  tailer still scans and reports ready, never silently as live. A stop request lets
  changes already received, and any that arrive within one quiet period, reach the
  index before the tailer stops. A shutdown (`Tailer::shutdown`) cancels instead:
  the scan in progress is cancelled as above and the worker joined within a bound;
  a worker that does not end within it is left behind with its reader already
  killed. Restarting resumes from checkpoints: only appended input is read.
- `xtrace-core watch-native --db PATH --home DIR [--once | --for SECONDS]` runs the
  tailer headlessly and prints one JSON line per event (`ready`, `reconciled`,
  `stopped`), each carrying the freshness as of that moment; it exits 0 only when
  the final freshness is live and every scan was complete, so a root that could not
  be watched after readiness still fails the run. An event that cannot be written
  (the consumer of the stream is gone) stops the tailer and exits 1 with the reason
  on stderr, so the watcher never runs on with no observable output. Ctrl-C ends an unbounded run through the tailer's stop, after its final reconciliation, also with a single runtime worker. Both bounds are armed before the tailer starts: `--for` elapsing or Ctrl-C arriving during the initial scan stops the tailer once that scan is done, reconciling what it received, and the run exits 1 as not ready (also when readiness and the bound fall within the same wait step). The CLI's bound is the graceful stop: it waits for the scan in progress, so a pinned reader that never returns holds the command (the app cancels instead; see below).
- Scope kept out: scheduling frameworks, plugin delivery, cloud synchronization,
  full-content storage, automatic historical-identity repair and UI. The importer's
  per-file safety checks remain as documented above; the tailer adds no traversal
  boundary against a hostile local process.

## App integration

- **Startup.** XTrace Desktop starts the tailer in its `setup` hook, after opening
  the application database, over the user's home (`XTRACE_NATIVE_HOME` overrides
  it for synthetic homes and must be an existing directory, since an absent home
  has no ancestor the watcher could cover) for Claude, Codex and Cursor, with a second connection
  to the same database (`xtrace.db` under the data directory), the bundled readers
  and a 250 ms quiet period. Fixture mode (`--fixture`, `XTRACE_FIXTURE`) starts
  no index: its database is disposable. A data directory inside a native source
  root or the Cursor hook state area, or aliased to one, disables the index with
  the reason (the CLI's destination validation, shared), so the index's writes
  never read as source changes; so does a database that cannot be opened.
- **Bundled readers.** The pinned `plugins/memhub/scripts` tree, with the
  producer's `LICENSE` and `NOTICE`, is vendored under `vendor/agent-plugins`
  (`sh scripts/vendor-readers.sh` refreshes it from a checkout) and bundled as the
  `agent-plugins` app resource. The pin is compiled into the app; at startup the
  bundle is verified against it as above and the status reports the pinned commit
  and plugin version, or the reason it is unavailable. An installed plugin, a
  checkout or Git are never consulted.
- **Interpreter.** `XTRACE_PYTHON` names the interpreter and is used as named;
  otherwise it is discovered as above (a `PYTHON` variable the app inherits is
  not a choice: the library reads no environment variable, only the CLI honors
  `PYTHON`). The discovery that fills the status runs
  on its own thread beside the initial scan, never on the startup hook (the
  status reads `resolving` until it answers), and its probes register with the
  tailer's cancel token, so shutdown kills one still running; each Codex/Cursor
  scan discovers again on its own, so an interpreter installed later is found
  without a restart. A missing interpreter is reported in the status and by each
  Codex/Cursor scan as `missing_runtime` with the reasons; Claude indexing is
  unaffected. Resolving the interactive login shell's environment (for
  interpreters reachable only through it) is separate work; naming the
  interpreter is the workaround.
- **Status and events.** `native_index_status` returns the typed
  `NativeIndexStatus`: `phase` (`disabled` with reason, `scanning`, `ready`,
  `stopped`), `freshness` (`unknown`, `live`, `degraded` with reason), `python`
  (`resolving`, `available` with path, `missing` with reason),
  `readers`, one entry per host (`pending`, `complete`, `incomplete`,
  `missing_source`, `missing_runtime`, `pin_mismatch`, `reader_failed`,
  `cancelled`; the detail; sessions imported/partial/skipped; records new/enriched;
  diagnostics count), `reconciles` and `files_scanned`. Every change is published
  as `native-index://status` with the same payload: readiness, each
  reconciliation (a host a pass did not touch keeps its last scan), progress
  during the initial scan (at most every 250 ms) and the stop. Source paths stay
  out of it. Settings shows it; the shell invalidates its data queries on it.
- **Shutdown.** On the app's exit event the index is shut down before the
  database closes: the scan in progress is cancelled, a reader still running is
  killed and reaped, and the worker is awaited for at most 3 s; the final status
  reads `stopped`. A reader that never returns cannot hold the exit. What the
  cancelled scan committed stays; the next launch resumes Claude behind its
  checkpoints and reads Codex/Cursor whole.
- Scope kept out: dashboard metrics and the Sessions screen (they read the same
  index later), plugin delivery, cloud synchronization, a login-shell resolver,
  and any second parser, scheduler, database or capture path.

## Verification

| Area                         | Required checks and expected result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude filesystem            | Main/subagent mapping; ignored entries; partial lines; metadata-only default; repeated import without duplicates; unchanged originals; missing/permission/alias/name errors; cross-batch label consistency; corrected invalid files import successfully.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Stream and lifecycle         | Native identity/usage fixtures; malformed headers/records; no cross-session attribution; bounded batches; draining after stream failure; incomplete producer exit; partial rejection reports; exact-batch boundaries.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Checkpoints and tailing      | `claude_tail` suite: changes during the initial scan with no later event are indexed before ready; appends, completed partial lines, duplicate events, new files and whole new directories converge within seconds; truncation, replacement and in-place rewrite start new generations without duplicates; restart reads only appended input; a failed record transaction keeps the checkpoint behind the failed batch and the retry converges; originals unchanged; metadata-only rows; zero-position locators migrate by one full replay; an appended scan reports the surface the index holds and stops a record that disagrees with it; a watcher failure reports degraded; a root that appears later is watched before it is enumerated; a root created after its watches were decided is watched and scanned again before ready; the absent Cursor sibling root is seen through the parent watch; changes received before a stop request are reconciled; a root deleted and recreated at the same path is watched again; a root the platform reports removed is registered again; the Cursor hook's state pins are watched and a pin change alone is reconciled; a watcher error rebuilds every watch and rescans. Pinned conformance: every scan reads the whole host with no cutoff; a repeated scan adds nothing; a Codex session restored with an old clock, the original aged or rewritten in place with its clock put back, a store's `meta.json` rewritten with its clock put back and a hook state pin appearing are all read again with nothing new. Store: `native_checkpoints` suite (a batch without records cannot carry a checkpoint) and the atomicity failure list. CLI: `watch_native_once_reports_ready_and_resumes_on_the_next_run`; a closed event stream stops the run with exit 1 after the change was indexed; Ctrl-C with a single runtime worker still stops cleanly with `stopped`; a bound reached during the startup exits 1 as not ready. |
| Native locators              | Position remains zero for repeated, older and tied observations; old development offsets reset safely; empty undiscovered sessions create no locator; historical records survive shorter/empty replacements.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Storage ownership            | Mismatched host/native/conversation/surface/start facts reject before writes; existing batch atomicity and retention checks remain in force.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Source and executable safety | Database/source aliases reject before mutation; opened-file replacement and short-copy rejection; fixed snapshot under append; ignored Python modules excluded; stable interpreter selection.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Bundled readers              | `reader_bundle` suite: Git object identities computed as Git computes them (blob, executable blob, subtree, root; symlinks and bytecode caches refused); the vendored tree is the pinned objects without Git; edited, incomplete, extra-module, absent and pin-without-tree bundles are refused; a verified bundle reads a host in place.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Cancellation                 | `reader_cancel` suite: a hung pinned reader (fake interpreter) is killed and reaped on cancel, its completed session stays and the open one is ended early, the next host is not started; a tailer shut down during a hung initial scan ends within its bound publishing only `stopped`; cancel then graceful stop does not wait; a pre-cancelled scan reads nothing; a Claude scan cancelled between files keeps what it read and the next scan completes without duplicates.; an interpreter probe that never answers is killed at its bound and reported, killed at once by a cancel, and refused after one.; cancelling a reader launcher takes the descendant holding the stream with it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| App lifecycle                | `native_index` suite (app crate): a synthetic home indexes to ready with typed status, appends reconcile live, restart resumes without rereading, shutdown is prompt and final; a data directory inside the sources disables the index with the reason; a named interpreter that does not qualify and an absent bundle are reported while Claude indexes; a hung reader does not hold shutdown and is reaped; the compiled pin parses and the status serializes with tagged variants.; an interpreter that never answers its probe blocks neither startup (the status reads `resolving`) nor shutdown, and the probe is reaped.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Pinned conformance           | All five inventoried tests execute, including the real pinned reader-to-index import, deduplication, unchanged-source evidence and the bundled readers' byte equality and same-index import.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

```sh
cargo test -p xt-ingest --test claude_fs --test native --test reader_bundle --test reader_cancel --locked
cargo test -p xt-store --test completed_scan --locked
cargo test -p xt-server --test native_import --locked
cargo test -p xtrace-desktop --test native_index --locked
node scripts/ci/run-hook.mjs plugin-conformance
pnpm check:native --base FULL_REVIEWED_BASE_SHA
```

The built app is validated by hand against synthetic homes with no plugin
configuration or cloud credentials: `XTRACE_DATA_DIR`, `XTRACE_NATIVE_HOME` and,
for the hung-reader case, `XTRACE_PYTHON` pointing at a fake interpreter; the
database rows, source hashes and the Settings status are the evidence.

Use synthetic fixtures. Record the exact source/base, actual macOS version and
results in the PR. The portable Unix invalid-name helper test runs on macOS; the
real invalid-filename filesystem test is Linux-only because macOS rejects those
names. A macOS run does not claim that Linux-only case or minimum-macOS release QA.
