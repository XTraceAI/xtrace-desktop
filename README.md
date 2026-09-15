# XTrace Desktop

XTrace Desktop is a macOS app for understanding work with coding agents. The
current build includes the native shell, shared UI components and gallery,
SQLite storage, and a headless importer for local Claude, Codex and Cursor history.
The importer defaults to metadata-only storage and leaves source files unchanged.
Automatic history indexing and real-data Dashboard/Sessions integration are still
in development; launching the app does not yet import your history.

There is no supported downloadable release yet. Build locally using the guide
below; do not expect an installer or Homebrew cask to be available.

Use macOS 14.0 or newer, Xcode command-line tools, Rust 1.94.1, Node 22.23.1 and pnpm 10.33.0:

```sh
pnpm install --frozen-lockfile
pnpm check
cargo test --workspace --locked
pnpm tauri dev
```

Build a local debug app with `pnpm tauri build --debug --bundles app`. The output is `target/debug/bundle/macos/XTrace Desktop.app`. See [DEVELOPING.md](DEVELOPING.md) for setup, all required checks and native verification, [the architecture map](docs/ARCHITECTURE.md) for subsystem ownership, and [desktop shell acceptance](docs/acceptance/desktop-shell.md) for expected results.

Source code is licensed under [Apache-2.0](LICENSE). [Trademark terms](TRADEMARKS.md) cover the XTrace name and logo; [contributions](CONTRIBUTING.md) use DCO sign-off.
