# Continuous integration

The required checks are `ci-ok` for source validation and `publication-content`
for current disclosure review. The temporary `publication-checks` workflow remains
available during integration; its immutable checks are also included in `ci-ok`.
These files do not configure repository rules or merge changes.

`CI` runs on pull requests, merge groups and pushes to `main`. It checks Rust
formatting, Clippy and tests; UI types, lint, formatting and tests; WebKit and
Chromium boot cases; source-commit DCO and checkpoint policy; publication scanning;
SBOM/notices; native debug bundle startup; and installed integration hooks.
`ci-ok` always runs and rejects failed, cancelled, missing and skipped jobs.
Hook absence before its implementation is an explicit successful result, with no
conformance or DTO coverage claimed.

The native jobs use `macos-14` and verify arm64 before the bundle smoke.
GitHub currently assigns that label to arm64, but the image is scheduled for
retirement on November 2, 2026. A replacement runner must preserve macOS 14 floor
testing; changing the deployment target is a separate decision.
See [runner specifications](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and the [macOS 14 image](https://github.com/actions/runner-images/blob/main/images/macos/macos-14-arm64-Readme.md).

## PR classification and acceptance

Every scheduled PR includes a standalone paragraph such as `Plan slot: FND-03`
and uses the matching `feat/fnd-03-` branch prefix. The reviewed publication repair
uses `Maintenance: publication-checks` and the exact `feat/publication-checks`
branch. Unknown classifications fail. Include substantive `## Change` and
`## Verification` sections describing setup, action, expected results, commands
actually run and remaining checks. Complete the separate disclosure review in
[PUBLICATION.md](PUBLICATION.md).

`scripts/ci/stages.json` contains only technical stage/dependency data and named
maintenance exceptions. It does not schedule work or record completion. CI reads
that file through GitHub at the current default-branch SHA, or the exact reviewed
SHA configured in repository variable `CI_POLICY_SHA`. A candidate cannot redefine
its own stage through its body or its edited map. Initial installation requires
an explicit reviewed policy SHA until the map exists on the default branch;
missing trusted policy fails closed.

The gate resolves actual source PRs from merge-queue entries and reads each PR's
contributed commit list. Every author must have a matching final `Signed-off-by`
trailer. Bootstrap history and GitHub's synthetic queue commit are not substitutes
for those source commits. Incomplete API pagination, unknown queue shapes and
metadata changes during validation fail. The supported source PR limit is 250
commits. Main pushes check the actual push comparison, with the same limit.

## Checkpoint evidence

Stages 1–2 can produce CP1 without an existing approval. CP1 gates stage 3 onward,
CP3 stage 7 onward, CP5 stage 12 onward and CP7 stage 15 onward. CP7 additionally
guards release publication. A release workflow must invoke the publication intent
with the exact candidate artifact identities; this CI change does not implement a
release workflow or grant release authorization.

Repository variable `CHECKPOINT_CONFIG` supplies the configured checkpoint issue
numbers and authorized human approver logins:

```json
{ "approvers": ["maintainer-login"], "issues": { "CP1": 17, "CP3": 18, "CP5": 19, "CP7": 20 } }
```

These are illustrative values, not configured accounts or approvals. Missing
configuration blocks dependent stages. Each required checkpoint issue must be
linked in the PR body, for example `Checkpoint evidence: #17`, so current
publication-content review includes changes to its evidence and discussion.

The open checkpoint issue body is a JSON evidence identity:

```json
{
  "checkpoint": "CP1",
  "source": "<40-character tested source SHA>",
  "artifacts": [],
  "decisions": "<64-character SHA-256 of accepted decisions>"
}
```

A configured person posts that same JSON with `"status":"approved"` added to
explicitly approve it. Artifact entries are SHA-256 digests; publication requires
at least one. Hash the exact reviewed decision document bytes and attach or link
that document after disclosure review. The latest explicit checkpoint decision
from a configured person wins, including rejection or revocation. Bots, labels,
quotes, examples, timeouts and agent disclosure attestations do not grant approval.
The demonstrated source must be part of the PR's base history.

Update the issue identity after a substantive change to demonstrated behavior,
artifacts or accepted decisions; an older approval then fails. CI compares the
declared identities and rereads current evidence, but cannot determine semantic
equivalence or decide what a person approved. Checkpoint review remains the
approver's responsibility. Publication-content event delivery and reconciliation
limitations also apply; run current checks again before a publication decision.

A configured person may approve a repair-only exception by posting a JSON PR
comment with `checkpoint`, `kind: "checkpoint-fix"`, `status: "approved"` and
`source` equal to that PR's current head SHA. A push invalidates it. This exception
never permits publication and never changes the checkpoint approval itself.

## Local checks and integration hooks

```sh
pnpm check
pnpm test:ci
pnpm test:supply-chain
pnpm --dir apps/desktop/ui exec playwright install webkit chromium
pnpm e2e
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
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
license allowlist is enforced, including when it rejects an existing dependency.
SBOM generation succeeds independently of notice approval. CI uploads only the
validated SBOM, without browser traces or native logs.

The native smoke launches the built debug app, observes that exact process's
finished launch and main window through AppKit/CoreGraphics for two seconds,
then terminates only its own child. It requires an interactive macOS session and
no existing instance of the app. It does not inspect WKWebView content, tray
interaction or production packaging. Those checks belong to later native and
release acceptance.

`node scripts/ci/run-hook.mjs plugin-conformance` invokes
`scripts/ci/plugin-conformance.sh` once FND-13 installs it. A `.plugin-pin` or
`assert-no-skipped-conformance.sh` also makes the runner mandatory. FND-13 owns
the pinned plugin checkout, environment and rejection of skipped conformance
tests. `node scripts/ci/run-hook.mjs dto` invokes `scripts/ci/check-dto.sh` once
FND-09 installs it; a generated DTO directory makes that check mandatory. The DTO
hook must generate into temporary output and reject missing, extra or stale
committed exports. UI jobs consume those committed exports, and `ci-ok` waits for
parity. Any installed hook failure or removal of a default-branch declaration
fails CI.

Actions use pinned commits. Rust, pnpm, Playwright and pinned supply-chain tools
have caches. Dependabot groups weekly Cargo, npm and Actions updates. Cache timing,
live queue behavior and default-branch content invalidation need recorded run
evidence before this foundation is accepted.
