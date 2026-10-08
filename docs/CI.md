# Continuous integration

Routine CI runs on Ubuntu 24.04 for PRs, merge groups and pushes to `main`.
Native validation runs locally before merging and on a hosted macOS 14 runner
only when explicitly preparing a downloadable release. Public disclosure review
remains manual. These workflows do not publish releases or change repository rules.

| Job        | Required hosted coverage                                                                   |
| ---------- | ------------------------------------------------------------------------------------------ |
| `policy`   | Reviewed source-commit DCO and substantive contribution descriptions.                      |
| `ui`       | Types, lint, formatting, unit tests, WebKit and Chromium boot tests.                       |
| `security` | CI/supply-chain/publication regressions and source-history/diff scanning.                  |
| `ci-ok`    | Requires all three jobs to succeed; rejects missing, skipped, cancelled or failed results. |

`ci-ok` certifies these hosted checks only. It does not certify native tests,
dependency notices, local evidence or disclosure approval. Maintainers review
those results separately before merging. Browser tests install their Linux system
dependencies even with cached binaries and do not certify native WKWebView behavior.

The UI job runs a small gallery smoke suite and production-exclusion checks.
The full 92-story gallery sweep runs locally before merging gallery, shared UI,
theme, font or brand-asset changes. Record its source SHA and results in the PR;
hosted success does not certify the full gallery. See [gallery checks](GALLERY.md#add-or-update-a-story).

`PR metadata` handles title/body edits with the same reviewed policy validator.
A target-branch edit calls the full Linux CI against the new base. Separate
concurrency groups prevent a text edit from cancelling base-change validation.
Review `base-ci / ci-ok` when present instead of relying on an older source result.

## Native validation before merging

Quit any running XTrace Desktop instance before the launch checks; the smoke
test does not terminate an existing app, and single-instance behavior prevents a
second copy from opening. Use a clean macOS checkout containing the exact combined source and current base.
After fetching the current target branch, record its full commit SHA and integrate
it into the candidate. Run:

```sh
pnpm check:native --base FULL_REVIEWED_BASE_SHA
```

The command requires a clean checkout and verifies the base is an ancestor of the
tested head. It installs frozen dependencies; runs workspace Rust fmt, strict
all-target/all-feature Clippy and tests; verifies installed DTO/plugin hooks
against a temporary copy of that base; generates and scans the SBOM; checks
notices; and builds and launches the debug app. It stops on the first failure.
The temporary hook baseline is removed on success and failure. Installed hooks
cannot disappear compared with the base; absent hooks claim no coverage.

Record the returned source SHA, base SHA and actual macOS version in the PR,
with test counts and relevant behavior checks. Local validation on a newer OS
is not macOS 14 compatibility evidence. Source changes or a dirty checkout during
the run invalidate the result. Before merging, verify the target branch still
matches the recorded base and repeat affected validation when either input advances.
Routine PRs require this local evidence; it is a maintainer review requirement,
not an automatically enforced status check. No self-hosted runner is installed.

## Native validation for a downloadable release

Only an explicit `workflow_dispatch` starts `Release native validation`.
Select the default branch for the workflow and supply the full SHA of a candidate
already merged into that branch. That path still requires the candidate to be
in default-branch history. Alternatively, manually select an explicitly reviewed
`refs/tags/vSemVer` tag (for example, `refs/tags/v0.1.1`) and supply its full
candidate commit SHA. The workflow source commit (`github.workflow_sha`), GitHub
source commit (`github.sha`), resolved tag commit and checked-out HEAD must all
equal that candidate. Non-default branches, other ref types, malformed versions
and mismatched identities fail. A tagged beta can be prepared and published from
that exact reviewed source without first merging its app source into `main`;
the workflow must already exist on the default branch to enable manual dispatch.
The workflow checks out the exact candidate without persistent credentials and
verifies actual macOS 14 arm64 on both paths.

The workflow resolves the candidate's first parent as its reviewed baseline and
runs `pnpm check:native --base REVIEWED_BASE_SHA --release`. Comparing against that
distinct predecessor detects installed hook declarations removed by the candidate;
using the candidate itself as its baseline would lose that protection. Earlier
merged changes still require their own recorded local validation before merging.
Release mode additionally builds release code with a unique test app identifier
and launches a separately signed disposable copy with its own data directory.
It does not launch the final signed public bundle.
The validation job uploads the validated, secret-scanned SBOM and has no signing
values.

On default-branch dispatches, only after validation passes does the `package`
job build the signed disk image. Reviewed tag dispatches skip this job. It runs
in the `macos-signing` GitHub environment. It compiles the release app without
any Apple values, then one step runs `node scripts/ci/release-dmg.mjs`, which:

1. runs `pnpm tauri bundle --bundles app,dmg` so Tauri signs the app with the
   hardened runtime and no extra entitlements, notarizes it and staples the ticket;
2. checks the app's Developer ID team, timestamp, hardened runtime, stapled ticket
   and Gatekeeper result, because Tauri only warns when notarization settings are
   incomplete;
3. mounts the disk image and checks the window layout with
   `scripts/ci/dmg-layout.mjs`: the app and the Applications shortcut must sit
   where `bundle.macOS.dmg` in `tauri.conf.json` places them, and the background
   picture must exist. Finder sometimes saves the window before it moves the icons,
   so a wrong layout is rebuilt up to three times before the job fails;
4. notarizes and staples the disk image, then requires
   `spctl -a -t open --context context:primary-signature`, `xcrun stapler validate`
   and `hdiutil verify` to pass and checks the mounted app again;
5. writes `artifacts/release/XTrace-Desktop-VERSION-macos-arm64.dmg` and its
   SHA-256 file, which the workflow uploads as the `dmg-CANDIDATE_SHA` artifact.

The workflow never creates or edits a GitHub release; attaching the disk image to
a release is the release owner's separate step. To run the layout check on any
local disk image, use `node scripts/ci/dmg-layout.mjs PATH_TO.dmg`. Local installs
and `pnpm check:native` still build with `--bundles app` only.

The `macos-signing` environment must require a maintainer's approval and holds
these secrets:

| Secret                       | Value                                                                        |
| ---------------------------- | ---------------------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Base64 of the exported Developer ID Application `.p12`, with its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting that `.p12`                                   |
| `APPLE_API_KEY`              | App Store Connect API key ID (10 characters)                                 |
| `APPLE_API_ISSUER`           | Issuer ID for that key                                                       |
| `APPLE_API_PRIVATE_KEY`      | Full contents of the key's `AuthKey_KEYID.p8` file                           |

The signing identity is fixed in the workflow. The script writes the `.p8`
contents to a private temporary file and deletes it when packaging ends.

Before publishing each downloadable version, require successful native release
validation for its exact source plus the separate acceptance of the final signed
and notarized downloadable bytes. Signing, notarization, universal builds,
Gatekeeper behavior and update delivery are not certified by this unsigned app
launch. Tag validation is source validation only, not disclosure approval or a
waiver of macOS 14 QA, final-byte acceptance or publication checks. Changes to
source or packaging invalidate the relevant release evidence.

Final signed launch from `/Applications` on a Mac that has never run the app
requires separate acceptance. The 0.1.4 testing release does not yet claim that
check.

`macos-14` is scheduled for retirement on November 2, 2026. Before retirement,
replace it with a runner that preserves actual macOS 14 floor testing. Building
with a deployment target of 14 on a newer OS does not test that floor.
See the [runner image](https://github.com/actions/runner-images/blob/main/images/macos/macos-14-arm64-Readme.md).

## Pinned plugin conformance

`.plugin-pin` names the one reviewed capture-producer revision Desktop is
validated against: the public `agent-plugins` repository, a full commit SHA,
the plugin root, the plugin's release version and the Git object IDs of the
reader sources the Desktop index will consume (`readers/`, `readers_cli.py`,
`cursor_flush.py`, and the whole `scripts` tree they import from, which the app
bundles as `vendor/agent-plugins` and verifies by the same object identities
without Git). `node scripts/ci/plugin-pin.mjs commit` prints a field;
`scripts/ci/plugin-pin.test.mjs` rejects malformed pins in hosted CI.

`scripts/ci/plugin-conformance.sh` owns the producer source, the environment
and unskipped execution. `AGENT_PLUGINS_SOURCE` defaults to `checkout`.
In checkout mode, without `AGENT_PLUGINS_DIR` it fetches exactly the
pinned commit from the public repository, without credentials, into the ignored
`artifacts/private/plugin-conformance/` directory; a supplied checkout is
accepted only at that commit. Either way the checkout must be clean and every
listed reader source must be the pinned object, so an edited file at the right
commit fails before any test runs.

Explicit `AGENT_PLUGINS_SOURCE=bundle` uses `vendor/agent-plugins` by default;
`AGENT_PLUGINS_DIR` can name a bundle's `plugins/memhub` directory instead.
Before any producer import or launch, a test-only Rust example calls the app's
existing `verify_bundle` against `.plugin-pin`: all four listed objects,
including the complete scripts tree with executable modes, must match. Extra
or missing files, altered bytes or modes, symlinks and bytecode caches fail.
The Python harnesses call that same verifier on their source and copied tree.
The gate runs six mutation regressions against all three Python harnesses.
It sets `PYTHONDONTWRITEBYTECODE=1` so imports cannot add caches to verified trees.
The explicit native release workflow selects this mode and uses the producer
bytes already in the candidate repository, with no external producer fetch.

Both modes require Python 3.10+ (`PYTHON` selects
the interpreter), clears `PYTHONOPTIMIZE`, `PYTHONPATH`, `PYTHONHOME` and
`PYTHONSTARTUP` so the harnesses' assertions and standard library stay intact
(an interpreter running with `-O` is rejected), runs
`cargo test --workspace --all-features --locked -- conformance --nocapture`
with both output streams captured to `artifacts/private/conformance/cargo-test.log`,
then runs `scripts/ci/assert-no-skipped-conformance.sh` on that log. The
validator requires every name in `scripts/ci/conformance-inventory.txt` to have
run and passed, and fails on any `SKIP` line, ignored test, failure, zero count
or missing name. Under a plain `cargo test` without the environment the
conformance tests print `SKIP` and return; that output can never satisfy the
gate. Missing source, missing Python, an unfetchable checkout pin or differing bundle object, a
removed hook or an absent named test are all red before merging.

The inventory currently requires:

| Test                               | Real contract exercised                                                                                                                                                                                                                                                                                                                                                     |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `conformance_flush_turn`           | The pinned Stop hook imports through both routing mechanisms, advancing its real cursor only after committed acknowledgements, including retry after a failed commit and metadata-only storage.                                                                                                                                                                             |
| `conformance_plugin_transport`     | The pinned decoder and token client against the headless binary: initialize, SSE `tools/list`, token mint/list/delete, notification and empty-import rejection.                                                                                                                                                                                                             |
| `conformance_native_reader_stream` | The pinned `readers_cli.py` over the fixture catalog's synthetic native Codex/Cursor files ([F18 and F20](FIXTURES.md)); see [acceptance](acceptance/plugin-conformance.md).                                                                                                                                                                                                |
| `conformance_native_import`        | The complete native import through the pinned readers into a disposable index: identity and counts per session, a repeated run adds nothing, source bytes unchanged; see [native import](acceptance/native-import.md).                                                                                                                                                      |
| `conformance_bundled_readers`      | Both modes verify the pinned bundled scripts and read F18 in place to its expected identities and counts, leaving sources unchanged. Checkout mode additionally compares every scripts file and LICENSE/NOTICE byte with the producer commit and compares the two independent indexes.                                                                                      |
| `conformance_exact_detail`         | The bundle's exact-detail mode reads F18's JSONL sessions and a synthetic paginated Codex group whole, matches ordinary export record for record, refuses the Cursor store beside a committed write-ahead log, and changes no source or temporary snapshot. Checkout mode additionally runs exact detail through the verified checkout and uses its own pagination fixture. |
| `conformance_codex_fork_import`    | A Codex conversation forked in Codex Desktop imports as its own session: its own ID and start, only its own records, and the original's counters at the cutoff as its usage baseline. Bundle mode supplies equivalent synthetic history data; checkout mode uses the pinned checkout's fixture.                                                                             |
| `conformance_tool_sent_import`     | The pinned bundle over a synthetic Codex and Cursor home: with `--automated-input-evidence` the lines differ from the plain export only by the evidence, and a scan into an index an earlier build filled binds one proof to each subagent notification, interrupted-turn note, app page record, Cursor summary and Cursor import banner, changing no record.               |

Record in the PR the pin commit printed by the hook and the `executed N of N`
line with its test names and the selected source mode. A bundle-mode pass does
not certify the independent checkout/index comparison, the producer checkout's
pagination fixture, or notices against upstream Git; run checkout mode for those
checks. A bundle execution is never reported as an independent checkout.
Every producer release train that Desktop adopts
updates `.plugin-pin`, the reader source
object IDs, the bundled copy (`sh scripts/vendor-readers.sh`, then the printed
scripts tree ID in the pin) and, when the reader stream changes on purpose, the fixture goldens
(`python3 scripts/conformance/test-reader-stream.py --write-golden`, then review
the diff). A pin whose readers emit a different stream fails the reader test
until the golden and the consumer are updated together.

## Disclosure and manual audits

Routine PRs have no disclosure checkbox or snapshot gate. Before changing the
repository to public, and again before the first downloadable release, run the
local comprehensive review after inspecting source/history, relevant PR and issue
text, comments and attachments. Follow [PUBLICATION.md](PUBLICATION.md) for the
procedure and token isolation. Ordinary PR events, comments, reviews, CI
completion and time passing do not start hosted publication scans. There is no
scheduled sweep or review-event relay. Old advisory results are not current
publication evidence.

`Publication content advisory` remains available for an explicitly requested
repository-wide audit through manual dispatch on the default branch. It reads
reviewed code and never executes a candidate's scanner with metadata credentials.
It has no automatic enforcement role; its GitHub Actions identity is spoofable.

## Reviewed merges

Serialize reviewed squash merges. Verify current head/base identities, hosted CI
provenance, DCO, local native results and unresolved findings together. Green
Linux CI alone does not authorize a merge or direct push.
Failed required local checks prevent merging just as failed hosted checks do.

Merge queues remain disabled pending separately approved activation and live
combined-result acceptance. Linux `merge_group` support and synthetic queue tests
remain, but they do not certify native or disclosure validation of a live queue.

## Contribution requirements

PRs use descriptive names and include substantive `## Change` and
`## Verification` sections. Describe the problem, resulting behavior, test setup,
actions, expected and actual results, and any checks still pending. Complete the
publication/release review in [PUBLICATION.md](PUBLICATION.md) before making the
repository or a downloadable artifact public. Public acceptance contracts
describe observable behavior and belong in [acceptance/](acceptance/).

The `security` job tests candidate code without API credentials. A separate
`policy` job checks out the default branch, or the full commit SHA in repository
variable `CI_POLICY_SHA`, before installing its pinned dependencies. Only that
reviewed source receives the read-only metadata token. The workflow itself
remains subject to manual provenance review.

For initial installation, a maintainer reviews the complete policy entrypoint,
its imports and dependency graph, and sets `CI_POLICY_SHA` to that exact 40-digit
commit SHA. Rerun CI on the same candidate after configuration. Missing policy
fails closed with a bootstrap diagnostic; a green source-only run is not live
policy validation. After the default branch contains the reviewed validator,
remove the bootstrap variable and verify a subsequent run uses the default
branch. Do not advance the variable automatically with PR pushes.

The validator resolves actual source PRs from merge-queue entries and reads each
PR's contributed commit list. Every author must have a matching final
`Signed-off-by` trailer. Bootstrap history and GitHub's synthetic queue commit are
not substitutes for those source commits. Incomplete API pagination, unknown
queue shapes and metadata changes during validation fail. The supported source
PR limit is 250 commits.

DCO and contribution descriptions are checked before merge. Main pushes validate
the resulting source, with policy limited to checking the push repository,
default branch and commit identities. They do not re-certify GitHub's synthesized
squash commit or reread mutable merged PR metadata. The repository must require
reviewed PR merges for this procedure; a green post-merge build does not
authorize a direct push.

## Local checks and integration hooks

```sh
pnpm check
pnpm test:ci
pnpm test:supply-chain
pnpm --dir apps/desktop/ui exec playwright install webkit chromium
pnpm e2e
pnpm check:native --base FULL_REVIEWED_BASE_SHA
pnpm publication:test
pnpm secrets:check
```

The native validation command installs the pinned tools and adds their cache
directories to its child-command `PATH`. For separate supply-chain commands, add
those directories to `PATH`, or set `SYFT` and `CARGO_ABOUT` explicitly. Syft 1.51.1 produces the schema-validated CycloneDX 1.5
artifact; cargo-about 0.9.2 and actual npm license texts produce notices. The active
license allowlist includes MPL-2.0, Zlib and Unicode-3.0. See
[dependency obligations](DEPENDENCY_LICENSES.md) for retained notices and
corresponding-source access; unsupported obligations still fail.
SBOM generation succeeds independently of notice approval. CI scans the generated artifact for secrets and uploads only the
validated SBOM, without browser traces or native logs.

The native smoke launches the built debug app, observes that exact process's
finished launch and main window through AppKit/CoreGraphics for two seconds,
then terminates only its own child. It requires an interactive macOS session and
no existing instance of the app. It does not inspect WKWebView content, tray
interaction or production packaging. Those checks belong to later native and
release acceptance.

`node scripts/ci/run-hook.mjs plugin-conformance` invokes the installed
`scripts/ci/plugin-conformance.sh`; `.plugin-pin` and
`assert-no-skipped-conformance.sh` also make the runner mandatory, so none of the
three can disappear compared with the reviewed base. See
[pinned plugin conformance](#pinned-plugin-conformance) for what the hook
requires and records. `node scripts/ci/run-hook.mjs dto` invokes `scripts/ci/check-dto.sh` once
installed; a generated DTO directory makes that check mandatory. The DTO
hook must generate into temporary output and reject missing, extra or stale
committed exports. UI jobs consume those committed exports; the local native result supplies
the separate parity evidence. Any installed hook failure or removal compared with the reviewed base
fails native validation.

Actions use pinned commits. Rust, pnpm, Playwright and pinned supply-chain tools
have caches. Automated dependency-update PRs are deferred until their generated commits and metadata meet
the sign-off, verification and disclosure requirements; no Dependabot schedule is installed by
this change. Dependency updates use the normal reviewed PR process. Cache timing and manual combined-result validation need recorded run evidence.
The release-only workflow uses the bundled producer under `vendor/agent-plugins`
and supplies its plugin root to the mandatory native conformance hook. Before the
first workspace test, native validation builds the bundle verifier and checks the
bundled producer's identity and file modes against `.plugin-pin`. The workflow
tests enforce bundle mode and bootstrap order. This adds no PR job.
The 0.1.3 release completed native validation for its exact source on macOS 14
arm64. Each later release needs a new exact-source result. Live queue evidence
is required before queue activation.

Native Rust tests and Clippy enable all debug features so fixture-mode tests cannot silently disappear behind an optional feature. Production packaging uses its normal feature set and must still exclude debug fixture assets.
