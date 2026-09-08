# FND-09 native state and generated contract acceptance

The app owns one `Mutex<Store>`. `app_info` and `db_counts` return Rust DTOs from the app crate; xtask serializes those same structs and ts-rs 12.0.1 generates their TypeScript bindings. There is no handwritten TypeScript DTO mirror or separate fixture database builder.

## Startup and isolation

Normal startup resolves `XTRACE_DATA_DIR` before the platform app-data directory and opens `xtrace.db`. Fixture startup requires a debug build with the explicit `fixtures` feature and either `XTRACE_FIXTURE=F1` or `--fixture F1`; CLI selection takes precedence. Duplicate or missing fixture arguments fail.

Fixture mode resolves no default live directory. It creates a new temporary directory for each instance, optionally under the explicit override, materializes the populated canonical fixture, and owns that directory until the store closes. It never reuses an existing database. Listening is false, and this stage starts no probes or watchers. F2/F20 are unfinished skeletons and fail explicitly.

```sh
XTRACE_FIXTURE=F1 pnpm tauri dev --features fixtures
VITE_XTRACE_FIXTURE=F1 pnpm dev
pnpm fixtures:export
```

The first command is a developer launch instruction; acceptance below tests the state builder without opening a native window. Stop an existing development instance before changing startup options because single-instance activation preserves its running state.

## Executed checks and expected results

| Check                                                                            | Expected result                                                                                                                                                         |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test -p xtrace-desktop --all-features fixture_mode`                       | Unique fixture directories, F1 counts, temporary cleanup, explicit live override, persistent live DB, CLI validation and skeleton rejection pass without window launch. |
| `cargo test -p xtrace-desktop fixture_mode`                                      | A build without the optional feature rejects fixture selection before resolving live data.                                                                              |
| `cargo test -p xtrace-desktop generated`                                         | Counts up to 2^53−1 serialize exactly; 2^53 and larger fail in every count field.                                                                                       |
| `cargo test -p xtask generated`                                                  | Shared F1 DTO export matches committed JSON; F2/F20 shell exports fail; generated TypeScript directory matches exactly.                                                 |
| `bash scripts/ci/check-dto.sh`                                                   | Regeneration into a temporary directory agrees byte for byte, with no missing or extra export.                                                                          |
| `node --test scripts/ci/dto.test.mjs`                                            | Stale, missing and extra files fail; a linked root or linked repository parent fails before host files can substitute for committed declarations.                       |
| FND-02 `run-hook.mjs dto` against this source                                    | The installed `/bin/bash` hook actually executes and passes; it is not treated as an absent optional check.                                                             |
| `cargo check -p xtrace-desktop --release --locked --offline`                     | Normal production native configuration compiles against the built renderer.                                                                                             |
| `cargo check -p xtrace-desktop --release --features fixtures --locked --offline` | Fails specifically with the intentional compile-time fixture guard. This is an expected negative result.                                                                |
| `cargo tree -p xtrace-desktop --edges normal --prefix none --locked --offline`   | The normal application dependency tree excludes `xt-fixtures`.                                                                                                          |

The native and browser export comparison normalizes only `data_dir`: the browser uses `fixture://F1`, while a native fixture reports its actual owned temporary directory. Counts are one session, 25 canonical records and 15 raw usage rows. These are storage smoke counts, not deduplicated product metrics. No runtime path is committed in the fixture export.

The generated-file hook was executed through the reviewed FND-02 runner during integration. That runner becomes an in-repository entry point after its producer lands. CI's all-feature debug test job includes fixture tests; production uses its normal feature set.

These checks establish state-builder, serialization and compile-time boundaries. They do not claim a native window launch, WebDriver IPC end-to-end execution, platform-floor component qualification or product checkpoint approval. [Frontend evidence](FND-09-ui.md) covers the renderer and both browser engines.
