# XTrace Desktop

XTrace Desktop is a macOS beta for understanding your work with coding agents.
It indexes local Claude Code, Codex and Cursor history and brings sessions,
activity, usage and linked pull requests into one app.

[Download the current update release](https://github.com/XTraceAI/xtrace-desktop/releases/latest)
for **Apple Silicon Macs running macOS 14 or newer**. Open the release's `.dmg`,
drag **XTrace Desktop** to **Applications**, then open it from Applications.
Read the release notes for the checks completed on that download and any limits.
Newer testing candidates, when available, appear on [all releases](https://github.com/XTraceAI/xtrace-desktop/releases);
GitHub's latest-release link and automatic updates can remain on an earlier release.
There is no Homebrew cask or Intel Mac release. See [setup](docs/SETUP.md) for
first launch, optional integrations and troubleshooting, and [updates](docs/UPDATES.md)
for automatic and manual updates.

## What you can use

- **Dashboard:** daily agent time and estimated API-equivalent cost, leverage,
  concurrency, hands-off time, merged pull requests and recent session activity.
  Definitions explain how each number is counted and what is missing. Cost is
  an estimate at published API rates, not your subscription bill.
- **Sessions:** searchable history with host filters, session details, activity
  timelines and verified parent/sub-session groups. Claude transcripts can be
  read from their original local files; Codex and Cursor transcript viewing is
  unavailable in this build, while their indexed metrics remain available.
- **Pull requests:** merged-PR reports and a cached inventory of linked PRs.
  GitHub checks use your installed, signed-in GitHub CLI.
- **Usage and menu bar:** Claude and Codex account-limit readings where the local
  tools can supply them, plus a menu-bar view of today's activity.
- **Rulebook:** recorded local rule activity. This view does not edit or enable
  rules, and missing outcomes remain unknown.
- **Settings:** light/dark/system appearance, typing speed and human-time break
  length, indexing status, local storage and content deletion controls.

This is a beta. Missing source history can leave measurements incomplete;
unknown values are not zero. Some nested Guardian conversations can appear under
the root conversation instead of their immediate parent. Leaderboard and rule
editing are not implemented. See [testing limits](docs/UPDATES.md#beta-testing-limits).

## Your data

The local index defaults to metrics and metadata, with previews of up to 280
characters for each message classified as yours and each recognized Claude Code
background-task summary. Full transcript archiving is off by default. Settings
can enable it for future imports or delete saved content and previews. Indexing
and content deletion leave original conversation files unchanged.

The app has no cloud transcript upload. Anonymous usage telemetry (see
[Telemetry](#telemetry)), account-usage reads, GitHub checks and release updates
can contact their providers. Claude
usage checks run your local Claude Code CLI, which can save its own configuration
and probe files. See [data and network details](docs/SETUP.md#data-and-network)
before treating the app as offline or its database as free of sensitive text.

## Telemetry

XTrace Desktop sends a few anonymous usage events, such as when the app is opened
and roughly how often it is used, with the app version, the macOS version, which
coding agents have history on this Mac and a random ID. It never sends repository
names, file paths, prompts or other conversation text, pull-request links, token
counts or costs. Telemetry is on by default, and nothing is sent until the app has
shown a notice about it. Turn it off with **Turn off** in that notice, the switch in
Settings, or by starting the app with `XTRACE_TELEMETRY=0`. [TELEMETRY.md](TELEMETRY.md)
lists every event and property.

## Build and contribute

Use macOS 14 or newer, Xcode command-line tools, Rust 1.94.1, Node 22.23.1 and
pnpm 10.33.0. From a source checkout:

```sh
pnpm install --frozen-lockfile
pnpm check
cargo test --workspace --locked
pnpm tauri dev
```

Development builds read local history by default. See [DEVELOPING.md](DEVELOPING.md)
for synthetic data, all required checks and native verification. A local debug
app can be built with `pnpm tauri build --debug --bundles app`; its output is
`target/debug/bundle/macos/XTrace Desktop.app`. Building source does not publish
a release or enable the public updater. The release page, rather than this
checkout's version, identifies the currently downloadable beta.

See the [architecture map](docs/ARCHITECTURE.md) for subsystem ownership and
[contributor guide](CONTRIBUTING.md) for review and DCO sign-off. Source code is
licensed under [Apache-2.0](LICENSE); [trademark terms](TRADEMARKS.md) cover the
XTrace name and logo.
