# FND-02 acceptance

Status: implementation draft. Required repository configuration and live required-check evidence remain pending.
The dependency policy includes MPL-2.0, Zlib and Unicode-3.0; live merge-queue
evidence is deferred until queue activation. This document does not
mark FND-02 complete or approve a checkpoint.

| Case                    | Setup and action                                                                                                                                                               | Expected result                                                                                                                                        | Evidence state                                                                                                                                                                                                                                                                                              |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source checks           | Frozen install; run UI, Rust and browser commands in [CI.md](../CI.md).                                                                                                        | Types, lint, formatting, unit tests and both browser boot projects pass on the checked source.                                                         | After producer integration: pnpm check, Rust fmt/strict workspace Clippy/workspace tests and both browser boot projects pass. The current Rust baseline has no tests; this is command/build evidence, not storage acceptance.                                                                               |
| DCO and queue selection | Disposable real Git history includes unsigned bootstrap, signed contribution and unsigned contribution; select actual PR commits through synthetic PR and queue API responses. | Signed source passes; unsigned contributed source fails for both events. Bootstrap/synthetic queue text cannot satisfy or defeat source certification. | Local policy tests pass; actual unsigned-PR and queue run URLs pending.                                                                                                                                                                                                                                     |
| Checkpoint transitions  | Exercise producer stages, later stages, missing/configured person approvals, changed identities and repair exceptions.                                                         | Producers remain possible; dependent work requires exact current approval. Repair is current-head-only and never publication permission.               | Local policy tests pass; configured issues/approvers and live evidence pending.                                                                                                                                                                                                                             |
| Aggregate failures      | Change each required job result to failure, cancellation, skip or missing in synthetic test data.                                                                              | Aggregate fails in every case; only explicitly inapplicable skips are allowed by the helper. Current workflow allows none.                             | Local negative tests pass; the first live run correctly failed ci-ok when dependent gates failed. Queue execution remains pending.                                                                                                                                                                          |
| Integration hooks       | Run real temporary shell hooks that succeed/fail; remove a default-branch declaration.                                                                                         | Absent pre-owner hook reports no coverage; installed failure, missing declaration and removal fail.                                                    | Local negative tests pass. FND-09/FND-13 coverage is not installed yet.                                                                                                                                                                                                                                     |
| Native bundle           | Build the actual debug `.app`, launch it and inspect its own AppKit process/window state.                                                                                      | Finished native launch and main window persist for two seconds; only the created process is terminated.                                                | Debug .app builds successfully; Swift inspector compiles and invalid arguments fail. Actual macOS 14 CI native launch passes in run 34217438128. This is not WKWebView-content or tray QA.                                                                                                                  |
| SBOM/notices            | Generate from the frozen Cargo/pnpm graph; validate schema and uniqueness; compare actual notice texts with the committed file. Add drift/invalid-license fixtures.            | CycloneDX 1.5 includes both ecosystems once; missing texts, unsupported licenses and drift fail.                                                       | Nine local supply-chain cases pass. Syft 1.51.1 produces a valid 812-component / 802-PURL SBOM. Both pinned tool installers pass locally. The current policy accepts MPL-2.0, Zlib and Unicode-3.0 with preserved notices and versioned MPL source links; candidate release packaging remains a later gate. |
| Disclosure and secrets  | Run publication/scanner tests and scan the exact source history, constituent diffs and proposed outbound content.                                                              | Synthetic known-secret history and fixture-path cases fail with redacted diagnostics; changed public content invalidates its reviewed snapshot.        | Producer tests cover the negative cases; integrated source/history and generated-SBOM scans pass; default-branch event evidence pending. Semantic and attachment review remain separate.                                                                                                                    |
| CI operation            | Run an actual PR and merge group, download and validate SBOM, exercise negative gates and measure a warm-cache run.                                                            | `ci-ok` plus `publication-content` match required checks; failures block; warm-cache wall time is under 12 minutes.                                    | First live PR run completed and its downloaded SBOM validated. Full green, queue and warm-cache evidence remain pending. No repository settings are changed.                                                                                                                                                |

Local CI policy validation: 15 tests passed in the integrated branch. All 48 publication/scanner tests and 4 UI cases pass; WebKit and Chromium each pass the boot smoke. The tests include real disposable Git commits and actual shell hook failures; GitHub queue and checkpoint API responses are synthetic. Owned JavaScript lint/format checks and actionlint 1.7.12 passed. No local app was launched.

## First live CI run

[Run 34217438128](https://github.com/XTraceAI/xtrace-desktop/actions/runs/34217438128)
tested source `61b61897841e6dbc1c7d3842045e16dbdfb96085` through GitHub merge
commit `4e3251637866eb523a6a730f3e9736406309b68d`. Rust, UI, integration hooks
and the native debug build/launch job passed on the macOS 14 runner.

The downloaded `sbom-4e3251637866eb523a6a730f3e9736406309b68d` artifact
contains `sbom.cdx.json` with 812 components and 802 package URLs. Independent
offline schema, identity, path and secret checks pass. Its SHA-256 is
`58260ad95009ece3f760e998c4d1c5c02b1724610f766b346693f45c858ea2dd`.

The run remains failed: notice policy and trusted-policy bootstrap are unresolved,
and the publication snapshot needed refreshing. Chromium passed; WebKit exposed
a runner mismatch. [Playwright 1.59 removed macOS 14 WebKit support](https://playwright.dev/docs/release-notes#version-159),
so the browser runner is now pinned to 1.58.2 while retaining the macOS 14 floor.
[Run 34218827020](https://github.com/XTraceAI/xtrace-desktop/actions/runs/34218827020)
verifies the correction on macOS 14: both WebKit and Chromium, Rust, UI, hooks
and native debug build/launch pass. It tested source
`4e69d1abf3aaa3d62021e3cdf5b96c9ef885247a` through merge commit
`4f3eeffdd180bc81bacbc7be2e0ca012a0aaabfb`. The overall run remains failed on
license policy, trusted-policy bootstrap and a stale disclosure snapshot.

Its downloaded SBOM has 813 components / 803 package URLs and passes the same
four validation steps.

Artifact SHA-256:
`2c2cad7d568304881f531067ff214371889736e254faae17187391694843ae44`.
The extra package follows the compatible browser runner's frozen dependency graph.

Next: resolve trusted-policy configuration, refresh the linked disclosure
snapshots, then collect required-check, manual combined-result and warm-cache
evidence. Live queue acceptance is required before queue activation. Keep outbound evidence synthetic and follow [publication review](../PUBLICATION.md).

## Current policy and merge strategy

The license allowlists and generated notices now cover the existing dependencies.
`pnpm notices:check` must reproduce the committed file, including versioned source
links for MPL-covered Rust packages. See [dependency obligations](../DEPENDENCY_LICENSES.md).
Historical failed runs above remain historical evidence, not current success.

Private integration uses the serialized reviewed procedure in [CI.md](../CI.md).
Queue-only acceptance remains open until activation; trusted-policy bootstrap,
current required-check evidence and blocking review findings remain prerequisites
for merging. Neither this document nor the license policy approves a checkpoint.

Current policy validation: `pnpm check` passes typecheck, lint, formatting and four UI cases; `pnpm test:ci` passes 15 policy/hook cases; `pnpm test:supply-chain` passes nine cases. Fresh notice generation and `pnpm notices:check` pass. All five MPL source archives download successfully and match their locked SHA-256 checksums. No native app rebuild or launch is required for these policy/documentation changes.
