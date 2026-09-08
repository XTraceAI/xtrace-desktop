# XTrace Desktop

XTrace Desktop is a macOS app for understanding work with coding agents. This first foundation PR supplies the buildable application shell, native sidebar window controls, Rust workspace, pinned toolchains and contribution files. The shell displays application information; capture, storage, metrics and other product subsystems are introduced by later PRs.

Use macOS 14.0 or newer, Xcode command-line tools, Rust 1.94.1, Node 22.23.1 and pnpm 10.33.0:

```sh
pnpm install --frozen-lockfile
pnpm check
cargo test --workspace --locked
pnpm tauri dev
```

Build a local debug app with `pnpm tauri build --debug --bundles app`. The output is `target/debug/bundle/macos/XTrace Desktop.app`. See [DEVELOPING.md](DEVELOPING.md) for setup, all required checks and native verification, [the architecture map](docs/ARCHITECTURE.md) for subsystem ownership, and [FND-01 acceptance](docs/acceptance/FND-01.md) for expected results.

Source code is licensed under [Apache-2.0](LICENSE). [Trademark terms](TRADEMARKS.md) cover the XTrace name and logo; [contributions](CONTRIBUTING.md) use DCO sign-off.
