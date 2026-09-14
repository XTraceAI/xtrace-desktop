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
  table with **position zero**, meaning full replay. The timestamp is only a last-seen
  observation. There is no scan-order arbitration, byte-offset resume or mtime resume.
  Empty scans update an existing locator but create no new one. Failed/partial scans
  do not replace a locator. Reimports preserve indexed history when sources shrink.

## Source safety and scope

- The CLI validates database and SQLite-sidecar destinations before opening SQLite.
  Paths inside native history and existing multiply-linked destinations are rejected.
- Claude source roots, project aliases and transcript aliases are diagnosed instead
  of deliberately followed. Missing, unreadable and undecodable entries have explicit
  coverage results; blank session identities cannot enter discovery.
- On Unix, transcript opens use no-follow/nonblocking flags and check the enumerated
  device/inode before copying. A bounded anonymous snapshot must contain exactly the
  captured source length. Validation and import share its bytes; later appends wait
  for the next scan. Temporary contents are removed on close.
- Surface consistency is checked within each Claude file before writing. Dropped
  records still contribute known surface facts; unusable explicit labels are errors.
  Separate files with contradictory identities remain explicit conflicts; changing
  source files does not automatically replace historical index identities.
- Reader code executes from a temporary export of the verified Git commit, excluding
  ignored Python files/caches. On Unix, the probed interpreter is resolved to an
  absolute path before changing the reader's working directory.
- These checks do not provide a security boundary against another local process
  concurrently replacing ancestor directories. Directory-handle-relative traversal
  is not implemented. Source file bytes are never intentionally modified by import.

File watching, generation-aware resume, scan scheduling, reader bundling and UI are
subsequent work. An incremental consumer must validate source generation before
skipping input; neither the current locator nor its timestamp establishes it.

## Verification

| Area                         | Required checks and expected result                                                                                                                                                                                                                      |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude filesystem            | Main/subagent mapping; ignored entries; partial lines; metadata-only default; repeated import without duplicates; unchanged originals; missing/permission/alias/name errors; cross-batch label consistency; corrected invalid files import successfully. |
| Stream and lifecycle         | Native identity/usage fixtures; malformed headers/records; no cross-session attribution; bounded batches; draining after stream failure; incomplete producer exit; partial rejection reports; exact-batch boundaries.                                    |
| Native locators              | Position remains zero for repeated, older and tied observations; old development offsets reset safely; empty undiscovered sessions create no locator; historical records survive shorter/empty replacements.                                             |
| Storage ownership            | Mismatched host/native/conversation/surface/start facts reject before writes; existing batch atomicity and retention checks remain in force.                                                                                                             |
| Source and executable safety | Database/source aliases reject before mutation; opened-file replacement and short-copy rejection; fixed snapshot under append; ignored Python modules excluded; stable interpreter selection.                                                            |
| Pinned conformance           | All four inventoried tests execute, including the real pinned reader-to-index import, deduplication and unchanged-source evidence.                                                                                                                       |

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
