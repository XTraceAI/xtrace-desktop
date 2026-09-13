# Native source import acceptance

`xtrace-core import-native` reads a machine's Claude, Codex and Cursor history
into the local index without an installed plugin, a cloud login or any network
use. Codex and Cursor history is read by the exact shared readers named in
`.plugin-pin`; Claude history is read from its canonical JSONL directly. Every
host takes one path: the shared stream (one session header, then that session's
canonical records) is split per session and written through the transactional
canonical writer under the persisted content policy, which defaults to metadata
only. Native files are only ever opened for reading.

```sh
xtrace-core import-native --db PATH --home DIR [--host claude|codex|cursor]... \
  [--pin FILE] [--plugin-root DIR] [--python EXE]
```

The report is JSON on stdout. Exit 0 means every requested host imported every
record of every discovered session; exit 2 means a host or session was
unavailable, unreadable, incomplete, partial or malformed and says which; exit 1
means the command itself could not run. A session whose identity and accepted
records are stored but whose other records the writer rejected (a UUID owned by
another session, a type conflict) or that lacked a UUID is reported as `partial`
with each rejection named, and its host as `incomplete`. Reader output is
consumed as a stream: each session imports as soon as its block completes, so a
large history is never held in memory as a whole, and sessions read before a
later reader failure stay imported and are listed under that failure.

| Case                           | Setup/action                                                                                                                                                                            | Expected and observed result                                                                                                                                                                                                                                                                               |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude tree                    | A fake `~/.claude/projects` with a main transcript (25 F1 records, one structural line, a partial trailing line), a `subagents/` sidechain file, `memory/`, non-JSONL and hidden files. | Main and sidechain records land in the parent canonical session with native ID, `cli` surface, cwd and branch; sidechains keep `agentId`/`isSidechain`; only complete lines import; ignored trees stay ignored; the file cursor stops before the partial tail; `discovered_sessions` records the identity. |
| Reader streams                 | F18 Codex/Cursor goldens (store, IDE transcript, rollout) imported as the pinned producer emits them; F20 metadata-only headers.                                                        | One canonical session per native ID with surface, native start, cwd, branch, timestamps, model and tool counts; readable usage kept, absent usage null; metadata-only headers discover sessions without writing rows.                                                                                      |
| Metadata-only default          | Import under an unconfigured store and inspect every content column.                                                                                                                    | `content_json`, `tool_uses.input_json` and `sessions.title` stay NULL (reader titles derive from prompts and are never passed); IDs, lengths, counts and usage remain.                                                                                                                                     |
| Repeated import                | Run the same import twice.                                                                                                                                                              | The second run reports zero new records for every session and the row counts are unchanged.                                                                                                                                                                                                                |
| Unchanged sources              | Hash every file under the home before and after both runs.                                                                                                                              | Every byte and the set of files are identical.                                                                                                                                                                                                                                                             |
| Explicit gaps                  | Home without a host's directory; `--python` pointing at nothing; no plugin root; a checkout at another commit; a file whose records name another session; a malformed header or record. | `missing_source`, `missing_runtime`, `pin_mismatch` per host with a reason; `skipped` per session with the stream line; the remaining sessions still import and the process exits 2.                                                                                                                       |
| Partial sessions               | A stream whose session reuses a UUID owned by another session and contains a record without a UUID.                                                                                     | The session's own records import; the outcome is `partial` naming `rejected_ownership` and `missing_uuid`; the host is `incomplete`; the report is never `complete`.                                                                                                                                       |
| Pinned producer (native check) | `conformance_native_import` materializes F18's native inputs and runs the real pinned readers through `import_native`.                                                                  | Both hosts report `complete` with the pinned commit; sessions and counts equal the fixture expectations; a second run adds nothing; source bytes are unchanged. The test is in the required conformance inventory and cannot skip in the gate.                                                             |

## Verification

Run `cargo test -p xt-ingest --test claude_fs --test native --locked` and
`cargo test -p xt-server --test native_import --locked` for the synthetic trees,
streams, explicit results and the command. Run the required native check
(`node scripts/ci/run-hook.mjs plugin-conformance`) for the pinned-producer
case; it lists `conformance_native_import` among the executed tests.

Limitations of this change: it performs one initial read of the sources.
Incremental rescans, per-file byte cursors for reader hosts, file watching,
launch/focus scheduling, the runtime resolver for `python3` and the bundled
reader delivery belong to later work. The recorded `source_cursors` rows hold
the native locator (Claude: bytes through the last complete line; reader hosts:
the producer's update clock in milliseconds) as private local index metadata;
they are never part of a cloud or telemetry payload. The pin is verified with
`git` against the supplied checkout; a bundled, hash-verified reader is later work.
