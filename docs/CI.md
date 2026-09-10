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
already merged into that branch. The workflow rejects malformed identities,
non-default workflow refs and candidates outside default-branch history. It checks
out the exact candidate without persistent credentials and verifies macOS 14 arm64.

The workflow resolves the candidate's first parent as its reviewed baseline and
runs `pnpm check:native --base REVIEWED_BASE_SHA --release`. Comparing against that
distinct predecessor detects installed hook declarations removed by the candidate;
using the candidate itself as its baseline would lose that protection. Earlier
merged changes still require their own recorded local validation before merging.
Release mode additionally builds and launches the production app configuration.
Only the validated, secret-scanned SBOM is uploaded. The recorded candidate SHA
identifies the tested source; the workflow does not publish a downloadable app.

Before publishing each downloadable version, require successful native release
validation for its exact source plus the separate acceptance of the final signed
and notarized downloadable bytes. Signing, notarization, universal builds,
Gatekeeper behavior and update delivery are not certified by this unsigned app
launch. Changes to source or packaging invalidate the relevant release evidence.

`macos-14` is scheduled for retirement on November 2, 2026. Before retirement,
replace it with a runner that preserves actual macOS 14 floor testing. Building
with a deployment target of 14 on a newer OS does not test that floor.
See the [runner image](https://github.com/actions/runner-images/blob/main/images/macos/macos-14-arm64-Readme.md).

## Disclosure and manual audits

Run the local current-content disclosure check immediately before every merge,
after reviewing source/history, final PR text, linked records, comments and
attachments. Follow [PUBLICATION.md](PUBLICATION.md) for token isolation and
snapshot preparation. Ordinary PR events, comments, reviews, CI completion and
time passing do not start hosted publication scans. There is no scheduled sweep
or review-event relay. Old advisory results may remain on existing PRs and are
not current approval evidence.

`Publication content advisory` remains available for an explicitly requested
repository-wide audit through manual dispatch on the default branch. It reads
reviewed code and never executes a candidate's scanner with metadata credentials.
It has no automatic enforcement role; its GitHub Actions identity is spoofable.

## Reviewed merges

Serialize reviewed squash merges. Verify current head/base identities, hosted CI
provenance, DCO, local native results, current disclosure review and unresolved
findings together. Green Linux CI alone does not authorize a merge or direct push.
Failed required local checks prevent merging just as failed hosted checks do.

Merge queues remain disabled pending separately approved activation and live
combined-result acceptance. Linux `merge_group` support and synthetic queue tests
remain, but they do not certify native or disclosure validation of a live queue.

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

`node scripts/ci/run-hook.mjs plugin-conformance` invokes
`scripts/ci/plugin-conformance.sh` when installed. A `.plugin-pin` or
`assert-no-skipped-conformance.sh` also makes the runner mandatory. The hook must
use a pinned plugin checkout, configure its environment and reject skipped
conformance tests. `node scripts/ci/run-hook.mjs dto` invokes `scripts/ci/check-dto.sh` once
installed; a generated DTO directory makes that check mandatory. The DTO
hook must generate into temporary output and reject missing, extra or stale
committed exports. UI jobs consume those committed exports; the local native result supplies
the separate parity evidence. Any installed hook failure or removal compared with the reviewed base
fails native validation.

Actions use pinned commits. Rust, pnpm, Playwright and pinned supply-chain tools
have caches. Automated dependency-update PRs are deferred until their generated commits and metadata meet
the sign-off, verification and disclosure requirements; no Dependabot schedule is installed by
this change. Dependency updates use the normal reviewed PR process. Cache timing and manual combined-result validation need recorded run evidence.
The first hosted release dispatch remains pending until release preparation. Live queue evidence
is required before queue activation, rather than during private manual merging.

Native Rust tests and Clippy enable all debug features so fixture-mode tests cannot silently disappear behind an optional feature. Production packaging uses its normal feature set and must still exclude debug fixture assets.
