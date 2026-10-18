# Development guide

Run commands from the repository root. Native development needs macOS 14 or newer,
Xcode command-line tools, and Rust installed through rustup. The repository pins
Rust in `rust-toolchain.toml`, Node in `.node-version`, and pnpm in `package.json`.
Install the pinned Node version, then select pnpm:

```sh
corepack enable
corepack prepare pnpm@10.33.0 --activate
pnpm install --frozen-lockfile
pnpm check
```

## Run and inspect

```sh
pnpm tauri dev
```

This launches the native shell. Close another XTrace instance first: a second
launch focuses the existing app. The app indexes the native history under your
home at launch (metadata only, into its own database) and keeps it current;
`XTRACE_NATIVE_HOME=DIR` indexes a synthetic home (an existing directory) instead, `XTRACE_PYTHON=EXE`
names the interpreter for the Codex/Cursor readers, and `XTRACE_DATA_DIR=DIR`
relocates the database. Settings shows the index status. See
[architecture](ARCHITECTURE.md) for implemented and planned boundaries.

Usage telemetry ([TELEMETRY.md](../TELEMETRY.md)) is built in: every build
includes XTrace's public, write-only PostHog project token and the US host
`https://us.i.posthog.com`. When compiling, `XTRACE_POSTHOG_KEY` sends to
another project and `XTRACE_POSTHOG_HOST` to another PostHog host (set both
for a project on another host); `XTRACE_POSTHOG_KEY=""` builds an app that
never sends anything. Only an app built with the identifier `ai.xtrace.app`
and using its own data folder sends, and only after its notice was shown and
while telemetry is on; that includes `pnpm tauri dev` and local debug builds
on the default data folder, which use that identifier. The identifier checked
is the one compiled in, so editing a copy's `Info.plist` afterwards does not
change it. Copies built with another identifier, runs with `XTRACE_DATA_DIR`
naming another folder, and fixture mode never send. `XTRACE_TELEMETRY=0` turns
telemetry off at run time. Whether the PostHog
project stores IP addresses is a project setting outside this repository;
check it against TELEMETRY.md before a release.

For UI development without a native build:

```sh
VITE_GALLERY=1 pnpm dev
```

Open the printed Vite address at `/gallery`. Gallery stories use illustrative
component data, not your sessions. For a synthetic application data source, use
`VITE_XTRACE_FIXTURE=F1 pnpm dev`; native fixture mode is
`XTRACE_FIXTURE=F1 pnpm tauri dev --features fixtures`.
See [data sources](DATA-SOURCE.md), [gallery](GALLERY.md), and [design](DESIGN.md).

## Validate a change

`pnpm check` runs UI types, lint, formatting and unit tests. Add a focused failing
regression before changing behavior, then run the relevant
[acceptance contract](acceptance/) and record actual results.

```sh
pnpm check
pnpm test:ci
pnpm check:native --base FULL_REVIEWED_BASE_SHA
```

The last command requires a clean committed macOS checkout with that base
integrated. It runs Rust checks, installed conformance/DTO hooks, supply-chain
checks, and debug build/launch. The pinned producer is fetched when needed;
Python 3.10+ is required for reader conformance. Do not substitute plain Cargo
tests that skip a missing producer for that gate.

For UI changes, install browsers with
`pnpm --dir apps/desktop/ui exec playwright install chromium webkit`, then run
`pnpm e2e`, `pnpm e2e:gallery:smoke`, and `pnpm e2e:production`.
Shared UI/gallery changes also need `pnpm e2e:gallery` and visual review; see
[visual testing](VISUAL_TESTS.md) for the `pnpm parity` workflow.
[CI](CI.md) owns the full check matrix: routine hosted checks run on Ubuntu;
macOS checks run locally, or explicitly for release preparation.

## Fixtures and native details

`cargo xtask --help` lists fixture and DTO commands. Run
`cargo xtask fixture-validate` to distinguish populated fixtures from skeletons.
[FIXTURES.md](FIXTURES.md) is the fixture contract; [native import acceptance](acceptance/native-import.md)
describes testing the headless importer against synthetic native trees.

[Native development](NATIVE-DEVELOPMENT.md) covers icon generation and window QA.
[Publication review](PUBLICATION.md) covers the final public-source/release audit;
routine PRs have no disclosure checkbox or snapshot requirement.
