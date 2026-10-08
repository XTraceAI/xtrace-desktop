# XTrace Desktop

XTrace Desktop is a macOS app for understanding work with coding agents. The
current build includes the native shell, shared UI components and gallery,
SQLite storage, and automatic indexing of local Claude, Codex and Cursor history.
Indexing defaults to metadata-only storage (plus a short, at most 280-character
preview of each message you typed and of each Claude Code background-task
summary) and leaves source files unchanged.
Sessions shows indexed metadata with search, host filters and pagination; Settings
reports indexing status and incomplete coverage. See [Sessions acceptance](docs/acceptance/indexed-sessions.md)
for the current behavior. Dashboard metrics and session detail remain in development.

Testing releases for Apple Silicon Macs running macOS 14 or newer are available
from [GitHub Releases](https://github.com/XTraceAI/xtrace-desktop/releases/latest).
The 0.1.4 source prepares the next release; publication is a separate step.
See [updates and testing limits](docs/UPDATES.md). A Homebrew cask is not available.

Use macOS 14.0 or newer, Xcode command-line tools, Rust 1.94.1, Node 22.23.1 and pnpm 10.33.0:

```sh
pnpm install --frozen-lockfile
pnpm check
cargo test --workspace --locked
pnpm tauri dev
```

Build a local debug app with `pnpm tauri build --debug --bundles app`. The output is `target/debug/bundle/macos/XTrace Desktop.app`. See [DEVELOPING.md](DEVELOPING.md) for setup, all required checks and native verification, [the architecture map](docs/ARCHITECTURE.md) for subsystem ownership, and [desktop shell acceptance](docs/acceptance/desktop-shell.md) for expected results.

Source code is licensed under [Apache-2.0](LICENSE). [Trademark terms](TRADEMARKS.md) cover the XTrace name and logo; [contributions](CONTRIBUTING.md) use DCO sign-off.
