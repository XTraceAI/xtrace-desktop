# Environment probe and bridge

The Environment bridge reports how often structural tool identities were called (M-17) beside the components that supported host configuration files state. It is an honest alpha boundary: the registries a local reader can open cannot prove a complete, callable host inventory. The bridge therefore always supplies an **unknown** inventory to M-17 and returns verified configured components separately. It never derives "not installed", "never called" or a used/installed ratio.

## What the probe reads

`xt_probes::environment::probe(&ProbeRoots)` reads only the documented paths below, joined onto roots the caller supplied. There is no default or implicit home.

| Host   | Scope            | Path under the root                                 | Source id                   | States                  |
| ------ | ---------------- | --------------------------------------------------- | --------------------------- | ----------------------- |
| claude | home, repository | `.claude/skills/<name>/SKILL.md`                    | `claude_skill_directory`    | standalone skills       |
| claude | home             | `.claude.json` → `mcpServers`                       | `claude_user_mcp_config`    | MCP server names        |
| claude | repository       | `.mcp.json` → `mcpServers`                          | `claude_project_mcp_config` | MCP server names        |
| claude | home             | `.claude/plugins/installed_plugins.json` (schema 2) | `claude_plugin_registry`    | installed plugins       |
| claude | home             | `.claude/settings.json` → `enabledPlugins`          | `claude_enabled_plugins`    | stated enablement flags |
| claude | home             | `.claude/settings.json` → `hooks`                   | `claude_hook_config`        | configured hook entries |
| codex  | home             | `.codex/config.toml` → `[mcp_servers]`              | `codex_mcp_config`          | MCP server names        |
| cursor | home, repository | `.cursor/mcp.json` → `mcpServers`                   | `cursor_mcp_config`         | MCP server names        |

- A skill is an unlinked directory entry holding a regular, unlinked `SKILL.md`. Any other entry, including a linked or non-UTF-8 one, is counted and skipped.
- A plugin counts as installed only when one of its entries is an object whose `scope` is `user`, `project` or `local`, whose `installPath` is a nonempty string and whose `installedAt` is a string the workspace's locked `chrono` RFC 3339 parser accepts (an offset is required; impossible dates and times such as February 30 or hour 25 are rejected). Empty, non-string or other values prove nothing and are skipped. Other schema versions are `unsupported`.
- An enablement flag is kept only when it is a boolean; `null` means unstated, never `false`.
- A hook entry is named `<event>:<matcher>` (`*` when the matcher is absent or `null`). It counts only when the event name is usable, the group is an object, a stated matcher is a nonempty string, and `hooks` is a nonempty list whose every element is an object. Any other group, a `null` or scalar entry, or an event whose value is not a list is counted and skipped. `"hooks": null` at the top level is `empty`.
- An MCP entry states a server, not that server's tools, so no `mcp__server__tool` identity is produced. A server counts only when its value is an object (a TOML table for Codex); `null`, a number, a string or a list is counted and skipped. A `mcpServers` value that is not an object makes the source `malformed`.
- `.codex/plugins/cache` and `.cursor/plugins/cache` are counted as **cache observations** only. A cached download is not an installed or configured component.

Codex TOML is parsed with the workspace's locked `toml` crate. Only the shape of a server's value, a hook entry or a plugin entry is examined. Commands, arguments, environment tables, headers and URLs are never kept, and an install-path field is only checked for being nonempty, so those field values cannot be returned. Results carry structural names, counts and statuses only. Names retain their existing spelling rules even when a name looks like a path; it is still a name and is never opened as a path. The probe does not return discovered filesystem paths; a root is identified by its scope and its position in the supplied list.

## Bounds and statuses

The probe launches no process, opens no socket, reads no transcript and never walks a directory recursively.

- **Links.** On macOS and Linux, each directory from `/` through a supplied root and each documented source is opened one component at a time with `O_DIRECTORY | O_NOFOLLOW`. Metadata checks, child opens and bounded directory enumeration use those pinned handles. A root under a linked ancestor has root state `symlink`; a link under a root makes the source `unsupported` (`symlink`). A linked directory entry is counted and skipped. A replacement ancestor cannot redirect reads: the probe either uses the original pinned directory or refuses the replacement. Skill children are opened through the listing's handle and checked against their saved identity before `SKILL.md` is checked through the child handle. Files are opened with `O_NOFOLLOW | O_NONBLOCK` and checked as the same regular file before any byte is read, so replacement links, FIFOs and devices cannot redirect or block the read. The final name is checked again through its parent after reading or listing. Other platforms report roots as `unreadable` and open no supplied source.
- **Bytes.** A file is opened and at most 4 MiB + 1 bytes are consumed, whatever its size was when it was examined. More than 4 MiB is `unsupported` (`file_too_large`), including a file that grows after it was examined.
- **Entries.** A JSON or TOML map with more than 256 entries is `incomplete` (`entry_limit`) with the overflow counted; the first 256 names in sorted order are kept. A directory is enumerated only to 257 entries. Past that, the enumeration stops, the source is `incomplete` (`entry_limit`) with `stated` and `skipped` both 257 (a lower bound), and no component is taken from it: an unordered listing cannot choose a deterministic subset.
- **Names.** A name longer than 128 characters or containing a control character is skipped.
- A documented file path that is not a regular file is `unsupported` (`not_regular_file`); a documented directory, or an intermediate component, that is not a directory is `unsupported` (`not_directory`).

Each source under each root reports one status: `missing` (absent), `empty` (present, states nothing), `read`, `incomplete`, `malformed`, `unreadable` or `unsupported`. None of these is a measurement of zero installed components. Components are deduplicated by host, source and name; the first root to state one (home roots before repository roots, each in supplied order) sets its scope.

## Roots

- **Native:** the app's native home option (`XTRACE_NATIVE_HOME`, or the user's home) plus local repository paths stored session metadata states (`repo`, else `cwd`) on the first session page, most recent first. Remote identifiers (`https://…`, `git@…`, `ssh://…`), relative values and `..` are refused. No Git command runs and no parent directory is walked.
- **Fixture:** the same probe over F16's small synthetic matrix under `fixtures/F16/input/probes/env/`, with roots declared in F16's `env` snapshot. F16 remains a skeleton; its matrix is input, not a golden expectation, and it is not the planned 58-item inventory.

## Bridge

`environment(days)` invokes `metrics_environment` with `{ windowDays }` (7, 14 or 30) and returns the generated `EnvironmentMetrics`; see [App data and routing](DATA-SOURCE.md#environment).
