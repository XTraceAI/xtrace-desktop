# Pinned plugin conformance acceptance

The required local native check validates the real capture producer at the one
revision named by `.plugin-pin`. The producer is never mocked; a skipped,
ignored, absent or zero-count test cannot satisfy the gate.

| Case                       | Setup/action                                                                                                                                          | Expected and observed result                                                                                                                                                               |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Pin resolution             | Run the hook with no `AGENT_PLUGINS_DIR`; then with a checkout at another commit; then with a pinned file edited in place.                            | The pinned commit is fetched without credentials into the ignored artifact directory; a foreign commit, a differing reader object or a dirty tree fails before any test runs.              |
| Unskipped execution        | Run the hook with Python and the checkout present; separately run `cargo test -- conformance` with no environment and feed that log to the validator. | The hook lists `executed 3 of 3 required conformance tests` by name. The plain log contains `SKIP` lines and the validator rejects it.                                                     |
| Missing prerequisites      | Run the hook with `PYTHON=/nonexistent`; delete `.plugin-pin`; remove a name from the log.                                                            | Each fails with a specific message; `run-hook.mjs` reports the declared hook missing when the script is absent while its signals remain.                                                   |
| Interpreter settings       | Run the hook with `PYTHONOPTIMIZE=1` exported; separately point `PYTHON` at a wrapper that forces `-O`.                                               | The hook clears the inherited variable and passes with assertions active; an interpreter that still optimizes is rejected before any test runs.                                            |
| Reader stream contract     | The pinned `readers_cli.py` reads F18/F20 native inputs in full and metadata-only mode from a disposable home with a network audit guard.             | Every session header carries the hand-written identity, surface, start, cwd and branch; record counts match; every record parses as a canonical record through `xt-ingest`; goldens match. |
| Incompatible reader change | The harness self-test copies the snapshot and changes the exported `conversation_id` format, then reruns the same checks.                             | The mutated producer fails with `session header shape changed`; a pin listing a different reader object fails source verification.                                                         |
| Release validation         | `Release native validation` checks out the producer at the workflow's `ref` and runs the same hook through `pnpm check:native --release`.             | The workflow test requires that `ref` to equal the `.plugin-pin` commit; no routine hosted macOS job is added.                                                                             |

## Verification

Run `node scripts/ci/run-hook.mjs plugin-conformance` on a macOS checkout with
Python 3.10+ and network access to the public producer repository, or export
`AGENT_PLUGINS_DIR` to an existing checkout at the pinned commit. The hook prints
the pinned producer commit and plugin version, the executed test names and the
`executed N of N` count. Record those lines with the native validation result.
Hosted Ubuntu CI runs only the pure pin, validator, hook-runner and workflow
tests (`pnpm test:ci`); it does not execute the producer.

The reader harness (`scripts/conformance/test-reader-stream.py`) accepts
`--self-test` to demonstrate the incompatible-change failure and `--write-golden`
to capture a new contract golden after a deliberate producer update. Review the
golden diff as part of the pin change; the goldens are the stream the Desktop
index consumer will be built against.
