# Bundled reader sources

This directory holds the shared native readers XTrace Desktop runs for Codex
and Cursor history, copied from the pinned producer named by `.plugin-pin`:
the `plugins/memhub/scripts` tree of
[XTraceAI/agent-plugins](https://github.com/XTraceAI/agent-plugins) at the
pinned commit, with the repository's `LICENSE` (Apache-2.0) and `NOTICE`. The
tree is bundled into the app as the `agent-plugins` resource, so normal use
needs no plugin installation, developer checkout or Git.

Nothing here is edited by hand. At runtime, and in `cargo test`, every reader
source object the pin lists (the scripts tree, `readers/`, `readers_cli.py`,
`cursor_flush.py`) is verified by computing its Git object identity from the
files, so an edited, incomplete or foreign copy is refused and Codex/Cursor
scans report the mismatch. The pinned conformance gate also compares every file
with the pinned commit byte for byte.

To refresh after moving the pin:

```sh
AGENT_PLUGINS_DIR=/path/to/agent-plugins/plugins/memhub sh scripts/vendor-readers.sh
```

Then record the printed scripts tree object under `reader_sources` in
`.plugin-pin` (key `plugins/memhub/scripts`) with the other reader source IDs.
