# Continuous integration

`ci-ok` aggregates source validation and policy diagnostics. Maintainers inspect
its run provenance and results before merging. Disclosure review remains manual;
`publication-content` is advisory and must not be configured as an enforcement
boundary. Candidate Actions workflows can imitate check names under the same
GitHub Actions app. Source publication tests and scanning run once in CI; the
standalone duplicate workflow has been removed. These files do not configure
repository rules or merge changes.

`CI` runs when a PR opens, reopens, receives code or becomes ready for review,
on merge groups, and on pushes to `main`. It has six substantive jobs and one
overall result:

| Job            | Coverage                                                                                  |
| -------------- | ----------------------------------------------------------------------------------------- |
| `policy`       | Reviewed source-commit DCO and substantive contribution description checks.               |
| `rust`         | Formatting, Clippy, tests and installed DTO/plugin integration hooks.                     |
| `ui`           | Types, lint, formatting, unit tests, WebKit and Chromium boot checks.                     |
| `security`     | CI/supply-chain/publication regression tests and source-history/diff scanning.            |
| `supply-chain` | Validated and scanned SBOM, license policy and reproducible notices.                      |
| `debug-bundle` | Actual native macOS debug build and launch.                                               |
| `ci-ok`        | Requires all six jobs to succeed; rejects failure, cancellation, missing or skipped jobs. |

`PR metadata` handles PR edits. Title/body-only edits run the same reviewed policy
validator without rebuilding the app or replacing the existing `ci-ok` result.
A changed target branch instead calls the full CI workflow against the new base.
Its full result appears under `base-ci / ci-ok`; review that run's current
head/base rather than an older source run. Separate concurrency groups prevent a
text edit from cancelling that full validation. A new source push runs normal CI
again. Current metadata policy failures still block the manual merge decision,
even when the unchanged source has a previous successful build.

Publication's default-branch advisory continues to handle mutable content and
reviews. Its workflow-completion signals follow `CI`, `PR metadata` and the
unchanged review-event relay. The repository-wide scan matrix runs on the default
branch; each PR receives only its own `publication-content-advisory` result.
PR lifecycle and text edits reach the scanner through workflow completion,
including failed or cancelled runs. Issue/comment events and periodic rescans
remain enabled. Dispatch manual rescans against the default branch; dispatches
against another branch or a tag skip the matrix.

This routing takes effect after merge. Existing workflow runs retain their old
results; merging does not rewrite them. Verify the next PR event produces a
default-branch advisory run and only a PR-specific publication result on its head.

Hook absence before implementation is explicit success with no conformance or
DTO coverage claimed; installed failures and removal still fail the Rust job
and aggregate.

The native jobs use `macos-14` and verify arm64 before the bundle smoke.
GitHub currently assigns that label to arm64, but the image is scheduled for
retirement on November 2, 2026. A replacement runner must preserve macOS 14 floor
testing; changing the deployment target is a separate decision.
See [runner specifications](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and the [macOS 14 image](https://github.com/actions/runner-images/blob/main/images/macos/macos-14-arm64-Readme.md).

## Reviewed merges while private

While the repository is private, maintainers review and merge one PR at a time
using squash merges. Before each merge, verify DCO,
`ci-ok`, advisory publication results and review findings against the exact
source head and current base. Complete the manual disclosure review in
[PUBLICATION.md](PUBLICATION.md), including retained content and attachments. Test the combined result against that base. If either source
or base advances, rerun the affected validation before merging. Missing reviewed
policy, failed source checks or unresolved blocking findings prevent a merge.
This procedure grants no merge or publication approval by itself.

Merge-queue activation is deferred until a separately approved public launch.
Keep `merge_group` workflow support and synthetic negative tests. Before enabling
the queue, demonstrate live group execution, trusted combined-tree scanning,
current disclosure invalidation and fail-closed aggregation. Start
with one PR per group and a 60-minute timeout, without redundant strict
up-to-date rebases. Queue-specific open findings remain activation blockers;
deferral does not mark them fixed.

## Contribution requirements

PRs use descriptive names and include substantive `## Change` and
`## Verification` sections. Describe the problem, resulting behavior, test setup,
actions, expected and actual results, and any checks still pending. Complete the
separate disclosure review in [PUBLICATION.md](PUBLICATION.md). Public acceptance
contracts describe observable behavior and belong in [acceptance/](acceptance/).

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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
pnpm publication:test
pnpm secrets:check
node scripts/supply-chain/install-tools.mjs syft
node scripts/supply-chain/install-tools.mjs cargo-about
pnpm sbom
pnpm notices:check
pnpm tauri build --debug --bundles app
node scripts/ci/debug-bundle.mjs 'target/debug/bundle/macos/XTrace Desktop.app'
```

Outside Actions, add the installed tool cache directories to `PATH`, or set `SYFT`
and `CARGO_ABOUT` to their executables. The pinned installer adds them to
`GITHUB_PATH` in Actions. Syft 1.51.1 produces the schema-validated CycloneDX 1.5
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

`node scripts/ci/run-hook.mjs plugin-conformance` invokes
`scripts/ci/plugin-conformance.sh` when installed. A `.plugin-pin` or
`assert-no-skipped-conformance.sh` also makes the runner mandatory. The hook must
use a pinned plugin checkout, configure its environment and reject skipped
conformance tests. `node scripts/ci/run-hook.mjs dto` invokes `scripts/ci/check-dto.sh` once
installed; a generated DTO directory makes that check mandatory. The DTO
hook must generate into temporary output and reject missing, extra or stale
committed exports. UI jobs consume those committed exports, and `ci-ok` waits for
parity. Any installed hook failure or removal of a default-branch declaration
fails CI.

Actions use pinned commits. Rust, pnpm, Playwright and pinned supply-chain tools
have caches. Automated dependency-update PRs are deferred until their generated commits and metadata meet
the sign-off, verification and disclosure requirements; no Dependabot schedule is installed by
this change. Dependency updates use the normal reviewed PR process. Cache timing,
manual combined-result validation and default-branch advisory invalidation need
recorded run evidence before this foundation is accepted. Live queue evidence
is required before queue activation, rather than during private manual merging.

Rust test and Clippy jobs enable all debug features so fixture-mode tests cannot silently disappear behind an optional feature. Production packaging uses its normal feature set and must still exclude debug fixture assets.
