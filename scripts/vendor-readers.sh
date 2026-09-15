#!/bin/sh
# Refresh the bundled reader sources from the pinned producer.
#
# The app runs Codex/Cursor readers from vendor/agent-plugins, a copy of the
# pinned plugin's scripts tree, its LICENSE and its NOTICE, exported from the
# exact commit named by .plugin-pin. Run this after moving the pin, with
# AGENT_PLUGINS_DIR naming a checkout of the producer (any commit: the pinned
# one is exported from its object store), then add the printed scripts tree
# object to .plugin-pin's reader_sources so the bundle verifies at runtime.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
COMMIT="$(node scripts/ci/plugin-pin.mjs commit)"
PLUGIN_ROOT="$(node scripts/ci/plugin-pin.mjs plugin_root)"
CHECKOUT="$(git -C "${AGENT_PLUGINS_DIR:?set AGENT_PLUGINS_DIR to a producer checkout}" rev-parse --show-toplevel)"
git -C "$CHECKOUT" cat-file -e "$COMMIT^{commit}" \
  || { echo "The checkout does not hold the pinned commit $COMMIT." >&2; exit 1; }
DESTINATION="$ROOT/vendor/agent-plugins"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/xtrace-vendor-readers.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
git -C "$CHECKOUT" archive "$COMMIT" "$PLUGIN_ROOT/scripts" LICENSE NOTICE | tar -xf - -C "$STAGING"
rm -rf "$DESTINATION/$PLUGIN_ROOT" "$DESTINATION/LICENSE" "$DESTINATION/NOTICE"
mkdir -p "$DESTINATION/$(dirname "$PLUGIN_ROOT")"
mv "$STAGING/$PLUGIN_ROOT" "$DESTINATION/$PLUGIN_ROOT"
mv "$STAGING/LICENSE" "$STAGING/NOTICE" "$DESTINATION/"
echo "vendored $PLUGIN_ROOT/scripts, LICENSE and NOTICE from $COMMIT"
echo "scripts tree object: $(git -C "$CHECKOUT" rev-parse "$COMMIT:$PLUGIN_ROOT/scripts")"
