# Developing XTrace Desktop

## Toolchains

The native target is macOS 14.0 or newer. Install Xcode command-line tools and Rust through their official installers; `rust-toolchain.toml` selects Rust 1.94.1 with rustfmt and Clippy. Install Node 22.23.1 using your preferred version manager (`.node-version` is committed), then enable Corepack and select the pinned package manager:

```sh
corepack enable
corepack prepare pnpm@10.33.0 --activate
node --version
pnpm --version
rustc --version
```

The UI targets Safari 17. Direct dependencies are pinned and both Cargo and pnpm lockfiles are committed. Tauri core is 2.11.5, tauri-build 2.6.3, its JS API 2.11.1 and CLI 2.11.4; React is 19.2.8, Vite 8.2.2, TypeScript 5.9.3 and Tailwind 4.3.3. Tauri packages have independent patch versions.

## Fresh-clone checks

Run from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm check
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pnpm tauri build --debug --bundles app
git diff --exit-code -- Cargo.lock pnpm-lock.yaml
```

`pnpm check` runs TypeScript, ESLint with zero warnings, formatting and Vitest. `pnpm test -- --run` also works. Workspace compilation validates the empty subsystem crates; their APIs and behavior tests arrive with their implementations. UI tests exercise the shell's application-info contract. Broader build/test CI, browser E2E and shared fixtures remain future work; these commands do not claim those later gates have run.

Before sending repository content or a GitHub payload, follow
[Publication checks](docs/PUBLICATION.md). Run `pnpm publication:test` for the
synthetic gate tests and `pnpm security:scan` for reachable history and tracked
working files. Add `--diff <base>..<head>` for the actual PR range and
`--content <path>` for prepared outbound text. Public-content review also covers
comments, attachments and prior edits that those local scans cannot evaluate.

`cargo xtask --help` describes the currently available helper. Future fixture commands must report unavailable until their owning implementation exists.

## Running

`pnpm tauri dev` starts Vite and the native shell. `pnpm dev` alone is a browser preview at `http://127.0.0.1:5173`; it labels native app information as unavailable in the browser. The native build calls Rust directly and starts no local HTTP listener. Close another XTrace instance before testing this checkout, since the second launch intentionally focuses the existing app.

The debug bundle can be opened with:

```sh
open "target/debug/bundle/macos/XTrace Desktop.app"
```

## Brand assets

The UI uses `apps/desktop/ui/public/mark.png`; its official source and checksum
are recorded in [TRADEMARKS.md](TRADEMARKS.md#brand-asset-provenance). After a
frozen pnpm install, regenerate the native icons from the repository root:

```sh
node scripts/generate-icons.mjs
```

The script invokes the pinned Tauri CLI and updates `icon.png` and `icon.icns`
under `apps/desktop/ui/public/icons/` from the committed source mark. Rebuild the
debug bundle and inspect the UI mark and Finder/Dock icon before committing an
asset change.

## Native verification

At the default 1440×900 size and minimum 1120×720 size, confirm that the three native window controls sit above the sidebar brand and the main area reaches the top edge without a separate title strip. Drag the empty sidebar/header areas; Refresh must still work as a button. Verify minimize/restore, fullscreen/return and close/reopen. Launch the executable again and confirm it focuses the same process and window.

The controls use system spacing. Custom traffic-light offsets in the pinned Tauri version can reset after fullscreen, so the shell does not set them ([upstream issue](https://github.com/tauri-apps/tauri/issues/15451)).

This debug bundle is for local development. Signing, notarization, universal builds, oldest-supported macOS release QA and distribution belong to later release cards.
