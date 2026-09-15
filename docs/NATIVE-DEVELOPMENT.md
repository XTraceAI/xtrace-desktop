# Native development details

Run commands from the repository root. See [the development guide](DEVELOPING.md) for setup.

## Brand assets

The UI uses `apps/desktop/ui/public/mark.png`; its official source and checksum
are recorded in [TRADEMARKS.md](../TRADEMARKS.md#brand-asset-provenance). After a
frozen pnpm install, regenerate the native icons from the repository root:

```sh
node scripts/generate-icons.mjs
```

The script invokes the pinned Tauri CLI and updates `icon.png` and `icon.icns`
under `apps/desktop/ui/public/icons/` from the committed source mark. It centers
the artwork on a dark charcoal (`#17181b`) background with 15% padding on each edge. The browser favicon uses
the padded PNG; in-app branding uses the original mark. Rebuild the debug bundle
and inspect the favicon and Finder/Dock icon before committing an asset change.

## Native verification

At the default 1440×900 size and minimum 1120×720 size, confirm that the three native window controls sit above the sidebar brand and the main area reaches the top edge without a separate title strip. Drag the empty sidebar/header areas; Refresh must still work as a button. Verify minimize/restore, fullscreen/return and close/reopen. Launch the executable again and confirm it focuses the same process and window.

The native controls keep their system spacing with roughly 16 points of space from the
top and left edges. After layout and focus changes, the macOS helper completes pending frame layout
and redraws the native content view so the configured inset survives fullscreen transitions
([upstream issue](https://github.com/tauri-apps/tauri/issues/15451)). Also maximize
the window before entering and leaving fullscreen; the inset must remain stable
without a manual resize. The configured `{ x: 16, y: 26 }` accounts for AppKit's
button frame offset; the visible frame should measure about 15 points from the
left and 16 points from the top.

This debug bundle is for local development. Signing, notarization, universal builds, oldest-supported macOS release QA and distribution require separate release acceptance.
