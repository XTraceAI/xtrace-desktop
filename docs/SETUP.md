# Install and use XTrace Desktop

## Download and install

The beta download targets Apple Silicon (M-series) Macs with macOS 14 or newer.
Intel Macs, Windows and Linux do not have a supported download. A deployment
target of macOS 14 is not proof of testing on every supported Mac; check the
[release notes](https://github.com/XTraceAI/xtrace-desktop/releases/latest) and
[testing limits](UPDATES.md#beta-testing-limits) for the actual release evidence.

1. Open the [current update release](https://github.com/XTraceAI/xtrace-desktop/releases/latest),
   or visit [all releases](https://github.com/XTraceAI/xtrace-desktop/releases) for
   newer testing candidates. The latest-release link and automatic updates can
   remain on an earlier release while a newer beta is being tested. Choose the release's `XTrace-Desktop-<version>-macos-arm64.dmg` asset. The
   `.app.tar.gz`, `.sig` and `latest.json` assets serve the updater; GitHub's
   source-code archives do not contain an installed app.
2. Open the downloaded disk image. Drag **XTrace Desktop** onto **Applications**.
   If replacing an earlier copy, quit XTrace Desktop first and confirm replacement
   in Finder.
3. Eject the disk image and open **XTrace Desktop** from **Applications**.
   Accept macOS's ordinary downloaded-app confirmation if shown. If macOS rejects
   the app's signature or says it is damaged, stop and report the release version
   and exact message; do not remove quarantine or disable Gatekeeper.

You do not need Rust, Node or pnpm to use a downloaded app. There is no Homebrew cask.

## First launch

The welcome screen shows indexing progress and per-host results as the app reads
history already on this Mac. Choose **Open Dashboard** to explore. The first scan
can take time; **Settings → Native index** shows missing readers or incomplete
coverage. An empty host does not mean the host is installed or recording activity.

Claude Code history is read directly. Codex and Cursor indexing need **Python
3.10 or newer**; the reader scripts are bundled with XTrace, but Python is not.
If Settings reports a missing interpreter, install a compatible Python and
restart XTrace. Saved history must remain readable in the host's normal local
folders. The app does not fetch conversations that exist only in the cloud.

The Dashboard uses 7-, 14- or 30-day ranges; its recent-session activity covers the
last 48 hours. Sessions supports search and host filters. Opening a session shows
its indexed details and timeline; original-file transcript viewing currently
supports Claude only. Missing sources and unsupported transcript formats show an
unavailable state. API-equivalent cost estimates are not actual subscription charges.

Use **Settings** (⌘,) for appearance, typing speed, human-time break length,
storage and index status. The sidebar shows the app version. The menu-bar icon
opens today's activity; closing the main window can leave XTrace running.

## Optional local integrations

- **Account limits:** signed-in local Claude Code and Codex tools supply the Usage
  widget. Missing tools or unreadable limits show an unavailable or stale reading.
  These readings describe provider account limits, separate from indexed token
  counts and estimated cost.
- **GitHub:** an installed, signed-in `gh` CLI lets the app check linked pull
  requests. Checks run automatically on Dashboard/focus and roughly hourly,
  subject to retry and freshness limits. They read PR facts and do not post to
  GitHub. Without access, the app keeps its cached facts and reports the gap.
- **Rule activity:** the Rulebook reads retained activity from the default local
  rulebook source when available. It does not install, enable or change rules.

Native history indexing does not require the optional local capture server to
be running. Developer details are in [data sources](DATA-SOURCE.md) and the
[server guide](SERVER.md).

## Data and network

XTrace stores its database and usage files under
`~/Library/Application Support/ai.xtrace.app`. Settings shows the actual data
directory. The default **Metrics and indexing** mode keeps identifiers,
repository paths, timestamps, counts, usage and short text previews. It is not a
text-free or anonymous database. Previews are at most 280 characters for each
message classified as yours and each recognized Claude Code background-task
summary, in either storage mode, and cannot be switched off separately.

**Archive full transcript content** is off by default and affects future imports
and enrichment. Turning it off does not erase content already saved.
**Delete stored content** removes saved transcript content, tool input, saved
session titles and previews after confirmation. It preserves metric/index
records and original host files; later imports can create new previews.
Viewing a Claude transcript reads the original local source on demand without
enabling archival. Account-usage history is stored separately from the database.

There is no cloud transcript upload in this build. Network activity occurs for
anonymous usage telemetry (on by default after a first notice; see
[TELEMETRY.md](../TELEMETRY.md) for every event and how to turn it off),
account limits, GitHub PR facts and public release checks/downloads. Claude usage checks run the installed Claude Code CLI
at launch and about every ten minutes, with failure backoff, in an app-owned
probe folder. Claude Code can record that folder's trust and usage cache in its
own configuration and leave probe files in its history. These checks send no
model prompt. On an explicit **Refresh usage**, if the CLI read fails, XTrace
can use Claude Code's Keychain login to request usage from the provider;
automatic reads do not use that fallback. Codex usage reads use its local tool
and signed-in account. See [data-source details](DATA-SOURCE.md#routes-and-shell-controls)
for the precise behavior.

## Updates and troubleshooting

An updater-enabled release checks at launch and every six hours, downloads a
newer signed update, then offers **Restart to update** beside the version.
Use **Retry update** after a reported failure. You can also download the newest
DMG and replace the app in Applications after quitting it. Replacing the app
keeps its separate data directory. See [updates](UPDATES.md) for details.

For empty or incomplete results, check **Settings → Native index** first. For
missing PR facts, check the GitHub CLI's sign-in and repository access. For stale
account limits, use the Usage refresh control and inspect the displayed error.

Report bugs through [GitHub issues](https://github.com/XTraceAI/xtrace-desktop/issues)
with the app version, macOS version, steps and expected/actual result. Redact
conversation text, repository paths, account details and credentials from
screenshots and logs; do not attach your local database.
