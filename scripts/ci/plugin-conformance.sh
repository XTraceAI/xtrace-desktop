#!/bin/sh
# Required local native gate for the pinned capture producer.
#
# Owns the pinned checkout, environment and unskipped execution: it resolves
# .plugin-pin, obtains (or verifies) a checkout at exactly that commit, exports
# AGENT_PLUGINS_DIR/PYTHON, runs every workspace conformance test with output
# captured, and rejects skipped, ignored, zero or missing named tests through
# assert-no-skipped-conformance.sh. A skip or absent prerequisite never passes.
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
REPOSITORY="$(node scripts/ci/plugin-pin.mjs repository)"
COMMIT="$(node scripts/ci/plugin-pin.mjs commit)"
PLUGIN_ROOT="$(node scripts/ci/plugin-pin.mjs plugin_root)"
PYTHON="${PYTHON:-python3}"
# The harnesses check their contracts with assert statements, and inherited
# interpreter settings must not weaken them: PYTHONOPTIMIZE strips asserts,
# PYTHONPATH can shadow the standard library, PYTHONHOME replaces the runtime.
unset PYTHONOPTIMIZE PYTHONPATH PYTHONHOME PYTHONSTARTUP
"$PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' \
  || { echo "Conformance requires Python 3.10+; set PYTHON." >&2; exit 1; }
"$PYTHON" -c 'import sys; sys.exit(0 if sys.flags.optimize == 0 else 1)' \
  || { echo "Conformance requires Python assertions to stay active; PYTHON must not enable -O." >&2; exit 1; }
export PYTHON
if [ -n "${AGENT_PLUGINS_DIR:-}" ]; then
  CHECKOUT="$(git -C "$AGENT_PLUGINS_DIR" rev-parse --show-toplevel)"
else
  # No checkout supplied: fetch exactly the pinned commit from the public
  # producer into an ignored private artifact directory (no credentials).
  CHECKOUT="$ROOT/artifacts/private/plugin-conformance"
  if [ ! -d "$CHECKOUT/.git" ]; then
    mkdir -p "$CHECKOUT"
    git -C "$CHECKOUT" init -q
  fi
  if [ "$(git -C "$CHECKOUT" rev-parse --verify --quiet HEAD || true)" != "$COMMIT" ]; then
    GIT_TERMINAL_PROMPT=0 git -C "$CHECKOUT" -c credential.helper= fetch -q --depth 1 "$REPOSITORY" "$COMMIT" \
      || { echo "Pinned commit $COMMIT is not fetchable from $REPOSITORY." >&2; exit 1; }
    git -C "$CHECKOUT" checkout -q --detach FETCH_HEAD
  fi
  AGENT_PLUGINS_DIR="$CHECKOUT/$PLUGIN_ROOT"
fi
export AGENT_PLUGINS_DIR
test -d "$AGENT_PLUGINS_DIR" || { echo "Pinned plugin root is absent: $AGENT_PLUGINS_DIR" >&2; exit 1; }
# HEAD, every pinned reader source object and a clean tree must match the pin.
node -e '
import("./scripts/ci/plugin-pin.mjs").then(async ({ readPin, verifyCheckout }) => {
  const { execFileSync } = await import("node:child_process");
  const pin = await readPin(process.cwd());
  const git = (args) => execFileSync("git", ["-C", process.argv[1], ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  console.log("pinned producer " + verifyCheckout(pin, git) + " (memhub " + pin.plugin_version + ")");
}).catch((error) => { console.error(error.message); process.exit(1); });
' "$CHECKOUT"
mkdir -p artifacts/private/conformance
LOG="artifacts/private/conformance/cargo-test.log"
: > "$LOG"
# Both streams are captured: a SKIP printed to stderr must be as visible as one
# on stdout. cargo's own status survives the tee pipeline through a status file
# that is removed first and written by an errexit-safe construct, so a failed or
# aborted cargo run can never inherit an earlier run's 0.
STATUS="artifacts/private/conformance/cargo-test.status"
rm -f "$STATUS"
{ cargo test --workspace --all-features --locked -- conformance --nocapture && echo 0 > "$STATUS" || echo "$?" > "$STATUS"; } 2>&1 | tee "$LOG"
cargo_status="$(cat "$STATUS" 2>/dev/null || echo unknown)"
[ "$cargo_status" = 0 ] || echo "cargo test exited with status $cargo_status" >&2
sh scripts/ci/assert-no-skipped-conformance.sh "$LOG"
[ "$cargo_status" = 0 ]
echo "Pinned plugin conformance passed at $COMMIT."
