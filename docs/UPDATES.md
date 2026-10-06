# Desktop updates

The macOS release app checks at startup and every six hours. A newer update
downloads in the background with a ten-minute timeout; checks time out after
30 seconds. Tauri verifies its signature and signed release version before
the app offers **Restart to update** beside the version in the sidebar footer.
Update progress, status and **Retry update** also appear there; idle or disabled
updates show no control. The restart button installs the downloaded
update, then requests the normal Exit/restart path. Installation errors keep
the downloaded update for retry. Restart errors retry restart without installing
again. Navigation and React remounts do not duplicate checks or lose a ready update.

Updates replace the application bundle, not the application-data directory,
SQLite store, or original session files. The existing Exit handler closes
workers and the database after successful installation. No pre-install shutdown
or data migration is added here. Tauri handles bundle replacement and may ask
for administrator permission. A real signed macOS update and restart still need
release validation; compile and unit checks do not establish that behavior.

## Release configuration

The base configuration has no endpoint or trusted public key, so ordinary builds
leave updates disabled. Nonfixture native main windows with updates disabled offer
an information-only **Updates** control that opens public releases after a click,
including packaged debug and optimized builds. When native updates are enabled,
the sidebar uses the public updater instead. If the native mode cannot be read,
neither update control is exposed. The release overlay includes the real public
signing key and GitHub Releases endpoint. Debug builds, native fixture builds,
browser previews, and unsupported platforms never register/use the updater.
The main window can check, download and install separately; the tray cannot,
and the combined download-and-install permission is not granted.

Use the release configuration to build an updater release:

```sh
export TAURI_SIGNING_PRIVATE_KEY="/absolute/path/to/private/xtrace-updater.key"
# For an unprotected key:
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
# For a password-protected key, securely set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
# to its password in the local shell instead.
pnpm build:updater apps/desktop/src-tauri/tauri.updater.release.json
```

Configure the matching private-key file path only in the local release shell.
The release overlay requires `createUpdaterArtifacts: true` and signed-version
verification. The public key is copied as already encoded file content, not a
file path or a second base64 encoding. The example overlay remains an invalid
template with an empty public key and endpoints.
An updater-enabled build fails if configuration or `TAURI_SIGNING_PRIVATE_KEY`
is missing. No signing keys are created by the app or these instructions.

The repository `XTraceAI/xtrace-desktop` is now public, so its GitHub Releases
feed needs no authentication. The configured endpoint is
`https://github.com/XTraceAI/xtrace-desktop/releases/latest/download/latest.json`.
The public endpoint is live and v0.1.1 has been published. Preparing v0.1.2
in source does not publish that version or change the feed. `latest.json` must not offer an update before its real archive and
signature assets are available. Never embed credentials in the app or URL.

Keep the private signing key and password in private release storage, outside
the checkout and keep a secure backup. Key generation is complete; use the
existing matching key rather than generating a replacement. Supply the key path
and password to the build process as shown above without logging private key
contents or passwords. For manual signing, use the path flag shown below.
The public key is safe to include in the shipped overlay.
Keep the same key for future releases. Every release must increase its SemVer.

## Packaging and publishing (not automated here)

The existing `.github/workflows/release-native.yml` only validates a candidate.
It does not sign, notarize, package, or publish updates. Its launch checks are
separate from updater release acceptance. Do not point a feed at unsigned test
bundles or add a publishing step without release setup.

Use the pinned Tauri CLI **2.12.0** and updater **2.12.0**. Core Tauri stays at
2.11.5. The CLI records the app version in the signature; `requireSignedVersion`
must remain true. Signatures from the older CLI lacking that version are rejected.
The build accepts a file path in `TAURI_SIGNING_PRIVATE_KEY`, but CLI 2.12.0's
manual signer expects literal key contents in that variable. For manual signing,
remove that variable from the command's environment and pass its local file path
explicitly:

```sh
env -u TAURI_SIGNING_PRIVATE_KEY pnpm tauri signer sign --private-key-path "$TAURI_SIGNING_PRIVATE_KEY" --app-version "0.1.2" "/absolute/path/to/XTrace Desktop.app.tar.gz"
```

Replace `0.1.2` with the app's exact release version and the archive path with the
final archive to sign. The shell expands the key path before `env` removes the
variable for the signer. Keep `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the local
environment: empty for an unprotected key, or securely supplied for a protected
key.

The release process must fail before packaging when updater signing configuration
is absent. Apple signing/notarization credentials are additionally required.
Before publishing, perform this sequence:

1. Build the release with the real overlay and `createUpdaterArtifacts: true`,
   Apple code signing, and notarization enabled. Check the bundler's notarization
   and stapling result for the `.app`. Never publish an archive made before stapling.
2. Validate the final `.app` with `codesign --verify --deep --strict`,
   `xcrun stapler validate`, and `spctl --assess --type execute`.
3. If the app was notarized/stapled after automatic updater packaging, discard
   the earlier updater archive/signature and recreate the `.app.tar.gz` from the
   final app (one top-level `.app` directory), then sign that final archive with
   CLI 2.12.0 and `--app-version`. Never modify the app/archive after updater signing.
4. Publish immutable HTTPS archive and signature assets. Publish the feed only
   after those assets are available. Put the **contents** of the `.sig` in the
   manifest, not its path. A static manifest uses `version` plus
   `platforms.darwin-aarch64`/`platforms.darwin-x86_64`, each with `url` and
   `signature`. A server returns 204 for no update, or `version`, `url`, and
   `signature` for an update. Publish only architectures actually built/tested.
5. On disposable signed app copies with synthetic data, verify valid update,
   bad signature, mismatched signed version, timeout, interrupted download,
   install permission failure, explicit restart, and retained local data.

See the [official updater guide](https://v2.tauri.app/plugin/updater/) and the
[pinned updater source](https://github.com/tauri-apps/plugins-workspace/tree/updater-v2.12.0/plugins/updater).
This source change configures the public release endpoint and trusted public
key. It does not publish v0.1.2 or change the existing feed; release packaging and
publication remain manual.

## Beta 0.1.2 limitation

Some nested Guardian review conversations may appear under the root conversation rather than their immediate parent. This beta does not change those existing links.
